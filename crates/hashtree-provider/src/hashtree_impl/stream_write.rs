//! Replace a file's contents from a local file without holding it in memory.

use std::fs::File;
use std::io::{self, Read};
use std::path::Path;

use futures::io::AllowStdIo;
use hashtree_core::{LinkType, Store, to_hex};
use sha2::{Digest, Sha256};

use super::{
    EntryWrite, HashTreeProviderFs, WHOLE_FILE_HASH_META_KEY, link_type_for_size, map_err,
    modified_at_meta, set_entry_with_meta,
};
use crate::error::ProviderError;

/// Hands out a file's bytes and hashes each one on the way.
struct HashingReader {
    file: File,
    hasher: Sha256,
    read: u64,
}

impl Read for HashingReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let count = self.file.read(buf)?;
        self.hasher.update(&buf[..count]);
        self.read += count as u64;
        Ok(count)
    }
}

fn source_error(source: &Path, error: &io::Error) -> ProviderError {
    ProviderError::Backend(format!("reading {}: {error}", source.display()))
}

impl<S: Store> HashTreeProviderFs<S> {
    /// Replace the existing file at `id` with the bytes of the local file
    /// `source`. The bytes are stored chunk by chunk as they are read, so a
    /// large video never sits in memory whole. Returns the stored size.
    pub async fn write_file_from_path(
        &self,
        id: &str,
        source: &Path,
    ) -> Result<u64, ProviderError> {
        let _guard = self.modify_lock.lock().await;
        let resolved = self.resolve(id).await?;
        if resolved.link_type == LinkType::Dir {
            return Err(ProviderError::IsDir);
        }
        let file = File::open(source).map_err(|error| source_error(source, &error))?;
        let expected = file
            .metadata()
            .map_err(|error| source_error(source, &error))?
            .len();
        let mut reader = HashingReader {
            file,
            hasher: Sha256::new(),
            read: 0,
        };
        let (cid, size) = self
            .tree
            .put_stream(AllowStdIo::new(&mut reader))
            .await
            .map_err(map_err)?;
        if size != expected || reader.read != expected {
            return Err(ProviderError::Backend(format!(
                "{} changed while it was being saved",
                source.display()
            )));
        }
        let digest: [u8; 32] = reader.hasher.finalize().into();
        let mut meta = modified_at_meta(resolved.meta);
        meta.insert(
            WHOLE_FILE_HASH_META_KEY.to_string(),
            serde_json::Value::String(to_hex(&digest)),
        );
        let (parent_segs, name) = Self::split_path(id)?;
        let new_root = set_entry_with_meta(
            &self.tree,
            &self.current_root().await,
            &parent_segs,
            EntryWrite {
                name,
                cid: &cid,
                size,
                link_type: link_type_for_size(size),
                meta: Some(meta),
            },
        )
        .await
        .map_err(map_err)?;
        self.apply_new_root(new_root).await?;
        Ok(size)
    }
}
