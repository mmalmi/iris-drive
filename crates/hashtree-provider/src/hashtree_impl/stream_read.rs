//! Stream a stored file to a local path without holding the whole file in memory.

use std::fs::{self, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use futures::StreamExt;
use hashtree_core::{HashTreeError, LinkType, Store};

use super::{HashTreeProviderFs, kind_from_link, map_err, modified_at_from_meta};
use crate::error::ProviderError;
use crate::provider::Item;

static STREAM_READ_TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

// The read future can be dropped while waiting for a remote chunk. Keep the
// cleanup owner outside the streaming future so its writer is closed first.
struct PendingOutput(Option<PathBuf>);

impl Drop for PendingOutput {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_file(path);
        }
    }
}

fn output_error(path: &Path, error: &io::Error) -> ProviderError {
    ProviderError::Backend(format!("writing {}: {error}", path.display()))
}

impl<S: Store> HashTreeProviderFs<S> {
    /// Stream the file at `id` directly to `output` chunk by chunk, enforcing
    /// the declared directory size and replacing `output` atomically on
    /// success. Never retains the whole file in memory.
    pub async fn read_file_to_path(
        &self,
        id: &str,
        output: &Path,
    ) -> Result<Item<String>, ProviderError> {
        let resolved = self.resolve(id).await?;
        if resolved.link_type == LinkType::Dir {
            return Err(ProviderError::IsDir);
        }
        let parent = output.parent().unwrap_or_else(|| Path::new("."));
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| output_error(parent, &error))?;
        }

        let counter = STREAM_READ_TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp_path = parent.join(format!(
            ".iris-drive-read.{}.{}.tmp",
            std::process::id(),
            counter
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(&tmp_path)
            .map_err(|error| output_error(&tmp_path, &error))?;
        let mut pending_output = PendingOutput(Some(tmp_path.clone()));

        let stream_result = async {
            let mut writer = BufWriter::new(file);
            let mut stream = self.tree.get_stream(&resolved.cid);
            let mut saw_chunk = false;
            let mut written = 0u64;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| match error {
                    HashTreeError::MissingChunk(_) => ProviderError::NotFound,
                    other => map_err(other),
                })?;
                saw_chunk = true;
                written = written.saturating_add(chunk.len() as u64);
                if written > resolved.size {
                    return Err(ProviderError::Backend(
                        "file size does not match directory entry".into(),
                    ));
                }
                writer
                    .write_all(&chunk)
                    .map_err(|error| output_error(&tmp_path, &error))?;
            }
            if !saw_chunk {
                return Err(ProviderError::NotFound);
            }
            if written != resolved.size {
                return Err(ProviderError::Backend(
                    "file size does not match directory entry".into(),
                ));
            }
            writer
                .flush()
                .map_err(|error| output_error(&tmp_path, &error))?;
            drop(writer);
            fs::rename(&tmp_path, output).map_err(|error| output_error(output, &error))?;
            pending_output.0 = None;
            Ok(())
        }
        .await;

        stream_result?;

        let name = id.rsplit('/').next().unwrap_or("").to_string();
        Ok(Item {
            id: id.to_owned(),
            name,
            kind: kind_from_link(resolved.link_type),
            size: resolved.size,
            version: resolved.cid.to_string(),
            modified_at: modified_at_from_meta(resolved.meta.as_ref()),
        })
    }
}
