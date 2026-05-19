//! Walk a local directory and build the equivalent hashtree directory.
//!
//! Used in two situations:
//! - First-time import: a user points iris-drive at an existing folder.
//! - Sync engine: compute the local CID before publishing a new root.
//!
//! The indexer is deterministic — the same on-disk tree always produces
//! the same root CID — and tests exercise that property directly.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use hashtree_core::{Cid, DirEntry, HashTree, HashTreeError, LinkType, Store};
use thiserror::Error;

use crate::merge::{PREV_LINK_PATH, TOMBSTONE_PREFIX, walk_device_tree};

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("tree: {0}")]
    Tree(#[from] HashTreeError),
    #[error("non-utf8 file name at {0}")]
    NonUtf8(String),
    #[error("path is not a directory: {0}")]
    NotADirectory(String),
}

/// Index a directory recursively, returning the root htree CID.
///
/// Symlinks are not followed; they are silently skipped for v1 — Drive
/// and Dropbox both ignore symlinks too. Files unreadable due to
/// permissions surface as `IndexError::Io`.
pub async fn index_dir<S: Store>(tree: &HashTree<S>, dir: &Path) -> Result<Cid, IndexError> {
    if !dir.is_dir() {
        return Err(IndexError::NotADirectory(dir.display().to_string()));
    }
    index_dir_inner(tree, dir).await
}

/// Like [`index_dir`], but also diffs against a previous root to emit
/// tombstones for files that have been removed since the last import.
/// Tombstones that already exist in the previous root carry forward
/// (preserving their original removal time) so long as the file remains
/// absent. Tombstones whose original path is now present on disk are
/// silently dropped — the file "came back."
///
/// First-time imports (`previous_root = None`) behave exactly like
/// `index_dir`; the tombstone subtree is only added when there's a
/// previous root to diff against.
pub async fn index_dir_with_history<S: Store>(
    tree: &HashTree<S>,
    dir: &Path,
    previous_root: Option<&Cid>,
    now_unix_seconds: i64,
) -> Result<Cid, IndexError> {
    let mut root = index_dir(tree, dir).await?;
    let Some(prev) = previous_root else {
        return Ok(root);
    };

    let mut current_paths: BTreeSet<String> = BTreeSet::new();
    collect_local_file_paths(dir, "", &mut current_paths)?;

    let (prev_files, prev_tombstones) = walk_device_tree(tree, prev)
        .await
        .map_err(IndexError::Tree)?;

    let mut tombstones: BTreeMap<String, i64> = BTreeMap::new();
    // Files that were in the previous root but are no longer on disk
    // get a fresh tombstone stamped at the import time.
    for f in prev_files {
        if !current_paths.contains(&f.path) {
            tombstones.insert(f.path, now_unix_seconds);
        }
    }
    // Tombstones from the previous root carry forward when the file is
    // still absent (preserves original removal time). When the file is
    // present again, the tombstone silently drops.
    for t in prev_tombstones {
        if !current_paths.contains(&t.path) {
            tombstones.entry(t.path).or_insert(t.tombstoned_at);
        }
    }

    if !tombstones.is_empty() {
        root = layer_tombstones(tree, root, &tombstones).await?;
    }

    // Add the revision back-link: a `._prev` entry at the root pointing
    // at the prior root's Cid (hash + key). Capability propagates
    // automatically when readers decrypt the new TreeNode — the prior
    // TreeNode is now navigable from the current one.
    root = attach_prev_link(tree, root, prev).await?;

    Ok(root)
}

async fn attach_prev_link<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    previous: &Cid,
) -> Result<Cid, IndexError> {
    let segments: Vec<&str> = PREV_LINK_PATH
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let (name, parent_segs) = segments.split_last().expect("PREV_LINK_PATH is non-empty");
    // Ensure each ancestor (just `.hashtree/` for now) exists.
    for depth in 1..=parent_segs.len() {
        let dir_path: Vec<String> = parent_segs[..depth]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        root = ensure_dir(tree, &root, &dir_path).await?;
    }
    let new_root = tree
        .set_entry(&root, parent_segs, name, previous, 0, LinkType::Dir)
        .await?;
    Ok(new_root)
}

fn collect_local_file_paths(
    dir: &Path,
    prefix: &str,
    out: &mut BTreeSet<String>,
) -> Result<(), IndexError> {
    let mut entries: Vec<(String, std::path::PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|s| IndexError::NonUtf8(s.to_string_lossy().into_owned()))?;
        entries.push((name, entry.path()));
    }
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, path) in entries {
        let metadata = std::fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let logical_path = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if metadata.is_dir() {
            collect_local_file_paths(&path, &logical_path, out)?;
        } else if metadata.is_file() {
            out.insert(logical_path);
        }
    }
    Ok(())
}

async fn layer_tombstones<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    tombstones: &BTreeMap<String, i64>,
) -> Result<Cid, IndexError> {
    // Gather every unique ancestor directory path under .tombstones/.
    // The BTreeSet ordering puts shorter prefixes before their children,
    // so creating them in iteration order guarantees parents exist
    // before each set_entry call.
    let mut ancestor_dirs: BTreeSet<Vec<String>> = BTreeSet::new();
    for orig_path in tombstones.keys() {
        let full = format!("{TOMBSTONE_PREFIX}/{orig_path}");
        let segs: Vec<&str> = full.split('/').filter(|s| !s.is_empty()).collect();
        for depth in 1..segs.len() {
            ancestor_dirs.insert(segs[..depth].iter().map(|s| (*s).to_string()).collect());
        }
    }
    for dir_path in &ancestor_dirs {
        root = ensure_dir(tree, &root, dir_path).await?;
    }
    for (orig_path, ts) in tombstones {
        let full = format!("{TOMBSTONE_PREFIX}/{orig_path}");
        let segs: Vec<&str> = full.split('/').filter(|s| !s.is_empty()).collect();
        let (name, parent_segs) = segs
            .split_last()
            .expect("tombstone path always has at least one segment");
        let bytes = ts.to_string().into_bytes();
        let (cid, size) = tree.put(&bytes).await?;
        root = tree
            .set_entry(&root, parent_segs, name, &cid, size, LinkType::Blob)
            .await?;
    }
    Ok(root)
}

/// Create `dir_path` as a directory under `root` if it isn't already.
/// All ancestors of `dir_path` must already exist (call this in
/// shortest-prefix-first order over a set of paths).
async fn ensure_dir<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    dir_path: &[String],
) -> Result<Cid, IndexError> {
    let segs: Vec<&str> = dir_path.iter().map(String::as_str).collect();
    let (name, parent_segs) = segs.split_last().expect("dir_path must be non-empty");
    let parent_cid = resolve_dir(tree, root, parent_segs).await?;
    let entries = tree.list_directory(&parent_cid).await?;
    if entries
        .iter()
        .any(|e| e.name == *name && e.link_type == LinkType::Dir)
    {
        return Ok(root.clone());
    }
    let empty = tree.put_directory(Vec::new()).await?;
    let new_root = tree
        .set_entry(root, parent_segs, name, &empty, 0, LinkType::Dir)
        .await?;
    Ok(new_root)
}

/// Resolve `segments` down from `root`, returning the CID of the
/// directory at that path. Empty `segments` returns `root`.
async fn resolve_dir<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    segments: &[&str],
) -> Result<Cid, IndexError> {
    let mut current = root.clone();
    for seg in segments {
        let entries = tree.list_directory(&current).await?;
        let entry = entries
            .iter()
            .find(|e| e.name == *seg && e.link_type == LinkType::Dir)
            .ok_or_else(|| IndexError::Tree(HashTreeError::PathNotFound((*seg).to_string())))?;
        current = Cid {
            hash: entry.hash,
            key: entry.key,
        };
    }
    Ok(current)
}

fn index_dir_inner<'a, S: Store>(
    tree: &'a HashTree<S>,
    dir: &'a Path,
) -> futures::future::BoxFuture<'a, Result<Cid, IndexError>> {
    Box::pin(async move {
        let mut entries: Vec<DirEntry> = Vec::new();
        let mut children: Vec<(String, std::path::PathBuf)> = Vec::new();

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .into_string()
                .map_err(|s| IndexError::NonUtf8(s.to_string_lossy().into_owned()))?;
            children.push((name, entry.path()));
        }

        // Sort for determinism.
        children.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, path) in children {
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) => return Err(IndexError::Io(e)),
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                let child_cid = index_dir_inner(tree, &path).await?;
                let mut e = DirEntry::from_cid(name, &child_cid);
                e.link_type = LinkType::Dir;
                entries.push(e);
            } else if metadata.is_file() {
                let bytes = std::fs::read(&path)?;
                let size = bytes.len() as u64;
                let (cid, _) = tree.put(&bytes).await?;
                let link_type = if size > hashtree_core::DEFAULT_CHUNK_SIZE as u64 {
                    LinkType::File
                } else {
                    LinkType::Blob
                };
                let mut e = DirEntry::from_cid(name, &cid).with_size(size);
                e.link_type = link_type;
                entries.push(e);
            }
        }

        let cid = tree.put_directory(entries).await?;
        Ok(cid)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashtree_core::{HashTreeConfig, MemoryStore};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn new_tree() -> HashTree<MemoryStore> {
        HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public())
    }

    #[tokio::test]
    async fn empty_dir_indexes_to_empty_htree_dir() {
        let dir = tempdir().unwrap();
        let tree = new_tree();
        let cid = index_dir(&tree, dir.path()).await.unwrap();
        let listing = tree.list_directory(&cid).await.unwrap();
        assert!(listing.is_empty());
    }

    #[tokio::test]
    async fn single_file_appears_with_correct_name_and_size() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), b"hi there").unwrap();
        let tree = new_tree();
        let cid = index_dir(&tree, dir.path()).await.unwrap();
        let listing = tree.list_directory(&cid).await.unwrap();
        assert_eq!(listing.len(), 1);
        assert_eq!(listing[0].name, "hello.txt");
        assert_eq!(listing[0].size, 8);
    }

    #[tokio::test]
    async fn nested_dir_indexed_recursively() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("a.txt"), b"a").unwrap();
        let tree = new_tree();
        let cid = index_dir(&tree, dir.path()).await.unwrap();
        let top = tree.list_directory(&cid).await.unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].name, "sub");
        let sub_cid = Cid {
            hash: top[0].hash,
            key: top[0].key,
        };
        let sub = tree.list_directory(&sub_cid).await.unwrap();
        assert_eq!(sub.len(), 1);
        assert_eq!(sub[0].name, "a.txt");
    }

    #[tokio::test]
    async fn indexing_is_deterministic() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"alpha").unwrap();
        std::fs::write(dir.path().join("b"), b"beta").unwrap();
        std::fs::create_dir(dir.path().join("inner")).unwrap();
        std::fs::write(dir.path().join("inner").join("c"), b"gamma").unwrap();
        let cid_1 = index_dir(&new_tree(), dir.path()).await.unwrap();
        let cid_2 = index_dir(&new_tree(), dir.path()).await.unwrap();
        assert_eq!(cid_1.hash, cid_2.hash);
    }

    #[tokio::test]
    async fn different_contents_produce_different_cids() {
        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();
        std::fs::write(dir_a.path().join("a.txt"), b"alpha").unwrap();
        std::fs::write(dir_b.path().join("a.txt"), b"different").unwrap();
        let cid_a = index_dir(&new_tree(), dir_a.path()).await.unwrap();
        let cid_b = index_dir(&new_tree(), dir_b.path()).await.unwrap();
        assert_ne!(cid_a.hash, cid_b.hash);
    }

    #[tokio::test]
    async fn symlinks_are_ignored() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("real.txt"), b"real").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path().join("real.txt"), dir.path().join("link.txt"))
            .unwrap();
        let tree = new_tree();
        let cid = index_dir(&tree, dir.path()).await.unwrap();
        let listing = tree.list_directory(&cid).await.unwrap();
        // On Unix we expect only the real file; on non-Unix the symlink
        // isn't created so we also expect just one entry. Either way:
        assert_eq!(listing.len(), 1);
        assert_eq!(listing[0].name, "real.txt");
    }

    #[tokio::test]
    async fn non_existent_dir_errors() {
        let tree = new_tree();
        let err = index_dir(&tree, Path::new("/this/should/not/exist/abcxyz"))
            .await
            .unwrap_err();
        assert!(matches!(err, IndexError::NotADirectory(_)));
    }

    // ----- index_dir_with_history / tombstone lifecycle -----

    #[tokio::test]
    async fn history_with_no_previous_root_matches_index_dir() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        let tree = new_tree();
        let cid_plain = index_dir(&tree, dir.path()).await.unwrap();
        let cid_history = index_dir_with_history(&tree, dir.path(), None, 1000)
            .await
            .unwrap();
        assert_eq!(cid_plain.hash, cid_history.hash);
    }

    #[tokio::test]
    async fn removed_file_emits_tombstone_in_next_import() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("removed.txt"), b"bye").unwrap();
        let tree = new_tree();
        let first = index_dir(&tree, dir.path()).await.unwrap();

        // Remove the file, re-import with history.
        std::fs::remove_file(dir.path().join("removed.txt")).unwrap();
        let second = index_dir_with_history(&tree, dir.path(), Some(&first), 1234)
            .await
            .unwrap();

        let (files, tombstones) = crate::merge::walk_device_tree(&tree, &second)
            .await
            .unwrap();
        assert!(files.is_empty(), "no live files expected, got {files:?}");
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].path, "removed.txt");
        assert_eq!(tombstones[0].tombstoned_at, 1234);
    }

    #[tokio::test]
    async fn tombstone_carries_forward_when_file_stays_absent() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("gone.txt"), b"x").unwrap();
        let tree = new_tree();
        let first = index_dir(&tree, dir.path()).await.unwrap();
        std::fs::remove_file(dir.path().join("gone.txt")).unwrap();
        let second = index_dir_with_history(&tree, dir.path(), Some(&first), 1000)
            .await
            .unwrap();

        // Third import: file still absent, tombstone should keep its
        // original timestamp (1000), not be refreshed to 2000.
        let third = index_dir_with_history(&tree, dir.path(), Some(&second), 2000)
            .await
            .unwrap();
        let (_, tombstones) = crate::merge::walk_device_tree(&tree, &third).await.unwrap();
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].tombstoned_at, 1000, "original ts preserved");
    }

    #[tokio::test]
    async fn tombstone_drops_when_file_returns() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("back.txt"), b"v1").unwrap();
        let tree = new_tree();
        let first = index_dir(&tree, dir.path()).await.unwrap();
        std::fs::remove_file(dir.path().join("back.txt")).unwrap();
        let second = index_dir_with_history(&tree, dir.path(), Some(&first), 1000)
            .await
            .unwrap();

        // File comes back.
        std::fs::write(dir.path().join("back.txt"), b"v2").unwrap();
        let third = index_dir_with_history(&tree, dir.path(), Some(&second), 2000)
            .await
            .unwrap();
        let (files, tombstones) = crate::merge::walk_device_tree(&tree, &third).await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "back.txt");
        assert!(tombstones.is_empty(), "tombstone should be gone");
    }

    #[tokio::test]
    async fn nested_file_removal_writes_nested_tombstone() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("photos")).unwrap();
        std::fs::write(dir.path().join("photos").join("img.heic"), b"photo").unwrap();
        let tree = new_tree();
        let first = index_dir(&tree, dir.path()).await.unwrap();

        std::fs::remove_file(dir.path().join("photos").join("img.heic")).unwrap();
        let second = index_dir_with_history(&tree, dir.path(), Some(&first), 5000)
            .await
            .unwrap();

        let (_, tombstones) = crate::merge::walk_device_tree(&tree, &second)
            .await
            .unwrap();
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].path, "photos/img.heic");
        assert_eq!(tombstones[0].tombstoned_at, 5000);
    }

    #[tokio::test]
    async fn surviving_files_unaffected_by_unrelated_removal() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("keep.txt"), b"k").unwrap();
        std::fs::write(dir.path().join("drop.txt"), b"d").unwrap();
        let tree = new_tree();
        let first = index_dir(&tree, dir.path()).await.unwrap();

        std::fs::remove_file(dir.path().join("drop.txt")).unwrap();
        let second = index_dir_with_history(&tree, dir.path(), Some(&first), 1000)
            .await
            .unwrap();

        let (files, tombstones) = crate::merge::walk_device_tree(&tree, &second)
            .await
            .unwrap();
        let live_paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        let tomb_paths: Vec<&str> = tombstones.iter().map(|t| t.path.as_str()).collect();
        assert_eq!(live_paths, vec!["keep.txt"]);
        assert_eq!(tomb_paths, vec!["drop.txt"]);
    }
}
