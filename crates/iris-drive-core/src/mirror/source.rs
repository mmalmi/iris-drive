//! Walk a selected source without following links. Verify new bytes by reading them back.
use super::{Checkpoint, Report};
use anyhow::{Context, Result, ensure};
use hashtree_core::{Cid, HashTree, Store};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::compat::TokioAsyncReadCompatExt;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct Stamp {
    len: u64,
    modified: SystemTime,
    #[cfg(unix)]
    identity: (u64, u64, i64, i64),
}

impl Stamp {
    fn read(meta: &fs::Metadata) -> Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        Ok(Self {
            len: meta.len(),
            modified: meta.modified()?,
            #[cfg(unix)]
            identity: (meta.dev(), meta.ino(), meta.ctime(), meta.ctime_nsec()),
        })
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub(super) struct Verified {
    stamp: Stamp,
    pub cid: String,
    pub digest: [u8; 32],
}

impl Verified {
    pub fn size(&self) -> u64 {
        self.stamp.len
    }
    pub fn modified(&self) -> Option<i64> {
        self.stamp
            .modified
            .duration_since(SystemTime::UNIX_EPOCH)
            .ok()?
            .as_secs()
            .try_into()
            .ok()
    }
}

pub(super) fn validate_location(config: &Path, source: &Path) -> Result<()> {
    let config = config.canonicalize()?;
    ensure!(
        !config.starts_with(source) && !source.starts_with(&config),
        "The source overlaps Iris Drive's own storage"
    );
    let mount = crate::paths::default_mountpoint_in(&config);
    ensure!(
        !source.starts_with(&mount) && !mount.starts_with(source),
        "Choose a source outside the Iris Drive mount"
    );
    #[cfg(target_os = "linux")]
    if let Some(status) = super::read::<serde_json::Value>(
        &config.join(crate::daemon_liveness::DAEMON_STATUS_FILE_NAME),
    )? && let Some(path) = status
        .pointer("/mount/mountpoint")
        .and_then(serde_json::Value::as_str)
    {
        let mount = Path::new(path);
        ensure!(
            !source.starts_with(mount) && !mount.starts_with(source),
            "Choose a source outside the Iris Drive mount"
        );
    }
    #[cfg(target_os = "macos")]
    ensure!(
        !source.ends_with("Library/CloudStorage"),
        "Choose a folder inside your cloud provider"
    );
    #[cfg(target_os = "macos")]
    for ancestor in source.ancestors() {
        if ancestor
            .parent()
            .and_then(Path::file_name)
            .is_some_and(|name| name == "CloudStorage")
        {
            let domain = ancestor
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .replace(' ', "")
                .to_lowercase();
            ensure!(
                !domain.starts_with("irisdrive"),
                "Choose a source outside Iris Drive"
            );
        }
    }
    Ok(())
}

fn placeholder(name: &str, meta: &fs::Metadata) -> bool {
    if name.starts_with('.') && name.ends_with(".icloud") {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::darwin::fs::MetadataExt;
        meta.st_flags() & 0x4000_0000 != 0
    } // SF_DATALESS
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & (0x1000 | 0x40000 | 0x400000) != 0
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let _ = meta;
        false
    }
}

#[cfg(unix)]
fn open_child(parent: &fs::File, name: &std::ffi::OsStr, _: &Path) -> Result<fs::File> {
    use rustix::fs::{Mode, OFlags, openat};
    Ok(openat(
        parent,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
        Mode::empty(),
    )?
    .into())
}

#[cfg(windows)]
fn open_child(_: &fs::File, _: &std::ffi::OsStr, path: &Path) -> Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    Ok(fs::OpenOptions::new()
        .read(true)
        .custom_flags(0x0200_0000 | 0x0020_0000)
        .open(path)?)
}

fn names(dir: &fs::File, path: &Path) -> Result<Vec<std::io::Result<std::ffi::OsString>>> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let _ = path;
        Ok(rustix::fs::Dir::read_from(dir)?
            .filter_map(|entry| match entry {
                Ok(e) if matches!(e.file_name().to_bytes(), b"." | b"..") => None,
                Ok(e) => Some(Ok(
                    std::ffi::OsStr::from_bytes(e.file_name().to_bytes()).to_owned()
                )),
                Err(e) => Some(Err(e.into())),
            })
            .collect())
    }
    #[cfg(windows)]
    {
        let _ = dir;
        Ok(fs::read_dir(path)?
            .map(|e| e.map(|item| item.file_name()))
            .collect())
    }
}

fn is_link(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileTypeExt;
        let kind = meta.file_type();
        kind.is_symlink_dir() || kind.is_symlink_file() || kind.is_symlink()
    }
    #[cfg(unix)]
    {
        meta.is_symlink()
    }
}

#[derive(Default)]
pub(super) struct Scan {
    pub files: std::collections::BTreeMap<String, Verified>,
    pub dirs: Vec<String>,
}

pub(super) async fn scan<S: Store>(
    tree: &HashTree<S>,
    source: &Path,
    previous: &Checkpoint,
    report: &mut Report,
) -> Result<Scan> {
    ensure!(
        source.canonicalize()? == source,
        "The selected folder moved or became a link"
    );
    #[cfg(target_os = "macos")]
    let root = {
        use std::os::unix::fs::OpenOptionsExt;
        const O_NOFOLLOW_ANY: i32 = 0x2000_0000;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(O_NOFOLLOW_ANY)
            .open(source)?
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let root = {
        use rustix::fs::{Mode, OFlags, openat};
        let mut dir = fs::File::open("/")?;
        for part in source.components() {
            if let std::path::Component::Normal(name) = part {
                dir = openat(
                    &dir,
                    name,
                    OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::RDONLY | OFlags::CLOEXEC,
                    Mode::empty(),
                )?
                .into();
            }
        }
        dir
    };
    #[cfg(windows)]
    let root = {
        use std::os::windows::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(0x0200_0000 | 0x0020_0000)
            .open(source)?
    };
    let metadata = root.metadata()?;
    ensure!(
        metadata.is_dir() && !is_link(&metadata),
        "Source is no longer a folder"
    );
    ensure!(
        !placeholder("", &metadata),
        "Waiting for download. Choose Keep Downloaded in Finder"
    );
    let mut result = Scan::default();
    // Depth-first walk keeps only ancestor directory handles open.
    let mut stack = vec![(root, PathBuf::from(source), String::new(), None)];
    while let Some((dir, path, relative, entries)) = stack.pop() {
        let mut entries = match entries {
            Some(entries) => entries,
            None => match names(&dir, &path) {
                Ok(items) => items.into_iter(),
                Err(error) => {
                    report.issue(&relative, error);
                    continue;
                }
            },
        };
        while let Some(entry) = entries.next() {
            let name = match entry {
                Ok(name) => name,
                Err(e) => {
                    report.issue(&relative, e);
                    continue;
                }
            };
            let Some(name) = name.to_str() else {
                report.issue(&relative, "Filename is not UTF-8");
                continue;
            };
            let child_path = path.join(name);
            let child = if relative.is_empty() {
                name.to_owned()
            } else {
                format!("{relative}/{name}")
            };
            if crate::indexer::should_ignore_name(name) {
                if name != ".DS_Store"
                    && !name.starts_with("._")
                    && !name.eq_ignore_ascii_case("Thumbs.db")
                {
                    report.issue(&child, "Skipped by Iris Drive's filename rules");
                }
                continue;
            }
            let item = (|| -> Result<fs::File> {
                crate::provider::normalize_provider_document_path(&child)?;
                let meta = fs::symlink_metadata(&child_path)?;
                ensure!(
                    !placeholder(name, &meta),
                    "Waiting for download. Choose Keep Downloaded in Finder"
                );
                ensure!(!is_link(&meta), "Symbolic link skipped");
                ensure!(meta.is_dir() || meta.is_file(), "Unsupported file type");
                let f = open_child(&dir, name.as_ref(), &child_path)?;
                let actual = f.metadata()?;
                ensure!(
                    !is_link(&actual) && !placeholder(name, &actual),
                    "Source changed during scan"
                );
                Ok(f)
            })();
            let f = match item {
                Ok(f) => f,
                Err(e) => {
                    report.issue(&child, e);
                    continue;
                }
            };
            let meta = f.metadata()?;
            if meta.is_dir() {
                result.dirs.push(child.clone());
                stack.push((dir, path, relative, Some(entries)));
                stack.push((f, child_path, child, None));
                break;
            }
            match verify_file(tree, f, previous.files.get(&child)).await {
                Ok(file) => {
                    result.files.insert(child, file);
                }
                Err(error) => report.issue(&child, error),
            }
        }
    }
    Ok(result)
}

pub(super) async fn verify_file<S: Store>(
    tree: &HashTree<S>,
    file: fs::File,
    old: Option<&Verified>,
) -> Result<Verified> {
    let meta = file.metadata()?;
    ensure!(meta.is_file(), "Expected a regular file");
    let stamp = Stamp::read(&meta)?;
    if let Some(old) = old.filter(|old| old.stamp == stamp)
        && tree.get_store().has(&Cid::parse(&old.cid)?.hash).await?
    {
        return Ok(old.clone());
    }
    let mut input = tokio::fs::File::from_std(file);
    let (cid, size) = tree.put_stream((&mut input).compat()).await?;
    ensure!(size == stamp.len, "File changed while copying");
    let digest = compare_contents(tree, &cid, &mut input, size).await?;
    ensure!(
        stamp == Stamp::read(&input.metadata().await?)?,
        "File changed during verification"
    );
    Ok(Verified {
        stamp,
        cid: cid.to_string(),
        digest,
    })
}

async fn compare_contents<S: Store>(
    tree: &HashTree<S>,
    cid: &Cid,
    input: &mut tokio::fs::File,
    size: u64,
) -> Result<[u8; 32]> {
    input.rewind().await?;
    let mut buffer = vec![0u8; 1024 * 1024];
    let mut offset = 0;
    let mut digest = Sha256::new();
    loop {
        let end = size.min(offset + buffer.len() as u64);
        let stored = tree
            .read_file_range_cid(cid, offset, Some(end))
            .await?
            .context("Stored file unavailable")?;
        let n = usize::try_from(end - offset)?;
        input.read_exact(&mut buffer[..n]).await?;
        ensure!(stored == buffer[..n], "Stored copy differs from source");
        digest.update(&stored);
        if end == size {
            break;
        }
        offset = end;
    }
    Ok(digest.finalize().into())
}
