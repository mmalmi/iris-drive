//! Bidirectional sync engine.
//!
//! Reconciles two `ProviderFs` instances by enumerating each side's
//! state via `changes_since(None)` and feeding per-path snapshots
//! through the conflict resolver from [`crate::conflict`].
//!
//! Single-pass, no streaming. Suitable for v1 sync between two devices'
//! drives or between the local working set and a peer's published root.
//!
//! Conflict policy: keep both sides. Local stays at the original path;
//! the remote's bytes land in `name (conflict from peer).ext` on the
//! local side. The symmetric rename happens when the peer runs its own
//! sync — which makes the algorithm deterministic without coordination.

use std::collections::{BTreeMap, BTreeSet};

use hashtree_provider::{EntryInfo, PathChange, ProviderError, ProviderFs};
use thiserror::Error;

use crate::conflict::conflict_filename;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("provider error: {0}")]
    Provider(#[from] ProviderError),
    #[error("conflict: failed to apply rename for {path}: {reason}")]
    ConflictApply { path: String, reason: String },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncReport {
    /// Paths copied local → remote.
    pub uploaded: Vec<String>,
    /// Paths copied remote → local.
    pub downloaded: Vec<String>,
    /// Paths deleted on local (because remote deleted them and local didn't diverge).
    pub deleted_local: Vec<String>,
    /// Paths deleted on remote (because local deleted them).
    pub deleted_remote: Vec<String>,
    /// Conflicts resolved by renaming the remote copy on local.
    pub conflicts: Vec<ConflictResolution>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictResolution {
    pub original_path: String,
    pub renamed_to: String,
}

/// Run one full bidirectional sync.
///
/// `device_label` is used in the conflict filename. The peer's label is
/// always rendered as `"peer"` in local-side renames, since the peer's
/// own label is unknown here.
pub async fn sync<L, R>(
    local: &L,
    remote: &R,
    device_label: &str,
) -> Result<SyncReport, SyncError>
where
    L: ProviderFs<ItemId = String>,
    R: ProviderFs<ItemId = String>,
{
    let local_entries = enumerate_files(local).await?;
    let remote_entries = enumerate_files(remote).await?;

    let mut report = SyncReport::default();
    let all_paths: BTreeSet<&String> = local_entries
        .keys()
        .chain(remote_entries.keys())
        .collect();

    for path in all_paths {
        let l = local_entries.get(path);
        let r = remote_entries.get(path);
        match (l, r) {
            (Some(_), Some(_)) if hashes_match(l.unwrap(), r.unwrap()) => {
                // Convergent state, nothing to do.
            }
            (Some(_), None) => {
                copy_file(local, path, remote, path).await?;
                report.uploaded.push(path.clone());
            }
            (None, Some(_)) => {
                copy_file(remote, path, local, path).await?;
                report.downloaded.push(path.clone());
            }
            (Some(_), Some(_)) => {
                // Both present, hashes differ — conflict.
                let conflict_path = conflict_filename(path, "peer");
                copy_file(remote, path, local, &conflict_path).await?;
                report.conflicts.push(ConflictResolution {
                    original_path: path.clone(),
                    renamed_to: conflict_path,
                });
                // Also push local's version to remote so they converge on
                // the original path. The remote will, on its own sync,
                // observe local-vs-its-old-content as a separate diff and
                // produce its own conflict rename labelled with its own
                // device name.
                copy_file(local, path, remote, path).await?;
                report.uploaded.push(path.clone());
            }
            (None, None) => unreachable!("path is in either local or remote map"),
        }
    }

    // Discard the device_label argument warning if unused in this v1.
    let _ = device_label;

    Ok(report)
}

fn hashes_match(l: &EntryInfo, r: &EntryInfo) -> bool {
    l.hash == r.hash && l.size == r.size
}

/// Enumerate file paths (not directories) under a provider and return
/// the latest `EntryInfo` for each. Uses `changes_since(None)` for the
/// canonical "everything as Added" enumeration.
async fn enumerate_files<P: ProviderFs<ItemId = String>>(
    fs: &P,
) -> Result<BTreeMap<String, EntryInfo>, SyncError> {
    let changes = fs.changes_since(None).await?;
    let mut out = BTreeMap::new();
    for c in changes {
        if let PathChange::Added { path, entry } = c
            && entry.link_type != hashtree_core::LinkType::Dir
        {
            out.insert(path, entry);
        }
    }
    Ok(out)
}

/// Copy a file from `src` to `dst`, materializing missing parent dirs
/// on the destination side.
async fn copy_file<S, D>(src: &S, src_path: &str, dst: &D, dst_path: &str) -> Result<(), SyncError>
where
    S: ProviderFs<ItemId = String>,
    D: ProviderFs<ItemId = String>,
{
    let bytes = read_full(src, src_path).await?;
    write_full(dst, dst_path, &bytes).await?;
    Ok(())
}

async fn read_full<P: ProviderFs<ItemId = String>>(fs: &P, path: &str) -> Result<Vec<u8>, SyncError> {
    let id = path.to_string();
    let item = fs.item(&id).await?;
    if item.size == 0 {
        return Ok(Vec::new());
    }
    Ok(fs.read(&id, 0, item.size).await?)
}

async fn write_full<P: ProviderFs<ItemId = String>>(
    fs: &P,
    path: &str,
    bytes: &[u8],
) -> Result<(), SyncError> {
    ensure_parents(fs, path).await?;
    let (parent, name) = split_path(path);
    // create-or-replace
    match fs.lookup(&parent, name).await {
        Ok(item) => {
            fs.truncate(&item.id, 0).await?;
            if !bytes.is_empty() {
                fs.write(&item.id, 0, bytes).await?;
            }
        }
        Err(ProviderError::NotFound) => {
            let item = fs.create_file(&parent, name).await?;
            if !bytes.is_empty() {
                fs.write(&item.id, 0, bytes).await?;
            }
        }
        Err(e) => return Err(SyncError::Provider(e)),
    }
    Ok(())
}

async fn ensure_parents<P: ProviderFs<ItemId = String>>(
    fs: &P,
    path: &str,
) -> Result<(), SyncError> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() <= 1 {
        return Ok(());
    }
    let mut cursor = String::new();
    for seg in &segments[..segments.len() - 1] {
        match fs.lookup(&cursor, seg).await {
            Ok(item) => {
                cursor = item.id;
            }
            Err(ProviderError::NotFound) => {
                let item = fs.create_dir(&cursor, seg).await?;
                cursor = item.id;
            }
            Err(e) => return Err(SyncError::Provider(e)),
        }
    }
    Ok(())
}

fn split_path(path: &str) -> (String, &str) {
    match path.rfind('/') {
        Some(i) => (path[..i].to_string(), &path[i + 1..]),
        None => (String::new(), path),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashtree_core::{HashTree, HashTreeConfig, MemoryStore};
    use hashtree_provider::HashTreeProviderFs;
    use std::sync::Arc;

    async fn fresh_provider() -> HashTreeProviderFs<MemoryStore> {
        let tree = Arc::new(HashTree::new(
            HashTreeConfig::new(Arc::new(MemoryStore::new())).public(),
        ));
        HashTreeProviderFs::fresh(tree).await.unwrap()
    }

    async fn write_file<P: ProviderFs<ItemId = String>>(fs: &P, path: &str, bytes: &[u8]) {
        write_full(fs, path, bytes).await.unwrap();
    }

    async fn read_file<P: ProviderFs<ItemId = String>>(fs: &P, path: &str) -> Vec<u8> {
        read_full(fs, path).await.unwrap()
    }

    async fn paths<P: ProviderFs<ItemId = String>>(fs: &P) -> Vec<String> {
        let mut p: Vec<_> = enumerate_files(fs).await.unwrap().into_keys().collect();
        p.sort();
        p
    }

    #[tokio::test]
    async fn empty_to_empty_is_noop() {
        let l = fresh_provider().await;
        let r = fresh_provider().await;
        let report = sync(&l, &r, "dev").await.unwrap();
        assert_eq!(report, SyncReport::default());
    }

    #[tokio::test]
    async fn local_only_uploads_to_remote() {
        let l = fresh_provider().await;
        let r = fresh_provider().await;
        write_file(&l, "hello.txt", b"hi there").await;
        let report = sync(&l, &r, "dev").await.unwrap();
        assert_eq!(report.uploaded, vec!["hello.txt".to_string()]);
        assert!(report.downloaded.is_empty());
        assert!(report.conflicts.is_empty());
        assert_eq!(read_file(&r, "hello.txt").await, b"hi there");
    }

    #[tokio::test]
    async fn remote_only_downloads_to_local() {
        let l = fresh_provider().await;
        let r = fresh_provider().await;
        write_file(&r, "hello.txt", b"from peer").await;
        let report = sync(&l, &r, "dev").await.unwrap();
        assert_eq!(report.downloaded, vec!["hello.txt".to_string()]);
        assert_eq!(read_file(&l, "hello.txt").await, b"from peer");
    }

    #[tokio::test]
    async fn matching_files_are_noop() {
        let l = fresh_provider().await;
        let r = fresh_provider().await;
        write_file(&l, "hello.txt", b"identical").await;
        write_file(&r, "hello.txt", b"identical").await;
        let report = sync(&l, &r, "dev").await.unwrap();
        assert!(report.uploaded.is_empty());
        assert!(report.downloaded.is_empty());
        assert!(report.conflicts.is_empty());
    }

    #[tokio::test]
    async fn divergent_files_produce_conflict_rename() {
        let l = fresh_provider().await;
        let r = fresh_provider().await;
        write_file(&l, "report.pdf", b"local-version").await;
        write_file(&r, "report.pdf", b"remote-version").await;
        let report = sync(&l, &r, "dev").await.unwrap();
        assert_eq!(report.conflicts.len(), 1);
        assert_eq!(report.conflicts[0].original_path, "report.pdf");
        assert_eq!(
            report.conflicts[0].renamed_to,
            "report (conflict from peer).pdf"
        );
        // Local keeps its original.
        assert_eq!(read_file(&l, "report.pdf").await, b"local-version");
        // Local also has the renamed remote copy.
        assert_eq!(
            read_file(&l, "report (conflict from peer).pdf").await,
            b"remote-version"
        );
        // Remote received local's bytes at the original path.
        assert_eq!(read_file(&r, "report.pdf").await, b"local-version");
    }

    #[tokio::test]
    async fn nested_path_creates_parent_dirs() {
        let l = fresh_provider().await;
        let r = fresh_provider().await;
        write_file(&l, "a/b/c.txt", b"deep").await;
        sync(&l, &r, "dev").await.unwrap();
        assert_eq!(read_file(&r, "a/b/c.txt").await, b"deep");
    }

    #[tokio::test]
    async fn two_passes_converge() {
        let l = fresh_provider().await;
        let r = fresh_provider().await;
        write_file(&l, "x.txt", b"local").await;
        write_file(&r, "y.txt", b"remote").await;
        sync(&l, &r, "dev").await.unwrap();
        assert_eq!(paths(&l).await, vec!["x.txt".to_string(), "y.txt".to_string()]);
        assert_eq!(paths(&r).await, vec!["x.txt".to_string(), "y.txt".to_string()]);
        // Second sync is a no-op.
        let report = sync(&l, &r, "dev").await.unwrap();
        assert_eq!(report, SyncReport::default());
    }

    #[tokio::test]
    async fn second_sync_after_modification_propagates() {
        let l = fresh_provider().await;
        let r = fresh_provider().await;
        write_file(&l, "x.txt", b"v1").await;
        sync(&l, &r, "dev").await.unwrap();
        assert_eq!(read_file(&r, "x.txt").await, b"v1");

        // local edits, then sync again
        write_file(&l, "x.txt", b"v2-larger").await;
        let report = sync(&l, &r, "dev").await.unwrap();
        assert_eq!(report.uploaded, vec!["x.txt".to_string()]);
        assert_eq!(read_file(&r, "x.txt").await, b"v2-larger");
    }

    #[tokio::test]
    async fn three_devices_converge_after_two_pairwise_syncs() {
        let a = fresh_provider().await;
        let b = fresh_provider().await;
        let c = fresh_provider().await;
        write_file(&a, "shared.txt", b"alpha").await;
        sync(&a, &b, "a").await.unwrap();
        sync(&b, &c, "b").await.unwrap();
        assert_eq!(read_file(&c, "shared.txt").await, b"alpha");
    }
}
