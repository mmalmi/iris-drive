//! Build the merged signed drive view exposed by virtual provider surfaces.

use std::collections::{BTreeMap, BTreeSet};

use hashtree_core::{
    Cid, CidParseError, DirEntry, HashTree, HashTreeError, LinkType, Store, from_hex, to_hex,
};
use thiserror::Error;

use crate::PRIMARY_DRIVE_ID;
use crate::config::{AppConfig, AppKeyRootRef, Drive};
use crate::conflict::conflict_filename;
use crate::indexer::{IndexError, read_root_meta, should_ignore_name};
use crate::merge::{
    AppKeySnapshot, MODIFIED_AT_META_KEY, MergedConflictFile, MergedConflictKind, MergedEntry,
    MergedView, merge_drives, walk_app_key_tree,
};
use crate::profile::ProfileState;
use crate::provider::{
    provider_collision_family_path, provider_file_probable_os_placeholder_family,
};

#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("config has no account; run `idrive init` first")]
    NoAccount,
    #[error("primary drive missing from config (expected drive_id={PRIMARY_DRIVE_ID})")]
    PrimaryDriveMissing,
    #[error("invalid root cid for AppKey {app_key_pubkey}: {root_cid}: {source}")]
    RootCid {
        app_key_pubkey: String,
        root_cid: String,
        source: CidParseError,
    },
    #[error("tree: {0}")]
    Tree(#[from] HashTreeError),
    #[error("index: {0}")]
    Index(#[from] IndexError),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrimaryMergedView {
    pub view: MergedView,
    pub authorized_app_keys: usize,
    pub app_key_roots_present: usize,
}

impl PrimaryMergedView {
    #[must_use]
    pub fn file_count(&self) -> usize {
        self.view.files.len()
    }

    #[must_use]
    pub fn top_level_entries(&self) -> usize {
        self.view
            .files
            .iter()
            .filter_map(|entry| entry.path.split('/').next())
            .filter(|segment| !segment.is_empty())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrimaryMergedRoot {
    pub root_cid: Cid,
    pub file_count: usize,
    pub top_level_entries: usize,
}

/// Build the merged view for the primary drive from locally available signed
/// roots.
pub async fn primary_merged_view<S: Store>(
    tree: &HashTree<S>,
    config: &AppConfig,
) -> Result<PrimaryMergedView, ProjectionError> {
    let account = config.profile.as_ref().ok_or(ProjectionError::NoAccount)?;
    let drive = config
        .drive(PRIMARY_DRIVE_ID)
        .ok_or(ProjectionError::PrimaryDriveMissing)?;
    let authorized = authorized_app_key_pubkeys(account);
    let merge_app_keys = merge_app_key_pubkeys(account, drive);

    let mut snapshots_data = Vec::new();
    for app_key_pubkey in &merge_app_keys {
        let Some(root) = drive.app_key_roots.get(app_key_pubkey) else {
            continue;
        };
        let Some(root) = merge_root_for_device(tree, app_key_pubkey, root, &merge_app_keys).await?
        else {
            continue;
        };
        let cid = Cid::parse(&root.root_cid).map_err(|source| ProjectionError::RootCid {
            app_key_pubkey: app_key_pubkey.clone(),
            root_cid: root.root_cid.clone(),
            source,
        })?;
        let (files, tombstones) = walk_app_key_tree(tree, &cid).await?;
        snapshots_data.push((app_key_pubkey.clone(), root, files, tombstones));
    }

    let merge_app_key_refs: Vec<&str> = merge_app_keys.iter().map(String::as_str).collect();
    let snapshots: Vec<AppKeySnapshot<'_>> = snapshots_data
        .iter()
        .map(|(app_key_pubkey, root, files, tombstones)| AppKeySnapshot {
            app_key_pubkey: app_key_pubkey.as_str(),
            root,
            files: files.clone(),
            tombstones: tombstones.clone(),
        })
        .collect();
    let mut view = merge_drives(&merge_app_key_refs, &snapshots);
    add_visible_conflict_entries(&mut view)?;
    suppress_probable_os_placeholder_entries(&mut view);
    add_primary_share_shortcut_entries(&mut view, config);
    Ok(PrimaryMergedView {
        view,
        authorized_app_keys: authorized.len(),
        app_key_roots_present: snapshots.len(),
    })
}

fn add_visible_conflict_entries(view: &mut MergedView) -> Result<(), ProjectionError> {
    let winners_by_path = view
        .files
        .iter()
        .map(|entry| (entry.path.clone(), entry.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut occupied_paths = winners_by_path.keys().cloned().collect::<BTreeSet<_>>();
    let mut conflict_entries = Vec::new();

    for conflict in &view.conflict_details {
        match conflict.kind {
            MergedConflictKind::WriteWrite => {
                let Some(winner) = winners_by_path.get(&conflict.path) else {
                    continue;
                };

                for file in &conflict.files {
                    if conflict_file_matches_entry(file, winner) {
                        continue;
                    }
                    conflict_entries.push(visible_conflict_entry(
                        &conflict.path,
                        file,
                        winner.published_at,
                        &mut occupied_paths,
                    )?);
                }
            }
            MergedConflictKind::WriteDelete => {
                if winners_by_path.contains_key(&conflict.path) {
                    continue;
                }
                let published_at = conflict.tombstone.as_ref().map_or(0, |t| t.tombstoned_at);
                for file in &conflict.files {
                    conflict_entries.push(visible_conflict_entry(
                        &conflict.path,
                        file,
                        published_at,
                        &mut occupied_paths,
                    )?);
                }
            }
        }
    }

    if !conflict_entries.is_empty() {
        view.files.extend(conflict_entries);
        view.files.sort_by(|a, b| a.path.cmp(&b.path));
    }

    Ok(())
}

fn suppress_probable_os_placeholder_entries(view: &mut MergedView) {
    let non_empty_families = view
        .files
        .iter()
        .filter(|entry| entry.size > 0)
        .map(|entry| {
            (
                entry.source_app_key_pubkey.clone(),
                provider_collision_family_path(&entry.path).0,
            )
        })
        .collect::<BTreeSet<_>>();

    view.files.retain(|entry| {
        let Some(family_path) =
            provider_file_probable_os_placeholder_family(&entry.path, entry.size)
        else {
            return true;
        };
        !non_empty_families.contains(&(entry.source_app_key_pubkey.clone(), family_path))
    });
}

fn visible_conflict_entry(
    original_path: &str,
    file: &MergedConflictFile,
    published_at: i64,
    occupied_paths: &mut BTreeSet<String>,
) -> Result<MergedEntry, ProjectionError> {
    let path = next_visible_conflict_path(original_path, &file.app_key_pubkey, occupied_paths);
    Ok(MergedEntry {
        path,
        source_path: Some(original_path.to_string()),
        hash: parse_conflict_hash(&file.content_cid_hash, original_path)?,
        size: file.size,
        whole_file_hash: parse_conflict_whole_file_hash(file, original_path)?,
        modified_at: file.modified_at,
        source_app_key_pubkey: file.app_key_pubkey.clone(),
        published_at,
    })
}

fn conflict_file_matches_entry(file: &MergedConflictFile, entry: &MergedEntry) -> bool {
    file.app_key_pubkey == entry.source_app_key_pubkey
        && file.content_cid_hash == to_hex(&entry.hash)
        && file.content_hash == entry_identity_hash_hex(entry)
        && file.size == entry.size
}

fn entry_identity_hash(entry: &MergedEntry) -> [u8; 32] {
    entry.whole_file_hash.unwrap_or(entry.hash)
}

fn entry_identity_hash_hex(entry: &MergedEntry) -> String {
    to_hex(&entry_identity_hash(entry))
}

fn next_visible_conflict_path(
    original_path: &str,
    app_key_pubkey: &str,
    occupied_paths: &mut BTreeSet<String>,
) -> String {
    for index in 1..=256 {
        let label = if index == 1 {
            app_key_pubkey.to_string()
        } else {
            format!("{app_key_pubkey} {index}")
        };
        let path = conflict_filename(original_path, &label);
        if occupied_paths.insert(path.clone()) {
            return path;
        }
    }

    conflict_filename(original_path, &format!("{app_key_pubkey} 257"))
}

fn parse_conflict_hash(hex: &str, path: &str) -> Result<[u8; 32], ProjectionError> {
    from_hex(hex).map_err(|error| {
        HashTreeError::Store(format!("invalid conflict hash for {path}: {error}")).into()
    })
}

fn parse_conflict_whole_file_hash(
    file: &MergedConflictFile,
    path: &str,
) -> Result<Option<[u8; 32]>, ProjectionError> {
    if file.content_hash == file.content_cid_hash {
        return Ok(None);
    }
    parse_conflict_hash(&file.content_hash, path).map(Some)
}

fn merged_entry_source_path(entry: &MergedEntry) -> &str {
    entry.source_path.as_deref().unwrap_or(&entry.path)
}

/// Build a user-visible hashtree root for the merged primary drive.
///
/// This is the root a virtual mount/FileProvider surface should expose. It is
/// intentionally free of Iris Drive's `.hashtree/` metadata; when the mounted
/// view is edited, [`crate::daemon::Daemon::import_visible_root`] layers the
/// metadata back onto the device's publishable root.
pub async fn primary_merged_root<S: Store>(
    tree: &HashTree<S>,
    config: &AppConfig,
) -> Result<PrimaryMergedRoot, ProjectionError> {
    let merged = primary_merged_view(tree, config).await?;
    primary_merged_root_from_view(tree, config, &merged).await
}

/// Build the visible provider root from an already-computed merged view.
///
/// FileProvider callers often need both the entry timestamp index from
/// [`primary_merged_view`] and the materialized provider root. Passing the view
/// through this helper keeps that path to one projection walk.
pub async fn primary_merged_root_from_view<S: Store>(
    tree: &HashTree<S>,
    config: &AppConfig,
    merged: &PrimaryMergedView,
) -> Result<PrimaryMergedRoot, ProjectionError> {
    let account = config.profile.as_ref().ok_or(ProjectionError::NoAccount)?;
    let drive = config
        .drive(PRIMARY_DRIVE_ID)
        .ok_or(ProjectionError::PrimaryDriveMissing)?;
    let merge_app_keys = merge_app_key_pubkeys(account, drive);
    let source_roots = merge_source_roots(tree, drive, &merge_app_keys).await?;
    let mut root = tree.put_directory(Vec::new()).await?;

    let target_dirs = merged_user_directory_paths(
        tree,
        drive,
        &merge_app_keys,
        &merged.view.suppressed_by_tombstone,
    )
    .await?;
    let mut target_dirs = target_dirs;
    add_primary_share_shortcut_directory_paths(&mut target_dirs, &merged.view.files, config);
    for dir in &target_dirs {
        root = ensure_visible_dir(tree, root, dir).await?;
    }

    for entry in &merged.view.files {
        root = ensure_visible_parent_dirs(tree, root, &entry.path).await?;
        let source = source_entry_for_merged_entry(tree, &source_roots, entry).await?;
        let cid = Cid {
            hash: source.hash,
            key: source.key,
        };
        let (parent, name) = split_visible_path(&entry.path)?;
        root =
            set_visible_entry_with_meta(tree, &root, &parent, name, &cid, &source, entry).await?;
    }

    Ok(PrimaryMergedRoot {
        root_cid: root,
        file_count: merged.file_count(),
        top_level_entries: merged.top_level_entries(),
    })
}

fn add_primary_share_shortcut_entries(view: &mut MergedView, config: &AppConfig) {
    if config.share_shortcuts.is_empty() {
        return;
    }

    let source_entries = view.files.clone();
    let mut occupied_paths = view
        .files
        .iter()
        .map(|entry| entry.path.clone())
        .collect::<BTreeSet<_>>();
    let mut shortcut_entries = Vec::new();

    for shortcut in &config.share_shortcuts {
        let Some(folder) = config.shared_folder(shortcut.share_id) else {
            continue;
        };
        let source_root = share_shortcut_primary_source_path(&folder.source_path, shortcut);
        for entry in &source_entries {
            let Some(suffix) = suffix_under_path(&entry.path, &source_root) else {
                continue;
            };
            let path = projected_share_shortcut_path(&shortcut.path, suffix);
            if !occupied_paths.insert(path.clone()) {
                continue;
            }

            let mut shortcut_entry = entry.clone();
            shortcut_entry.path = path;
            shortcut_entry.source_path = Some(merged_entry_source_path(entry).to_string());
            shortcut_entries.push(shortcut_entry);
        }
    }

    if !shortcut_entries.is_empty() {
        view.files.extend(shortcut_entries);
        view.files.sort_by(|left, right| left.path.cmp(&right.path));
    }
}

fn add_primary_share_shortcut_directory_paths(
    target_dirs: &mut BTreeSet<String>,
    visible_files: &[MergedEntry],
    config: &AppConfig,
) {
    if config.share_shortcuts.is_empty() {
        return;
    }

    let source_dirs = target_dirs.clone();
    let file_paths = visible_files
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<BTreeSet<_>>();
    for shortcut in &config.share_shortcuts {
        let Some(folder) = config.shared_folder(shortcut.share_id) else {
            continue;
        };
        let source_root = share_shortcut_primary_source_path(&folder.source_path, shortcut);
        for dir in &source_dirs {
            let Some(suffix) = suffix_under_path(dir, &source_root) else {
                continue;
            };
            let path = projected_share_shortcut_path(&shortcut.path, suffix);
            if !file_paths.contains(path.as_str()) {
                target_dirs.insert(path);
            }
        }
    }
}

fn share_shortcut_primary_source_path(
    source_path: &str,
    shortcut: &crate::sharing::ShareShortcut,
) -> String {
    if shortcut.target_path.is_empty() {
        source_path.to_string()
    } else {
        format!("{source_path}/{}", shortcut.target_path)
    }
}

fn suffix_under_path<'a>(path: &'a str, root: &str) -> Option<&'a str> {
    if path == root {
        return Some("");
    }
    let rest = path.strip_prefix(root)?;
    rest.strip_prefix('/')
}

fn projected_share_shortcut_path(shortcut_path: &str, suffix: &str) -> String {
    if suffix.is_empty() {
        shortcut_path.to_string()
    } else {
        format!("{shortcut_path}/{suffix}")
    }
}

async fn merge_source_roots<S: Store>(
    tree: &HashTree<S>,
    drive: &Drive,
    merge_app_keys: &[String],
) -> Result<BTreeMap<String, AppKeyRootRef>, ProjectionError> {
    let mut source_roots = BTreeMap::new();
    for app_key_pubkey in merge_app_keys {
        let Some(root) = drive.app_key_roots.get(app_key_pubkey) else {
            continue;
        };
        let Some(root) = merge_root_for_device(tree, app_key_pubkey, root, merge_app_keys).await?
        else {
            continue;
        };
        source_roots.insert(app_key_pubkey.clone(), root);
    }
    Ok(source_roots)
}

fn split_visible_path(path: &str) -> Result<(Vec<&str>, &str), ProjectionError> {
    let mut segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let name = segments
        .pop()
        .ok_or_else(|| HashTreeError::PathNotFound(path.to_string()))?;
    Ok((segments, name))
}

async fn ensure_visible_parent_dirs<S: Store>(
    tree: &HashTree<S>,
    root: Cid,
    path: &str,
) -> Result<Cid, ProjectionError> {
    let (parent, _) = split_visible_path(path)?;
    let mut current_root = root;
    for depth in 1..=parent.len() {
        current_root = ensure_visible_dir_segments(tree, current_root, &parent[..depth]).await?;
    }
    Ok(current_root)
}

async fn ensure_visible_dir<S: Store>(
    tree: &HashTree<S>,
    root: Cid,
    path: &str,
) -> Result<Cid, ProjectionError> {
    let segments: Vec<&str> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let mut current_root = root;
    for depth in 1..=segments.len() {
        current_root = ensure_visible_dir_segments(tree, current_root, &segments[..depth]).await?;
    }
    Ok(current_root)
}

async fn ensure_visible_dir_segments<S: Store>(
    tree: &HashTree<S>,
    root: Cid,
    segments: &[&str],
) -> Result<Cid, ProjectionError> {
    if segments.is_empty() {
        return Ok(root);
    }
    let path = segments.join("/");
    if let Some(existing) = tree.resolve(&root, &path).await? {
        if tree.is_dir(&existing).await? {
            return Ok(root);
        }
        return Err(HashTreeError::PathNotFound(path).into());
    }

    let dir = tree.put_directory(Vec::new()).await?;
    let Some((name, parent)) = segments.split_last() else {
        return Ok(root);
    };
    tree.set_entry(&root, parent, name, &dir, 0, LinkType::Dir)
        .await
        .map_err(Into::into)
}

async fn source_entry_for_merged_entry<S: Store>(
    tree: &HashTree<S>,
    source_roots: &BTreeMap<String, AppKeyRootRef>,
    entry: &MergedEntry,
) -> Result<hashtree_core::TreeEntry, ProjectionError> {
    let root = source_roots
        .get(&entry.source_app_key_pubkey)
        .ok_or_else(|| HashTreeError::PathNotFound(entry.source_app_key_pubkey.clone()))?;
    let root = Cid::parse(&root.root_cid).map_err(|source| ProjectionError::RootCid {
        app_key_pubkey: entry.source_app_key_pubkey.clone(),
        root_cid: root.root_cid.clone(),
        source,
    })?;
    tree_entry_at_path(tree, &root, merged_entry_source_path(entry)).await
}

async fn tree_entry_at_path<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    path: &str,
) -> Result<hashtree_core::TreeEntry, ProjectionError> {
    let (parent, name) = split_visible_path(path)?;
    let parent_cid = if parent.is_empty() {
        root.clone()
    } else {
        tree.resolve(root, &parent.join("/"))
            .await?
            .ok_or_else(|| HashTreeError::PathNotFound(parent.join("/")))?
    };
    tree.list_directory(&parent_cid)
        .await?
        .into_iter()
        .find(|entry| entry.name == name)
        .ok_or_else(|| HashTreeError::EntryNotFound(name.to_string()).into())
}

async fn set_visible_entry_with_meta<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    parent: &[&str],
    name: &str,
    cid: &Cid,
    source: &hashtree_core::TreeEntry,
    merged: &MergedEntry,
) -> Result<Cid, ProjectionError> {
    let parent_cid = if parent.is_empty() {
        root.clone()
    } else {
        tree.resolve(root, &parent.join("/"))
            .await?
            .ok_or_else(|| HashTreeError::PathNotFound(parent.join("/")))?
    };
    let mut entries = tree
        .list_directory(&parent_cid)
        .await?
        .into_iter()
        .filter(|entry| entry.name != name)
        .map(|entry| DirEntry {
            name: entry.name,
            hash: entry.hash,
            size: entry.size,
            key: entry.key,
            link_type: entry.link_type,
            meta: entry.meta,
        })
        .collect::<Vec<_>>();
    entries.push(DirEntry {
        name: name.to_string(),
        hash: cid.hash,
        size: source.size,
        key: cid.key,
        link_type: source.link_type,
        meta: visible_entry_meta(source, merged),
    });
    let new_parent_cid = tree.put_directory(entries).await?;
    if parent.is_empty() {
        return Ok(new_parent_cid);
    }

    let parent_of_parent = &parent[..parent.len() - 1];
    let dir_name = parent[parent.len() - 1];
    tree.set_entry(
        root,
        parent_of_parent,
        dir_name,
        &new_parent_cid,
        0,
        LinkType::Dir,
    )
    .await
    .map_err(Into::into)
}

fn visible_entry_meta(
    source: &hashtree_core::TreeEntry,
    merged: &MergedEntry,
) -> Option<std::collections::HashMap<String, serde_json::Value>> {
    let mut meta = source.meta.clone().unwrap_or_default();
    if !meta.contains_key(MODIFIED_AT_META_KEY)
        && let Some(modified_at) = merged.modified_at.filter(|value| *value > 0)
    {
        meta.insert(
            MODIFIED_AT_META_KEY.to_string(),
            serde_json::Value::Number(modified_at.into()),
        );
    }
    (!meta.is_empty()).then_some(meta)
}

async fn merged_user_directory_paths<S: Store>(
    tree: &HashTree<S>,
    drive: &crate::config::Drive,
    authorized_app_keys: &[String],
    suppressed_paths: &[String],
) -> Result<BTreeSet<String>, ProjectionError> {
    let mut dirs = BTreeSet::new();
    for app_key_pubkey in authorized_app_keys {
        let Some(root) = drive.app_key_roots.get(app_key_pubkey) else {
            continue;
        };
        let Some(root) =
            merge_root_for_device(tree, app_key_pubkey, root, authorized_app_keys).await?
        else {
            continue;
        };
        let cid = Cid::parse(&root.root_cid).map_err(|source| ProjectionError::RootCid {
            app_key_pubkey: app_key_pubkey.clone(),
            root_cid: root.root_cid.clone(),
            source,
        })?;
        collect_user_directory_paths(tree, &cid, "", &mut dirs).await?;
    }
    dirs.retain(|path| !path_is_suppressed_by_tombstone(path, suppressed_paths));
    Ok(dirs)
}

fn path_is_suppressed_by_tombstone(path: &str, suppressed_paths: &[String]) -> bool {
    suppressed_paths.iter().any(|suppressed| {
        path == suppressed
            || path
                .strip_prefix(suppressed)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

async fn merge_root_for_device<S: Store>(
    tree: &HashTree<S>,
    app_key_pubkey: &str,
    root: &AppKeyRootRef,
    active_app_keys: &[String],
) -> Result<Option<AppKeyRootRef>, ProjectionError> {
    let mut current = root.clone();
    for _ in 0..32 {
        if !current.local_only {
            return Ok(Some(current));
        }
        if should_use_local_only_root(app_key_pubkey, &current, active_app_keys) {
            return Ok(Some(current));
        }
        let Some(parent) = current
            .parents
            .iter()
            .rev()
            .find(|parent| parent.app_key_pubkey == app_key_pubkey)
        else {
            return Ok(None);
        };
        let parent_cid =
            Cid::parse(&parent.root_cid).map_err(|source| ProjectionError::RootCid {
                app_key_pubkey: app_key_pubkey.to_string(),
                root_cid: parent.root_cid.clone(),
                source,
            })?;
        let Some(meta) = read_root_meta(tree, &parent_cid).await? else {
            let mut parent_root = AppKeyRootRef::legacy(
                parent.root_cid.clone(),
                current.published_at,
                current.dck_generation,
            );
            parent_root.app_key_seq = parent.app_key_seq;
            return Ok(Some(parent_root));
        };
        current = AppKeyRootRef::from_meta(parent.root_cid.clone(), meta.created_at, &meta);
    }
    Ok(None)
}

fn should_use_local_only_root(
    app_key_pubkey: &str,
    root: &AppKeyRootRef,
    active_app_keys: &[String],
) -> bool {
    let observed_inactive_peer = root.observed.keys().any(|observed_app_key| {
        observed_app_key != app_key_pubkey
            && !active_app_keys
                .iter()
                .any(|active_app_key| active_app_key == observed_app_key)
    });
    let active_peer_present = active_app_keys
        .iter()
        .any(|active_app_key| active_app_key != app_key_pubkey);
    observed_inactive_peer && !active_peer_present
}

fn collect_user_directory_paths<'a, S: Store>(
    tree: &'a HashTree<S>,
    dir_cid: &'a Cid,
    prefix: &'a str,
    dirs: &'a mut BTreeSet<String>,
) -> futures::future::BoxFuture<'a, Result<(), HashTreeError>> {
    Box::pin(async move {
        let entries = tree.list_directory(dir_cid).await?;
        for entry in entries {
            if prefix.is_empty() && entry.name == crate::merge::META_DIR {
                continue;
            }
            if should_ignore_name(&entry.name) {
                continue;
            }
            if entry.link_type != LinkType::Dir {
                continue;
            }
            let path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{prefix}/{}", entry.name)
            };
            dirs.insert(path.clone());
            let child_cid = Cid {
                hash: entry.hash,
                key: entry.key,
            };
            collect_user_directory_paths(tree, &child_cid, &path, dirs).await?;
        }
        Ok(())
    })
}

fn authorized_app_key_pubkeys(state: &ProfileState) -> Vec<String> {
    state.active_root_writer_app_key_pubkeys()
}

#[must_use]
pub fn merge_app_key_pubkeys(account: &ProfileState, drive: &Drive) -> Vec<String> {
    let mut devices = authorized_app_key_pubkeys(account);
    for (app_key_pubkey, _) in drive.active_app_key_roots(Some(account)) {
        if !devices.contains(app_key_pubkey) {
            devices.push(app_key_pubkey.clone());
        }
    }
    devices
}

#[cfg(test)]
fn safe_relative_path(path: &str) -> Option<&str> {
    if path.is_empty() || path.contains('\\') {
        return None;
    }
    let mut depth = 0usize;
    for segment in path.split('/') {
        match segment {
            "" | "." => return None,
            ".." => {
                if depth == 0 {
                    return None;
                }
                depth -= 1;
            }
            _ => depth += 1,
        }
    }
    Some(path)
}

#[cfg(test)]
fn may_replace_destination(
    _remote_entry: Option<&crate::merge::AppKeyFileEntry>,
    local_entry: Option<&crate::merge::AppKeyFileEntry>,
    destination_was_imported: bool,
) -> bool {
    destination_was_imported || local_entry.is_none()
}

#[cfg(test)]
mod provider_perf_tests;
#[cfg(test)]
mod tests;
