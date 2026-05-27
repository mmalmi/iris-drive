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

use hashtree_core::{Cid, DirEntry, HashTree, HashTreeError, LinkType, Store, TreeEntry, to_hex};
use thiserror::Error;

use crate::conflict::ConflictRecord;
use crate::merge::{
    CONFLICTS_PREFIX, META_DIR, PREV_LINK_PATH, ROOT_META_PATH, WHOLE_FILE_HASH_META_KEY,
    walk_device_tree,
};
use crate::root_meta::DriveRootMeta;

const IGNORED_NAMES: &[&str] = &[
    ".DS_Store",
    ".hashtree",
    ".Trash",
    "$RECYCLE.BIN",
    "Thumbs.db",
    "desktop.ini",
];

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

pub fn should_ignore_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    IGNORED_NAMES
        .iter()
        .any(|ignored| name.eq_ignore_ascii_case(ignored))
        || name.starts_with("._")
        || lower.starts_with(".trash-")
        || name.ends_with('~')
        || (name.starts_with('#') && name.ends_with('#'))
        || Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sbak"))
}

#[must_use]
pub fn path_has_ignored_component(path: &str) -> bool {
    path.split('/').any(should_ignore_name)
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
        root = attach_history_for_current_paths(
            tree,
            root,
            Some(prev),
            Some(prev),
            now_unix_seconds,
            &current_paths,
            None,
        )
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
    root: Cid,
    previous_root: Option<&Cid>,
    now_unix_seconds: i64,
    root_meta: Option<&DriveRootMeta>,
) -> Result<Cid, IndexError> {
    layer_history_and_meta_on_root_with_tombstone_base(
        tree,
        root,
        previous_root,
        previous_root,
        now_unix_seconds,
        root_meta,
    )
    .await
}

/// Like [`layer_history_and_meta_on_root`], but lets virtual providers pass the
/// merged root that was actually displayed to the user as the deletion base.
pub async fn layer_history_and_meta_on_root_with_tombstone_base<S: Store>(
    tree: &HashTree<S>,
    root: Cid,
    previous_root: Option<&Cid>,
    tombstone_base_root: Option<&Cid>,
    now_unix_seconds: i64,
    root_meta: Option<&DriveRootMeta>,
) -> Result<Cid, IndexError> {
    layer_history_and_meta_on_root_with_tombstone_base_and_paths(
        tree,
        root,
        previous_root,
        tombstone_base_root,
        now_unix_seconds,
        root_meta,
        None,
    )
    .await
}

/// Like [`layer_history_and_meta_on_root_with_tombstone_base`], but limits new
/// tombstones to the supplied paths. This is for event-driven providers that
/// know which paths were explicitly deleted, while their projected root may be
/// temporarily missing unrelated remote files.
pub async fn layer_history_and_meta_on_root_with_tombstone_base_and_paths<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    previous_root: Option<&Cid>,
    tombstone_base_root: Option<&Cid>,
    now_unix_seconds: i64,
    root_meta: Option<&DriveRootMeta>,
    tombstone_paths: Option<&BTreeSet<String>>,
) -> Result<Cid, IndexError> {
    let phase = std::time::Instant::now();
    root = filter_ignored_entries_from_root(tree, &root).await?;
    tracing::debug!(
        elapsed_ms = phase.elapsed().as_millis(),
        "history layer filtered ignored entries"
    );
    if previous_root.is_some() || tombstone_base_root.is_some() {
        let phase = std::time::Instant::now();
        let (current_files, _) = walk_device_tree(tree, &root)
            .await
            .map_err(IndexError::Tree)?;
        let current_paths: BTreeSet<String> =
            current_files.into_iter().map(|file| file.path).collect();
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            "history layer walked current root"
        );
        let phase = std::time::Instant::now();
        root = attach_history_for_current_paths(
            tree,
            root,
            previous_root,
            tombstone_base_root,
            now_unix_seconds,
            &current_paths,
            tombstone_paths,
        )
        .await?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            "history layer attached history"
        );
    }
    if let Some(meta) = root_meta {
        let phase = std::time::Instant::now();
        root = layer_root_meta(tree, root, meta).await?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            "history layer attached root metadata"
        );
    }
    Ok(root)
}

#[derive(Debug, Clone)]
pub struct VisibleImportDelta {
    pub root: Cid,
    pub tombstone_paths: BTreeSet<String>,
}

/// Build this device's local contribution from an edited merged-visible root.
///
/// Virtual mounts expose a merged view, but publishing that entire view as the
/// current device's root would re-author unchanged remote files locally. Instead
/// this keeps only the previous local files that were still visible in the base
/// view, then applies changed files from the edited view.
pub async fn local_visible_root_for_mount_import<S: Store>(
    tree: &HashTree<S>,
    edited_root: &Cid,
    previous_root: Option<&Cid>,
    base_root: &Cid,
    tombstone_paths: Option<&BTreeSet<String>>,
) -> Result<VisibleImportDelta, IndexError> {
    let edited_root = filter_ignored_entries_from_root(tree, edited_root).await?;
    let base_root = filter_ignored_entries_from_root(tree, base_root).await?;

    let mut edited_files = BTreeMap::new();
    collect_visible_files(tree, &edited_root, "", &mut edited_files).await?;
    let mut edited_dirs = BTreeSet::new();
    collect_visible_dirs(tree, &edited_root, "", &mut edited_dirs).await?;
    let mut base_files = BTreeMap::new();
    collect_visible_files(tree, &base_root, "", &mut base_files).await?;
    let mut base_dirs = BTreeSet::new();
    collect_visible_dirs(tree, &base_root, "", &mut base_dirs).await?;

    let previous_visible_root = match previous_root {
        Some(previous_root) => filter_ignored_entries_from_root(tree, previous_root).await?,
        None => tree.put_directory(Vec::new()).await?,
    };
    let mut previous_files = BTreeMap::new();
    collect_visible_files(tree, &previous_visible_root, "", &mut previous_files).await?;
    let mut previous_dirs = BTreeSet::new();
    collect_visible_dirs(tree, &previous_visible_root, "", &mut previous_dirs).await?;

    let mut root = tree.put_directory(Vec::new()).await?;
    for path in previous_dirs.intersection(&base_dirs) {
        root = set_visible_dir(tree, root, path).await?;
    }
    for (path, previous) in &previous_files {
        if base_files
            .get(path)
            .is_some_and(|base| visible_entry_matches(base, previous))
        {
            root = set_visible_file_entry(tree, &root, path, previous).await?;
        }
    }

    let mut changed_paths = base_files
        .keys()
        .chain(edited_files.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut deleted_paths = BTreeSet::new();
    for path in std::mem::take(&mut changed_paths) {
        let base = base_files.get(&path);
        let edited = edited_files.get(&path);
        if base
            .zip(edited)
            .is_some_and(|(base, edited)| visible_entry_matches(base, edited))
        {
            continue;
        }
        match edited {
            Some(entry) => {
                root = set_visible_file_entry(tree, &root, &path, entry).await?;
            }
            None if tombstone_path_allowed(tombstone_paths, &path) => {
                deleted_paths.insert(path.clone());
                root = remove_visible_path_if_present(tree, &root, &path).await?;
            }
            None => {}
        }
    }

    let changed_dirs = base_dirs
        .union(&edited_dirs)
        .cloned()
        .collect::<BTreeSet<_>>();
    for path in changed_dirs {
        match (base_dirs.contains(&path), edited_dirs.contains(&path)) {
            (false, true) => {
                root = set_visible_dir(tree, root, &path).await?;
            }
            (true, false) => {
                if tombstone_path_allowed(tombstone_paths, &path) {
                    deleted_paths.insert(path.clone());
                }
                root = remove_visible_path_if_present(tree, &root, &path).await?;
            }
            _ => {}
        }
    }

    Ok(VisibleImportDelta {
        root,
        tombstone_paths: deleted_paths,
    })
}

pub async fn filter_ignored_entries_from_root<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
) -> Result<Cid, IndexError> {
    let (filtered, _) = filter_ignored_entries_from_dir(tree, root).await?;
    Ok(filtered)
}

fn filter_ignored_entries_from_dir<'a, S: Store>(
    tree: &'a HashTree<S>,
    dir: &'a Cid,
) -> futures::future::BoxFuture<'a, Result<(Cid, bool), IndexError>> {
    Box::pin(async move {
        let entries = tree.list_directory(dir).await?;
        let mut changed = false;
        let mut filtered = Vec::with_capacity(entries.len());

        for entry in entries {
            if should_ignore_name(&entry.name) {
                changed = true;
                continue;
            }

            let mut entry = DirEntry {
                name: entry.name,
                hash: entry.hash,
                size: entry.size,
                key: entry.key,
                link_type: entry.link_type,
                meta: entry.meta,
            };
            if entry.link_type == LinkType::Dir {
                let child = Cid {
                    hash: entry.hash,
                    key: entry.key,
                };
                let (filtered_child, child_changed) =
                    filter_ignored_entries_from_dir(tree, &child).await?;
                if child_changed {
                    entry.hash = filtered_child.hash;
                    entry.key = filtered_child.key;
                    changed = true;
                }
            }
            filtered.push(entry);
        }

        if changed {
            Ok((tree.put_directory(filtered).await?, true))
        } else {
            Ok((dir.clone(), false))
        }
    })
}

async fn attach_history_for_current_paths<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    previous_root: Option<&Cid>,
    tombstone_base_root: Option<&Cid>,
    now_unix_seconds: i64,
    current_paths: &BTreeSet<String>,
    tombstone_paths: Option<&BTreeSet<String>>,
) -> Result<Cid, IndexError> {
    let mut tombstones: BTreeMap<String, i64> = BTreeMap::new();
    if let Some(paths) = tombstone_paths {
        for path in paths {
            if !current_paths.contains(path) {
                tombstones.insert(path.clone(), now_unix_seconds);
            }
        }
    }

    // Files that were in the root being edited but are no longer visible get a
    // fresh tombstone stamped at import time.
    if let Some(base) = tombstone_base_root {
        let phase = std::time::Instant::now();
        let (base_files, _base_tombstones) = walk_device_tree(tree, base)
            .await
            .map_err(IndexError::Tree)?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            file_count = base_files.len(),
            "history attach walked tombstone base"
        );
        for f in base_files {
            if !current_paths.contains(&f.path) && tombstone_path_allowed(tombstone_paths, &f.path)
            {
                tombstones.insert(f.path, now_unix_seconds);
            }
        }
    }

    if let Some(prev) = previous_root {
        let phase = std::time::Instant::now();
        let (prev_files, prev_tombstones) = walk_device_tree(tree, prev)
            .await
            .map_err(IndexError::Tree)?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            file_count = prev_files.len(),
            tombstone_count = prev_tombstones.len(),
            "history attach walked previous root"
        );
        for f in prev_files {
            if !current_paths.contains(&f.path) && tombstone_path_allowed(tombstone_paths, &f.path)
            {
                tombstones.entry(f.path).or_insert(now_unix_seconds);
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
    }

    if !tombstones.is_empty() {
        let phase = std::time::Instant::now();
        root = layer_tombstones(tree, root, &tombstones).await?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            tombstone_count = tombstones.len(),
            "history attach layered tombstones"
        );
    }

    // Add the revision back-link: a `._prev` entry at the root pointing
    // at the prior root's Cid (hash + key). Capability propagates
    // automatically when readers decrypt the new TreeNode — the prior
    // TreeNode is now navigable from the current one.
    if let Some(prev) = previous_root {
        let phase = std::time::Instant::now();
        root = layer_prev_link(tree, root, prev).await?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            "history attach layered previous link"
        );
    }

    Ok(root)
}

fn tombstone_path_allowed(tombstone_paths: Option<&BTreeSet<String>>, path: &str) -> bool {
    match tombstone_paths {
        Some(paths) => paths.iter().any(|allowed| {
            path == allowed
                || path
                    .strip_prefix(allowed)
                    .is_some_and(|rest| rest.starts_with('/'))
        }),
        None => true,
    }
}

fn collect_visible_files<'a, S: Store>(
    tree: &'a HashTree<S>,
    root: &'a Cid,
    prefix: &'a str,
    out: &'a mut BTreeMap<String, TreeEntry>,
) -> futures::future::BoxFuture<'a, Result<(), IndexError>> {
    Box::pin(async move {
        let entries = tree.list_directory(root).await?;
        for entry in entries {
            if should_ignore_name(&entry.name) {
                continue;
            }
            let path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{prefix}/{}", entry.name)
            };
            if entry.link_type == LinkType::Dir {
                let cid = Cid {
                    hash: entry.hash,
                    key: entry.key,
                };
                collect_visible_files(tree, &cid, &path, out).await?;
            } else {
                out.insert(path, entry);
            }
        }
        Ok(())
    })
}

fn collect_visible_dirs<'a, S: Store>(
    tree: &'a HashTree<S>,
    root: &'a Cid,
    prefix: &'a str,
    out: &'a mut BTreeSet<String>,
) -> futures::future::BoxFuture<'a, Result<(), IndexError>> {
    Box::pin(async move {
        let entries = tree.list_directory(root).await?;
        for entry in entries {
            if should_ignore_name(&entry.name) || entry.link_type != LinkType::Dir {
                continue;
            }
            let path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{prefix}/{}", entry.name)
            };
            out.insert(path.clone());
            let cid = Cid {
                hash: entry.hash,
                key: entry.key,
            };
            collect_visible_dirs(tree, &cid, &path, out).await?;
        }
        Ok(())
    })
}

fn visible_entry_matches(left: &TreeEntry, right: &TreeEntry) -> bool {
    left.hash == right.hash
        && left.key == right.key
        && left.size == right.size
        && left.link_type == right.link_type
        && left.meta == right.meta
}

async fn set_visible_dir<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    path: &str,
) -> Result<Cid, IndexError> {
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    for depth in 1..=segments.len() {
        root = ensure_dir(tree, &root, &segments[..depth]).await?;
    }
    Ok(root)
}

async fn set_visible_file_entry<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    path: &str,
    entry: &TreeEntry,
) -> Result<Cid, IndexError> {
    let (parent, name) = split_visible_path(path)?;
    let mut root = ensure_parent_dirs(tree, root.clone(), &parent).await?;
    let parent_cid = resolve_dir(tree, &root, &parent).await?;
    let mut entries = tree
        .list_directory(&parent_cid)
        .await?
        .into_iter()
        .filter(|existing| existing.name != name)
        .map(|existing| DirEntry {
            name: existing.name,
            hash: existing.hash,
            size: existing.size,
            key: existing.key,
            link_type: existing.link_type,
            meta: existing.meta,
        })
        .collect::<Vec<_>>();
    entries.push(DirEntry {
        name: name.to_string(),
        hash: entry.hash,
        size: entry.size,
        key: entry.key,
        link_type: entry.link_type,
        meta: entry.meta.clone(),
    });
    let parent_cid = tree.put_directory(entries).await?;
    if parent.is_empty() {
        return Ok(parent_cid);
    }
    root = tree
        .set_entry(
            &root,
            &parent[..parent.len() - 1],
            parent[parent.len() - 1],
            &parent_cid,
            0,
            LinkType::Dir,
        )
        .await?;
    Ok(root)
}

async fn ensure_parent_dirs<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    parent: &[&str],
) -> Result<Cid, IndexError> {
    let mut current = Vec::new();
    for segment in parent {
        current.push((*segment).to_string());
        root = ensure_dir(tree, &root, &current).await?;
    }
    Ok(root)
}

async fn remove_visible_path_if_present<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    path: &str,
) -> Result<Cid, IndexError> {
    let (parent, name) = split_visible_path(path)?;
    let parent_cid = match resolve_dir(tree, root, &parent).await {
        Ok(parent_cid) => parent_cid,
        Err(IndexError::Tree(HashTreeError::PathNotFound(_))) => return Ok(root.clone()),
        Err(error) => return Err(error),
    };
    let entries = tree.list_directory(&parent_cid).await?;
    if entries.iter().all(|entry| entry.name != name) {
        return Ok(root.clone());
    }
    tree.remove_entry(root, &parent, name)
        .await
        .map_err(Into::into)
}

fn split_visible_path(path: &str) -> Result<(Vec<&str>, &str), IndexError> {
    let mut segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    let Some(name) = segments.pop() else {
        return Err(IndexError::Tree(HashTreeError::PathNotFound(path.into())));
    };
    Ok((segments, name))
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
    let mut tombstone_tree = TombstoneDir::default();
    for (orig_path, ts) in tombstones {
        tombstone_tree.insert(orig_path, *ts);
    }
    let tombstone_root = materialize_tombstone_dir(tree, &tombstone_tree).await?;
    let meta_root = tree
        .put_directory(vec![
            DirEntry::from_cid("tombstones", &tombstone_root).with_link_type(LinkType::Dir),
        ])
        .await?;
    root = tree
        .set_entry(&root, &[], META_DIR, &meta_root, 0, LinkType::Dir)
        .await?;
    Ok(root)
}

#[derive(Default)]
struct TombstoneDir {
    dirs: BTreeMap<String, TombstoneDir>,
    files: BTreeMap<String, i64>,
}

impl TombstoneDir {
    fn insert(&mut self, path: &str, tombstoned_at: i64) {
        let parts: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
        self.insert_parts(&parts, tombstoned_at);
    }

    fn insert_parts(&mut self, parts: &[&str], tombstoned_at: i64) {
        match parts {
            [] => {}
            [name] => {
                self.files.insert((*name).to_string(), tombstoned_at);
            }
            [dir, rest @ ..] => {
                self.dirs
                    .entry((*dir).to_string())
                    .or_default()
                    .insert_parts(rest, tombstoned_at);
            }
        }
    }
}

fn materialize_tombstone_dir<'a, S: Store>(
    tree: &'a HashTree<S>,
    dir: &'a TombstoneDir,
) -> futures::future::BoxFuture<'a, Result<Cid, IndexError>> {
    Box::pin(async move {
        let mut entries = Vec::with_capacity(dir.dirs.len() + dir.files.len());
        for (name, child) in &dir.dirs {
            let cid = materialize_tombstone_dir(tree, child).await?;
            entries.push(DirEntry::from_cid(name, &cid).with_link_type(LinkType::Dir));
        }
        for (name, tombstoned_at) in &dir.files {
            let bytes = tombstoned_at.to_string().into_bytes();
            let (cid, size) = tree.put(&bytes).await?;
            entries.push(
                DirEntry::from_cid(name, &cid)
                    .with_size(size)
                    .with_link_type(LinkType::Blob),
            );
        }
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(tree.put_directory(entries).await?)
    })
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
