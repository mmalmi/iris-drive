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

pub(crate) fn should_ignore_name(name: &str) -> bool {
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
        let mut current_paths: BTreeSet<String> = BTreeSet::new();
        collect_local_file_paths(dir, "", &mut current_paths)?;
        root = attach_history_for_current_paths(tree, root, prev, now_unix_seconds, &current_paths)
            .await?;
    }
    if let Some(meta) = root_meta {
        root = layer_root_meta(tree, root, meta).await?;
    }
    Ok(root)
}

/// Like [`index_dir_with_history_and_meta`], but starts from a hashtree root
/// that already represents the user-visible directory. This is used by
/// virtual mounts / file-provider adapters where bytes live in hashtree rather
/// than a normal materialized folder on disk.
pub async fn layer_history_and_meta_on_root<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    previous_root: Option<&Cid>,
    now_unix_seconds: i64,
    root_meta: Option<&DriveRootMeta>,
) -> Result<Cid, IndexError> {
    if let Some(prev) = previous_root {
        let (current_files, _) = walk_device_tree(tree, &root)
            .await
            .map_err(IndexError::Tree)?;
        let current_paths: BTreeSet<String> =
            current_files.into_iter().map(|file| file.path).collect();
        root = attach_history_for_current_paths(tree, root, prev, now_unix_seconds, &current_paths)
            .await?;
    }
    if let Some(meta) = root_meta {
        root = layer_root_meta(tree, root, meta).await?;
    }
    Ok(root)
}

async fn attach_history_for_current_paths<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    prev: &Cid,
    now_unix_seconds: i64,
    current_paths: &BTreeSet<String>,
) -> Result<Cid, IndexError> {
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
mod tests;
