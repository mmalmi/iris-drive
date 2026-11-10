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
use crate::dir_builder::{DirBuilder, LeafLink};
use crate::merge::{
    AppKeyFileEntry, AppKeyTombstone, CONFLICTS_PREFIX, MAX_PATH_KIND_REPLACEMENT_GENERATION,
    META_DIR, MODIFIED_AT_META_KEY, PATH_KIND_REPLACEMENTS_PATH, PREV_LINK_PATH,
    PathKindReplacement, PathKindReplacementDocument, ROOT_META_PATH, WHOLE_FILE_HASH_META_KEY,
    walk_app_key_tree, walk_app_key_tree_indexed,
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
    #[error("path-kind replacements: {0}")]
    PathKindReplacements(String),
}

#[must_use]
pub fn should_ignore_name(name: &str) -> bool {
    const TRASH_PREFIX: &[u8] = b".trash-";
    IGNORED_NAMES
        .iter()
        .any(|ignored| name.eq_ignore_ascii_case(ignored))
        || name.starts_with("._")
        || name
            .as_bytes()
            .get(..TRASH_PREFIX.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(TRASH_PREFIX))
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
/// absent. A visible recreation can coexist with its retained tombstone: the
/// marker remains a causal barrier against older roots, while merge always
/// lets the root's own visible entry win over its own marker.
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
    let root = index_dir(tree, dir).await?;
    if previous_root.is_some() {
        return layer_history_and_meta_on_root_with_tombstone_base_paths_and_replacements(
            tree,
            root,
            previous_root,
            previous_root,
            now_unix_seconds,
            root_meta,
            None,
            None,
        )
        .await;
    }
    let mut root = root;
    if let Some(meta) = root_meta {
        root = layer_root_meta(tree, root, meta).await?;
    }
    Ok(root)
}

/// Like [`index_dir_with_history_and_meta`], but starts from a hashtree root
/// that already represents the user-visible directory. This is used by
/// virtual mounts / file-provider adapters where bytes live in hashtree rather
/// than a normal folder on disk.
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
    root: Cid,
    previous_root: Option<&Cid>,
    tombstone_base_root: Option<&Cid>,
    now_unix_seconds: i64,
    root_meta: Option<&DriveRootMeta>,
    tombstone_paths: Option<&BTreeSet<String>>,
) -> Result<Cid, IndexError> {
    layer_history_and_meta_on_root_with_tombstone_base_paths_and_replacements(
        tree,
        root,
        previous_root,
        tombstone_base_root,
        now_unix_seconds,
        root_meta,
        tombstone_paths,
        None,
    )
    .await
}

/// Role-aware variant for providers that can distinguish a path-kind
/// replacement from an ordinary deletion. `Some(empty)` deliberately writes
/// an empty metadata document, preventing legacy recovery heuristics.
pub async fn layer_history_and_meta_on_root_with_tombstone_base_paths_and_replacements<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    previous_root: Option<&Cid>,
    tombstone_base_root: Option<&Cid>,
    now_unix_seconds: i64,
    root_meta: Option<&DriveRootMeta>,
    tombstone_paths: Option<&BTreeSet<String>>,
    kind_replacement_paths: Option<&BTreeSet<String>>,
) -> Result<Cid, IndexError> {
    let phase = std::time::Instant::now();
    root = filter_ignored_entries_from_root(tree, &root).await?;
    tracing::debug!(
        elapsed_ms = phase.elapsed().as_millis(),
        "history layer filtered ignored entries"
    );
    if previous_root.is_some() || tombstone_base_root.is_some() {
        let phase = std::time::Instant::now();
        let VisibleEntries {
            files: current_files,
            dirs: current_dirs,
        } = visible_entries(tree, &root).await?;
        let current_paths = current_files.keys().cloned().collect::<BTreeSet<_>>();
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            "history layer walked current root"
        );
        let mut derived_tombstone_paths = None;
        let mut derived_kind_replacement_paths = None;
        let mut walked = None;
        if (tombstone_paths.is_none() || kind_replacement_paths.is_none())
            && let Some(base_root) = tombstone_base_root.or(previous_root)
        {
            // The history attach below needs this root's files and
            // tombstones too, so one walk serves both.
            let (base_files, base_tombstones, base_index) =
                walk_app_key_tree_indexed(tree, base_root).await?;
            let base_file_paths = || base_files.iter().map(|file| &file.path);
            let base_dir_paths = || base_index.dirs.iter().map(|(path, _)| path);
            let replacements = base_dir_paths()
                .filter(|path| current_files.contains_key(*path))
                .chain(base_file_paths().filter(|path| current_dirs.contains_key(*path)))
                .cloned()
                .collect::<BTreeSet<_>>();
            let mut deletions = base_file_paths()
                .filter(|path| !current_files.contains_key(*path))
                .chain(base_dir_paths().filter(|path| !current_dirs.contains_key(*path)))
                .cloned()
                .collect::<BTreeSet<_>>();
            deletions.retain(|path| {
                !replacements.iter().any(|replacement| {
                    path != replacement && path_is_at_or_below(path, replacement)
                })
            });
            derived_tombstone_paths = Some(deletions);
            derived_kind_replacement_paths = Some(replacements);
            walked = Some(WalkedRoot {
                root: base_root.clone(),
                files: base_files,
                tombstones: base_tombstones,
            });
        }
        let tombstone_paths = tombstone_paths.or(derived_tombstone_paths.as_ref());
        let kind_replacement_paths =
            kind_replacement_paths.or(derived_kind_replacement_paths.as_ref());
        let phase = std::time::Instant::now();
        root = attach_history_for_current_paths(
            tree,
            root,
            previous_root,
            tombstone_base_root,
            now_unix_seconds,
            &current_paths,
            tombstone_paths,
            kind_replacement_paths,
            walked,
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
    pub kind_replacement_paths: BTreeSet<String>,
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
    projection_root: Option<&Cid>,
    tombstone_paths: Option<&BTreeSet<String>>,
) -> Result<VisibleImportDelta, IndexError> {
    let original_base_root = base_root.clone();
    let edited_root = filter_ignored_entries_from_root(tree, edited_root).await?;
    let base_root = filter_ignored_entries_from_root(tree, base_root).await?;

    let VisibleEntries {
        files: edited_files,
        dirs: edited_dirs,
    } = visible_entries(tree, &edited_root).await?;
    let VisibleEntries {
        files: base_files,
        dirs: base_dirs,
    } = visible_entries(tree, &base_root).await?;
    let mut projection_files = BTreeMap::new();
    if let Some(projection_root) = projection_root {
        if projection_root == &original_base_root {
            projection_files = base_files.clone();
        } else {
            let projection_root = filter_ignored_entries_from_root(tree, projection_root).await?;
            projection_files = visible_entries(tree, &projection_root).await?.files;
        }
    }

    let previous_visible_root = match previous_root {
        Some(previous_root) => filter_ignored_entries_from_root(tree, previous_root).await?,
        None => tree.put_directory(Vec::new()).await?,
    };
    let VisibleEntries {
        files: previous_files,
        dirs: previous_dirs,
    } = visible_entries(tree, &previous_visible_root).await?;

    // The whole local contribution is known up front, so assemble it in
    // memory and store each directory once instead of editing a stored tree
    // path by path.
    let mut delta = DirBuilder::default();
    for (path, previous) in &previous_dirs {
        if !base_dirs.contains_key(path)
            && (edited_dirs.contains_key(path) || tombstone_path_selected(tombstone_paths, path))
        {
            continue;
        }
        set_delta_dir(&mut delta, path, previous);
    }
    for (path, previous) in &previous_files {
        if base_files
            .get(path)
            .is_some_and(|base| visible_entry_matches(base, previous))
            || (!base_files.contains_key(path)
                && !edited_files.contains_key(path)
                && !tombstone_path_selected(tombstone_paths, path))
        {
            set_delta_file(&mut delta, path, previous)?;
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
                if projection_files
                    .get(&path)
                    .is_some_and(|projection| visible_entry_matches(projection, entry))
                    && !previous_files
                        .get(&path)
                        .is_some_and(|previous| visible_entry_matches(previous, entry))
                {
                    continue;
                }
                set_delta_file(&mut delta, &path, entry)?;
            }
            None if tombstone_path_allowed(tombstone_paths, &path) => {
                remove_delta_path(&mut delta, &path)?;
                deleted_paths.insert(path);
            }
            None => {}
        }
    }

    let changed_dirs = base_dirs
        .keys()
        .chain(edited_dirs.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    for path in changed_dirs {
        match (base_dirs.contains_key(&path), edited_dirs.get(&path)) {
            (true, Some(edited))
                if base_dirs
                    .get(&path)
                    .is_some_and(|base| visible_entry_matches(base, edited)) =>
            {
                // Do not re-author an unchanged remote directory locally. Its
                // children are merged independently, and copying only the
                // directory shell creates an empty duplicate conflict root.
            }
            (_, Some(edited)) => {
                set_delta_dir(&mut delta, &path, edited);
            }
            (true, None) if !edited_files.contains_key(&path) => {
                remove_delta_path(&mut delta, &path)?;
                if tombstone_path_allowed(tombstone_paths, &path) {
                    deleted_paths.insert(path);
                }
            }
            _ => {}
        }
    }

    // A kind replacement needs one exact-path causal barrier, not fresh
    // markers for every former descendant. The old subtree remains available
    // to conflict projection, while carried descendant markers continue to
    // hide children that had already been deleted before the replacement.
    let kind_replacement_paths = base_dirs
        .keys()
        .filter(|path| edited_files.contains_key(*path))
        .chain(
            base_files
                .keys()
                .filter(|path| edited_dirs.contains_key(*path)),
        )
        .filter(|path| tombstone_path_allowed(tombstone_paths, path))
        .cloned()
        .collect::<BTreeSet<_>>();
    deleted_paths.retain(|path| {
        !kind_replacement_paths.iter().any(|replacement| {
            path != replacement
                && path
                    .strip_prefix(replacement)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
    });
    deleted_paths.extend(kind_replacement_paths.iter().cloned());

    Ok(VisibleImportDelta {
        root: delta.put(tree).await?,
        tombstone_paths: deleted_paths,
        kind_replacement_paths,
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
        let entries = tree.list_directory_required(dir).await?;
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
    kind_replacement_paths: Option<&BTreeSet<String>>,
    mut walked: Option<WalkedRoot>,
) -> Result<Cid, IndexError> {
    let mut tombstones: BTreeMap<String, i64> = BTreeMap::new();
    if let Some(paths) = tombstone_paths {
        for path in paths {
            tombstones.insert(path.clone(), now_unix_seconds);
        }
    }

    // Files that were in the root being edited but are no longer visible get a
    // fresh tombstone stamped at import time. When that root is also the
    // previous root, the walk below stamps the same paths with the same time.
    if let Some(base) = tombstone_base_root.filter(|base| previous_root != Some(*base)) {
        let phase = std::time::Instant::now();
        let (base_files, _base_tombstones) = files_and_tombstones(tree, base, &mut walked).await?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            file_count = base_files.len(),
            "history attach walked tombstone base"
        );
        for f in base_files {
            if !current_paths.contains(&f.path)
                && tombstone_path_allowed(tombstone_paths, &f.path)
                && !path_is_below_visible_selected_barrier(&f.path, current_paths, tombstone_paths)
            {
                tombstones.insert(f.path, now_unix_seconds);
            }
        }
    }

    if let Some(prev) = previous_root {
        let phase = std::time::Instant::now();
        let (prev_files, prev_tombstones) = files_and_tombstones(tree, prev, &mut walked).await?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            file_count = prev_files.len(),
            tombstone_count = prev_tombstones.len(),
            "history attach walked previous root"
        );
        for f in prev_files {
            if !current_paths.contains(&f.path)
                && tombstone_path_allowed(tombstone_paths, &f.path)
                && !path_is_below_visible_selected_barrier(&f.path, current_paths, tombstone_paths)
            {
                tombstones.entry(f.path).or_insert(now_unix_seconds);
            }
        }
        // Retain previous barriers even after a visible recreation. Merge lets
        // this root's own entry win, while the marker still prevents older
        // roots from resurrecting through a later kind conflict.
        for t in prev_tombstones {
            tombstones.entry(t.path).or_insert(t.tombstoned_at);
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

    if let Some(replacement_paths) = kind_replacement_paths {
        let mut replacements = match previous_root {
            Some(previous_root) => read_path_kind_replacements(tree, previous_root)
                .await?
                .unwrap_or_default(),
            None => BTreeMap::new(),
        };
        if let Some(paths) = tombstone_paths {
            for path in paths {
                if replacement_paths.contains(path) {
                    continue;
                }
                replacements.retain(|active, _| !path_is_at_or_below(active, path));
            }
        }
        for path in replacement_paths {
            validate_path_kind_replacement(path, now_unix_seconds)?;
            replacements.insert(path.clone(), now_unix_seconds);
        }
        root = layer_path_kind_replacements(tree, root, &replacements).await?;
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

/// Files and tombstones of a root the caller has already walked.
struct WalkedRoot {
    root: Cid,
    files: Vec<AppKeyFileEntry>,
    tombstones: Vec<AppKeyTombstone>,
}

/// Files and tombstones of `root`, taken from `walked` when it holds that root.
async fn files_and_tombstones<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    walked: &mut Option<WalkedRoot>,
) -> Result<(Vec<AppKeyFileEntry>, Vec<AppKeyTombstone>), IndexError> {
    if let Some(known) = walked.take_if(|known| &known.root == root) {
        return Ok((known.files, known.tombstones));
    }
    walk_app_key_tree(tree, root)
        .await
        .map_err(IndexError::Tree)
}

fn path_is_at_or_below(path: &str, ancestor: &str) -> bool {
    path == ancestor
        || path
            .strip_prefix(ancestor)
            .is_some_and(|suffix| suffix.starts_with('/'))
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

fn tombstone_path_selected(tombstone_paths: Option<&BTreeSet<String>>, path: &str) -> bool {
    tombstone_paths.is_some_and(|paths| tombstone_path_allowed(Some(paths), path))
}

fn path_is_below_visible_selected_barrier(
    path: &str,
    current_paths: &BTreeSet<String>,
    tombstone_paths: Option<&BTreeSet<String>>,
) -> bool {
    tombstone_paths.is_some_and(|paths| {
        paths.iter().any(|barrier| {
            current_paths.contains(barrier)
                && path != barrier
                && path
                    .strip_prefix(barrier)
                    .is_some_and(|suffix| suffix.starts_with('/'))
        })
    })
}

/// User-visible files and directories of one root, keyed by path.
#[derive(Default)]
struct VisibleEntries {
    files: BTreeMap<String, TreeEntry>,
    dirs: BTreeMap<String, TreeEntry>,
}

async fn visible_entries<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
) -> Result<VisibleEntries, IndexError> {
    let mut entries = VisibleEntries::default();
    collect_visible_entries(tree, root, "", &mut entries).await?;
    Ok(entries)
}

fn collect_visible_entries<'a, S: Store>(
    tree: &'a HashTree<S>,
    root: &'a Cid,
    prefix: &'a str,
    out: &'a mut VisibleEntries,
) -> futures::future::BoxFuture<'a, Result<(), IndexError>> {
    Box::pin(async move {
        for entry in tree.list_directory(root).await? {
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
                out.dirs.insert(path.clone(), entry);
                collect_visible_entries(tree, &cid, &path, out).await?;
            } else {
                out.files.insert(path, entry);
            }
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

/// The directory at `segments`, created along with any missing ancestors. A
/// file holding one of those names gives way to the directory.
fn delta_dir<'d>(mut dir: &'d mut DirBuilder, segments: &[&str]) -> &'d mut DirBuilder {
    for segment in segments {
        dir.leaves.remove(*segment);
        dir = dir.dirs.entry((*segment).to_string()).or_default();
    }
    dir
}

/// Ensure the directory at `path` exists, adopting `entry`'s link metadata
/// when it carries any.
fn set_delta_dir(root: &mut DirBuilder, path: &str, entry: &TreeEntry) {
    let segments = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if segments.is_empty() {
        return;
    }
    let dir = delta_dir(root, &segments);
    if entry.meta.is_some() {
        dir.meta.clone_from(&entry.meta);
    }
}

/// Link `entry` at `path`, replacing whatever held that name.
fn set_delta_file(root: &mut DirBuilder, path: &str, entry: &TreeEntry) -> Result<(), IndexError> {
    let (parent, name) = split_visible_path(path)?;
    let dir = delta_dir(root, &parent);
    dir.dirs.remove(name);
    dir.leaves
        .insert(name.to_string(), LeafLink::from_entry(entry));
    if !parent.is_empty() {
        // Published roots have always re-linked a directory without metadata
        // whenever a file is written into it; keep that so roots stay stable.
        dir.meta = None;
    }
    Ok(())
}

/// Remove `path` if present, then drop every ancestor it leaves empty.
fn remove_delta_path(root: &mut DirBuilder, path: &str) -> Result<(), IndexError> {
    let (parent, name) = split_visible_path(path)?;
    remove_delta_entry(root, &parent, name);
    Ok(())
}

fn remove_delta_entry(dir: &mut DirBuilder, parent: &[&str], name: &str) -> bool {
    let Some((segment, rest)) = parent.split_first() else {
        let removed_dir = dir.dirs.remove(name).is_some();
        let removed_leaf = dir.leaves.remove(name).is_some();
        return removed_dir || removed_leaf;
    };
    let Some(child) = dir.dirs.get_mut(*segment) else {
        return false;
    };
    if !remove_delta_entry(child, rest, name) {
        return false;
    }
    if child.is_empty() {
        dir.dirs.remove(*segment);
    }
    true
}

pub(crate) async fn remove_visible_path_if_present<S: Store>(
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
    let mut root = tree
        .remove_entry(root, &parent, name)
        .await
        .map_err(IndexError::from)?;
    for depth in (1..=parent.len()).rev() {
        let dir_path = &parent[..depth];
        let dir_cid = match resolve_dir(tree, &root, dir_path).await {
            Ok(dir_cid) => dir_cid,
            Err(IndexError::Tree(HashTreeError::PathNotFound(_))) => continue,
            Err(error) => return Err(error),
        };
        if !tree.list_directory(&dir_cid).await?.is_empty() {
            continue;
        }
        root = tree
            .remove_entry(&root, &parent[..depth - 1], parent[depth - 1])
            .await
            .map_err(IndexError::from)?;
    }
    Ok(root)
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

include!("indexer/metadata.rs");
#[cfg(test)]
mod history_walk_tests;
#[cfg(test)]
mod mount_import_tests;
#[cfg(test)]
mod tests;
