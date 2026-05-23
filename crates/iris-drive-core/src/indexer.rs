//! Walk a local directory and build the equivalent hashtree directory.
//!
//! Used in two situations:
//! - First-time import: a user points iris-drive at an existing folder.
//! - Sync engine: compute the local CID before publishing a new root.
//!
//! The indexer is deterministic — the same on-disk tree always produces
//! the same root CID — and tests exercise that property directly.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

use hashtree_core::{Cid, DirEntry, HashTree, HashTreeError, LinkType, Store, to_hex};
use thiserror::Error;

use crate::conflict::ConflictRecord;
use crate::merge::{
    CONFLICTS_PREFIX, META_DIR, PREV_LINK_PATH, ROOT_META_PATH, TOMBSTONE_PREFIX,
    WHOLE_FILE_HASH_META_KEY, walk_device_tree,
};
use crate::root_meta::DriveRootMeta;

const IGNORED_NAMES: &[&str] = &[".DS_Store", ".hashtree", "Thumbs.db", "desktop.ini"];

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
    #[error("root metadata: {0}")]
    RootMeta(String),
    #[error("conflict record: {0}")]
    ConflictRecord(String),
}

fn should_ignore_name(name: &str) -> bool {
    IGNORED_NAMES.contains(&name)
        || name.starts_with("._")
        || name.ends_with('~')
        || (name.starts_with('#') && name.ends_with('#'))
        || Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sbak"))
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
    index_dir_with_history_and_meta(tree, dir, previous_root, now_unix_seconds, None).await
}

/// Like [`index_dir_with_history`], but embeds optional root-level
/// causal metadata at `.hashtree/root.json`.
pub async fn index_dir_with_history_and_meta<S: Store>(
    tree: &HashTree<S>,
    dir: &Path,
    previous_root: Option<&Cid>,
    now_unix_seconds: i64,
    root_meta: Option<&DriveRootMeta>,
) -> Result<Cid, IndexError> {
    let mut root = index_dir(tree, dir).await?;
    if let Some(prev) = previous_root {
        root = attach_history(tree, dir, root, prev, now_unix_seconds).await?;
    }
    if let Some(meta) = root_meta {
        root = layer_root_meta(tree, root, meta).await?;
    }
    Ok(root)
}

async fn attach_history<S: Store>(
    tree: &HashTree<S>,
    dir: &Path,
    mut root: Cid,
    prev: &Cid,
    now_unix_seconds: i64,
) -> Result<Cid, IndexError> {
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
    root = layer_prev_link(tree, root, prev).await?;

    Ok(root)
}

pub async fn layer_root_meta<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    meta: &DriveRootMeta,
) -> Result<Cid, IndexError> {
    let segments: Vec<&str> = ROOT_META_PATH
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let (name, parent_segs) = segments.split_last().expect("ROOT_META_PATH is non-empty");
    for depth in 1..=parent_segs.len() {
        let dir_path: Vec<String> = parent_segs[..depth]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        root = ensure_dir(tree, &root, &dir_path).await?;
    }
    let bytes = serde_json::to_vec_pretty(meta).map_err(|e| IndexError::RootMeta(e.to_string()))?;
    let (cid, size) = tree.put(&bytes).await?;
    let new_root = tree
        .set_entry(&root, parent_segs, name, &cid, size, LinkType::Blob)
        .await?;
    Ok(new_root)
}

/// Read root-level causal metadata from `.hashtree/root.json`, if
/// present. Absence is normal for legacy roots.
pub async fn read_root_meta<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
) -> Result<Option<DriveRootMeta>, IndexError> {
    let entries = tree.list_directory(root).await?;
    let Some(meta_entry) = entries
        .iter()
        .find(|e| e.name == META_DIR && e.link_type == LinkType::Dir)
    else {
        return Ok(None);
    };
    let meta_cid = Cid {
        hash: meta_entry.hash,
        key: meta_entry.key,
    };
    let meta_entries = tree.list_directory(&meta_cid).await?;
    let name = ROOT_META_PATH
        .rsplit_once('/')
        .map_or(ROOT_META_PATH, |(_, name)| name);
    let Some(root_meta) = meta_entries
        .iter()
        .find(|e| e.name == name && e.link_type != LinkType::Dir)
    else {
        return Ok(None);
    };
    let cid = Cid {
        hash: root_meta.hash,
        key: root_meta.key,
    };
    let raw = tree
        .get(&cid, None)
        .await?
        .ok_or_else(|| HashTreeError::MissingChunk(to_hex(&root_meta.hash)))?;
    let meta = serde_json::from_slice(&raw).map_err(|e| IndexError::RootMeta(e.to_string()))?;
    Ok(Some(meta))
}

/// Add or replace durable conflict provenance records under
/// `.hashtree/conflicts/<conflict_id>.json`.
pub async fn layer_conflict_records<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    records: &[ConflictRecord],
) -> Result<Cid, IndexError> {
    if records.is_empty() {
        return Ok(root);
    }
    let conflict_dir: Vec<&str> = CONFLICTS_PREFIX
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    for depth in 1..=conflict_dir.len() {
        let dir_path: Vec<String> = conflict_dir[..depth]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        root = ensure_dir(tree, &root, &dir_path).await?;
    }

    for record in records {
        let name = conflict_record_leaf_name(&record.conflict_id)?;
        let bytes = serde_json::to_vec_pretty(record)
            .map_err(|e| IndexError::ConflictRecord(e.to_string()))?;
        let (cid, size) = tree.put(&bytes).await?;
        root = tree
            .set_entry(&root, &conflict_dir, &name, &cid, size, LinkType::Blob)
            .await?;
    }
    Ok(root)
}

/// Read durable conflict provenance records from `.hashtree/conflicts`.
/// Absence is normal for snapshots without conflicts.
pub async fn read_conflict_records<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
) -> Result<Vec<ConflictRecord>, IndexError> {
    let Some(conflicts_dir) = resolve_conflicts_dir(tree, root).await? else {
        return Ok(Vec::new());
    };
    let mut entries = tree.list_directory(&conflicts_dir).await?;
    entries.sort_by(|a, b| a.name.cmp(&b.name));

    let mut records = Vec::new();
    for entry in entries {
        if entry.link_type == LinkType::Dir {
            continue;
        }
        let Some(conflict_id) = entry.name.strip_suffix(".json") else {
            continue;
        };
        validate_conflict_id(conflict_id)?;
        let cid = Cid {
            hash: entry.hash,
            key: entry.key,
        };
        let raw = tree
            .get(&cid, None)
            .await?
            .ok_or_else(|| HashTreeError::MissingChunk(to_hex(&entry.hash)))?;
        let record: ConflictRecord =
            serde_json::from_slice(&raw).map_err(|e| IndexError::ConflictRecord(e.to_string()))?;
        if record.conflict_id != conflict_id {
            return Err(IndexError::ConflictRecord(format!(
                "conflict_id {} does not match record filename {conflict_id}",
                record.conflict_id
            )));
        }
        records.push(record);
    }
    Ok(records)
}

async fn resolve_conflicts_dir<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
) -> Result<Option<Cid>, IndexError> {
    let root_entries = tree.list_directory(root).await?;
    let Some(meta_entry) = root_entries
        .iter()
        .find(|e| e.name == META_DIR && e.link_type == LinkType::Dir)
    else {
        return Ok(None);
    };
    let meta_cid = Cid {
        hash: meta_entry.hash,
        key: meta_entry.key,
    };
    let meta_entries = tree.list_directory(&meta_cid).await?;
    let conflict_dir_name = CONFLICTS_PREFIX
        .rsplit_once('/')
        .map_or(CONFLICTS_PREFIX, |(_, name)| name);
    let Some(conflicts_entry) = meta_entries
        .iter()
        .find(|e| e.name == conflict_dir_name && e.link_type == LinkType::Dir)
    else {
        return Ok(None);
    };
    Ok(Some(Cid {
        hash: conflicts_entry.hash,
        key: conflicts_entry.key,
    }))
}

fn conflict_record_leaf_name(conflict_id: &str) -> Result<String, IndexError> {
    validate_conflict_id(conflict_id)?;
    Ok(format!("{conflict_id}.json"))
}

fn validate_conflict_id(conflict_id: &str) -> Result<(), IndexError> {
    if conflict_id.is_empty()
        || conflict_id == "."
        || conflict_id == ".."
        || conflict_id.contains('/')
        || conflict_id.contains('\\')
    {
        return Err(IndexError::ConflictRecord(
            "conflict_id must be one non-empty path segment".into(),
        ));
    }
    Ok(())
}

pub async fn layer_prev_link<S: Store>(
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
        if should_ignore_name(&name) {
            continue;
        }
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
            if should_ignore_name(&name) {
                continue;
            }
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
                let whole_file_hash = hashtree_core::sha256(&bytes);
                let (cid, _) = tree.put(&bytes).await?;
                let link_type = if size > hashtree_core::DEFAULT_CHUNK_SIZE as u64 {
                    LinkType::File
                } else {
                    LinkType::Blob
                };
                let mut e = DirEntry::from_cid(name, &cid)
                    .with_size(size)
                    .with_meta(file_entry_meta(&whole_file_hash));
                e.link_type = link_type;
                entries.push(e);
            }
        }

        let cid = tree.put_directory(entries).await?;
        Ok(cid)
    })
}

fn file_entry_meta(whole_file_hash: &[u8; 32]) -> HashMap<String, serde_json::Value> {
    HashMap::from([(
        WHOLE_FILE_HASH_META_KEY.to_string(),
        serde_json::Value::String(to_hex(whole_file_hash)),
    )])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conflict::{ConflictRecord, ConflictSide, ConflictState};
    use crate::root_meta::{DriveRootMeta, RootObservation, RootParent};
    use hashtree_core::{DEFAULT_CHUNK_SIZE, HashTreeConfig, MemoryStore, sha256};
    use std::collections::BTreeMap;
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
    async fn built_in_noise_files_are_ignored() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("real.txt"), b"real").unwrap();
        std::fs::write(dir.path().join(".DS_Store"), b"finder state").unwrap();
        std::fs::write(dir.path().join("._real.txt"), b"resource fork").unwrap();
        std::fs::write(dir.path().join("Thumbs.db"), b"windows state").unwrap();
        std::fs::write(dir.path().join("desktop.ini"), b"windows metadata").unwrap();
        std::fs::write(dir.path().join("draft~"), b"editor backup").unwrap();
        std::fs::write(dir.path().join("#draft#"), b"emacs temp").unwrap();
        std::fs::write(dir.path().join("backup.sbak"), b"seafile backup").unwrap();
        std::fs::create_dir(dir.path().join(".hashtree")).unwrap();
        std::fs::write(dir.path().join(".hashtree").join("prev"), b"internal").unwrap();

        let tree = new_tree();
        let cid = index_dir(&tree, dir.path()).await.unwrap();
        let listing = tree.list_directory(&cid).await.unwrap();

        assert_eq!(listing.len(), 1);
        assert_eq!(listing[0].name, "real.txt");
    }

    #[tokio::test]
    async fn ignored_files_do_not_keep_removed_files_alive() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("removed.txt"), b"bye").unwrap();
        std::fs::write(dir.path().join(".DS_Store"), b"finder state").unwrap();
        let tree = new_tree();
        let first = index_dir(&tree, dir.path()).await.unwrap();

        std::fs::remove_file(dir.path().join("removed.txt")).unwrap();
        let second = index_dir_with_history(&tree, dir.path(), Some(&first), 1234)
            .await
            .unwrap();

        let (files, tombstones) = crate::merge::walk_device_tree(&tree, &second)
            .await
            .unwrap();
        assert!(files.is_empty());
        assert_eq!(tombstones.len(), 1);
        assert_eq!(tombstones[0].path, "removed.txt");
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
    async fn root_metadata_is_embedded_under_hashtree_and_not_user_visible() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        let tree = new_tree();
        let meta = DriveRootMeta {
            schema: DriveRootMeta::SCHEMA,
            drive_id: "main".into(),
            device_id: "device-a".into(),
            device_seq: 2,
            dck_generation: 1,
            materialized_only: false,
            parents: vec![RootParent {
                device_id: "device-a".into(),
                device_seq: 1,
                root_cid: "cid-parent".into(),
            }],
            observed: BTreeMap::from([(
                "device-b".into(),
                RootObservation {
                    device_seq: 7,
                    root_cid: "cid-b".into(),
                },
            )]),
            created_at: 1234,
        };

        let root = index_dir_with_history_and_meta(&tree, dir.path(), None, 1234, Some(&meta))
            .await
            .unwrap();

        let loaded = read_root_meta(&tree, &root).await.unwrap().unwrap();
        assert_eq!(loaded, meta);

        let (files, tombstones) = crate::merge::walk_device_tree(&tree, &root).await.unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "a.txt");
        assert!(tombstones.is_empty());
    }

    #[tokio::test]
    async fn indexed_large_files_preserve_whole_file_hash_metadata() {
        let dir = tempdir().unwrap();
        let bytes = vec![42u8; DEFAULT_CHUNK_SIZE + 1];
        let whole_file_hash = sha256(&bytes);
        std::fs::write(dir.path().join("large.bin"), &bytes).unwrap();
        let tree = new_tree();

        let root = index_dir(&tree, dir.path()).await.unwrap();

        let (files, tombstones) = crate::merge::walk_device_tree(&tree, &root).await.unwrap();
        assert!(tombstones.is_empty());
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "large.bin");
        assert_eq!(files[0].whole_file_hash, Some(whole_file_hash));
        assert_ne!(
            files[0].hash, whole_file_hash,
            "large-file CID is a chunk-tree hash, not the whole-file hash"
        );
    }

    #[tokio::test]
    async fn conflict_records_round_trip_and_are_not_user_visible() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        let tree = new_tree();
        let root = index_dir(&tree, dir.path()).await.unwrap();
        let record = ConflictRecord {
            schema: ConflictRecord::SCHEMA,
            conflict_id: "dev-a-2-dev-b-7".into(),
            path: "report.pdf".into(),
            visible_conflict_path: "report (conflict from phone).pdf".into(),
            local: ConflictSide {
                device_id: "dev-a".into(),
                device_seq: 2,
                root_cid: "cid-a".into(),
                whole_file_hash: "hash-a".into(),
            },
            remote: Some(ConflictSide {
                device_id: "dev-b".into(),
                device_seq: 7,
                root_cid: "cid-b".into(),
                whole_file_hash: "hash-b".into(),
            }),
            deleted: None,
            state: ConflictState::Unresolved,
            created_at: 1234,
        };

        let with_conflict = layer_conflict_records(&tree, root, std::slice::from_ref(&record))
            .await
            .unwrap();

        let records = read_conflict_records(&tree, &with_conflict).await.unwrap();
        assert_eq!(records, vec![record]);

        let (files, tombstones) = crate::merge::walk_device_tree(&tree, &with_conflict)
            .await
            .unwrap();
        let file_paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(file_paths, vec!["a.txt"]);
        assert!(tombstones.is_empty());
    }

    #[tokio::test]
    async fn missing_conflict_records_dir_reads_as_empty() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        let tree = new_tree();
        let root = index_dir(&tree, dir.path()).await.unwrap();

        let records = read_conflict_records(&tree, &root).await.unwrap();
        assert!(records.is_empty());
    }

    #[tokio::test]
    async fn conflict_record_id_must_be_single_path_segment() {
        let tree = new_tree();
        let root = tree.put_directory(Vec::new()).await.unwrap();
        let record = ConflictRecord {
            schema: ConflictRecord::SCHEMA,
            conflict_id: "bad/id".into(),
            path: "report.pdf".into(),
            visible_conflict_path: "report (conflict from phone).pdf".into(),
            local: ConflictSide {
                device_id: "dev-a".into(),
                device_seq: 2,
                root_cid: "cid-a".into(),
                whole_file_hash: "hash-a".into(),
            },
            remote: Some(ConflictSide {
                device_id: "dev-b".into(),
                device_seq: 7,
                root_cid: "cid-b".into(),
                whole_file_hash: "hash-b".into(),
            }),
            deleted: None,
            state: ConflictState::Unresolved,
            created_at: 1234,
        };

        let err = layer_conflict_records(&tree, root, &[record])
            .await
            .unwrap_err();
        assert!(matches!(err, IndexError::ConflictRecord(msg) if msg.contains("conflict_id")));
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
