//! Build the merged signed drive view exposed by virtual provider surfaces.

use std::collections::{BTreeMap, BTreeSet};

use hashtree_core::{
    Cid, CidParseError, DirEntry, HashTree, HashTreeError, LinkType, Store, from_hex, to_hex,
};
use thiserror::Error;

use crate::PRIMARY_DRIVE_ID;
use crate::config::{AppConfig, DeviceRootRef, Drive};
use crate::conflict::conflict_filename;
use crate::indexer::{IndexError, read_root_meta, should_ignore_name};
use crate::merge::{
    DeviceSnapshot, MODIFIED_AT_META_KEY, MergedConflictFile, MergedConflictKind, MergedEntry,
    MergedView, merge_drives, walk_device_tree,
};
use crate::profile::ProfileState;

#[derive(Debug, Error)]
pub enum ProjectionError {
    #[error("config has no account; run `idrive init` first")]
    NoAccount,
    #[error("primary drive missing from config (expected drive_id={PRIMARY_DRIVE_ID})")]
    PrimaryDriveMissing,
    #[error("invalid root cid for device {device_id}: {root_cid}: {source}")]
    RootCid {
        device_id: String,
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
    pub authorized_devices: usize,
    pub device_roots_present: usize,
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
    let authorized = authorized_device_pubkeys(account);
    let merge_devices = merge_device_pubkeys(account, drive);

    let mut snapshots_data = Vec::new();
    for device_pubkey in &merge_devices {
        let Some(root) = drive.device_roots.get(device_pubkey) else {
            continue;
        };
        let Some(root) = merge_root_for_device(tree, device_pubkey, root).await? else {
            continue;
        };
        let cid = Cid::parse(&root.root_cid).map_err(|source| ProjectionError::RootCid {
            device_id: device_pubkey.clone(),
            root_cid: root.root_cid.clone(),
            source,
        })?;
        let (files, tombstones) = walk_device_tree(tree, &cid).await?;
        snapshots_data.push((device_pubkey.clone(), root, files, tombstones));
    }

    let merge_device_refs: Vec<&str> = merge_devices.iter().map(String::as_str).collect();
    let snapshots: Vec<DeviceSnapshot<'_>> = snapshots_data
        .iter()
        .map(|(device_pubkey, root, files, tombstones)| DeviceSnapshot {
            device_pubkey: device_pubkey.as_str(),
            root,
            files: files.clone(),
            tombstones: tombstones.clone(),
        })
        .collect();
    let mut view = merge_drives(&merge_device_refs, &snapshots);
    add_visible_conflict_entries(&mut view)?;
    Ok(PrimaryMergedView {
        view,
        authorized_devices: authorized.len(),
        device_roots_present: snapshots.len(),
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

fn visible_conflict_entry(
    original_path: &str,
    file: &MergedConflictFile,
    published_at: i64,
    occupied_paths: &mut BTreeSet<String>,
) -> Result<MergedEntry, ProjectionError> {
    let path = next_visible_conflict_path(original_path, &file.device_id, occupied_paths);
    Ok(MergedEntry {
        path,
        source_path: Some(original_path.to_string()),
        hash: parse_conflict_hash(&file.content_cid_hash, original_path)?,
        size: file.size,
        whole_file_hash: parse_conflict_whole_file_hash(file, original_path)?,
        modified_at: file.modified_at,
        source_device: file.device_id.clone(),
        published_at,
    })
}

fn conflict_file_matches_entry(file: &MergedConflictFile, entry: &MergedEntry) -> bool {
    file.device_id == entry.source_device
        && file.content_cid_hash == to_hex(&entry.hash)
        && file.content_hash == entry_identity_hash_hex(entry)
        && file.size == entry.size
}

fn entry_identity_hash_hex(entry: &MergedEntry) -> String {
    entry
        .whole_file_hash
        .map_or_else(|| to_hex(&entry.hash), |hash| to_hex(&hash))
}

fn next_visible_conflict_path(
    original_path: &str,
    device_id: &str,
    occupied_paths: &mut BTreeSet<String>,
) -> String {
    for index in 1..=256 {
        let label = if index == 1 {
            device_id.to_string()
        } else {
            format!("{device_id} {index}")
        };
        let path = conflict_filename(original_path, &label);
        if occupied_paths.insert(path.clone()) {
            return path;
        }
    }

    conflict_filename(original_path, &format!("{device_id} 257"))
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
    let account = config.profile.as_ref().ok_or(ProjectionError::NoAccount)?;
    let drive = config
        .drive(PRIMARY_DRIVE_ID)
        .ok_or(ProjectionError::PrimaryDriveMissing)?;
    let merge_devices = merge_device_pubkeys(account, drive);
    let source_roots = merge_source_roots(tree, drive, &merge_devices).await?;
    let merged = primary_merged_view(tree, config).await?;
    let mut root = tree.put_directory(Vec::new()).await?;

    let target_dirs = merged_user_directory_paths(
        tree,
        drive,
        &merge_devices,
        &merged.view.suppressed_by_tombstone,
    )
    .await?;
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

async fn merge_source_roots<S: Store>(
    tree: &HashTree<S>,
    drive: &Drive,
    merge_devices: &[String],
) -> Result<BTreeMap<String, DeviceRootRef>, ProjectionError> {
    let mut source_roots = BTreeMap::new();
    for device_pubkey in merge_devices {
        let Some(root) = drive.device_roots.get(device_pubkey) else {
            continue;
        };
        let Some(root) = merge_root_for_device(tree, device_pubkey, root).await? else {
            continue;
        };
        source_roots.insert(device_pubkey.clone(), root);
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
    let (name, parent) = segments
        .split_last()
        .expect("non-empty segments checked above");
    tree.set_entry(&root, parent, name, &dir, 0, LinkType::Dir)
        .await
        .map_err(Into::into)
}

async fn source_entry_for_merged_entry<S: Store>(
    tree: &HashTree<S>,
    source_roots: &BTreeMap<String, DeviceRootRef>,
    entry: &MergedEntry,
) -> Result<hashtree_core::TreeEntry, ProjectionError> {
    let root = source_roots
        .get(&entry.source_device)
        .ok_or_else(|| HashTreeError::PathNotFound(entry.source_device.clone()))?;
    let root = Cid::parse(&root.root_cid).map_err(|source| ProjectionError::RootCid {
        device_id: entry.source_device.clone(),
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
    authorized_devices: &[String],
    suppressed_paths: &[String],
) -> Result<BTreeSet<String>, ProjectionError> {
    let mut dirs = BTreeSet::new();
    for device_pubkey in authorized_devices {
        let Some(root) = drive.device_roots.get(device_pubkey) else {
            continue;
        };
        let Some(root) = merge_root_for_device(tree, device_pubkey, root).await? else {
            continue;
        };
        let cid = Cid::parse(&root.root_cid).map_err(|source| ProjectionError::RootCid {
            device_id: device_pubkey.clone(),
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
    device_pubkey: &str,
    root: &DeviceRootRef,
) -> Result<Option<DeviceRootRef>, ProjectionError> {
    let mut current = root.clone();
    for _ in 0..32 {
        if !current.local_only {
            return Ok(Some(current));
        }
        let Some(parent) = current
            .parents
            .iter()
            .rev()
            .find(|parent| parent.device_id == device_pubkey)
        else {
            return Ok(None);
        };
        let parent_cid =
            Cid::parse(&parent.root_cid).map_err(|source| ProjectionError::RootCid {
                device_id: device_pubkey.to_string(),
                root_cid: parent.root_cid.clone(),
                source,
            })?;
        let Some(meta) = read_root_meta(tree, &parent_cid).await? else {
            let mut parent_root = DeviceRootRef::legacy(
                parent.root_cid.clone(),
                current.published_at,
                current.dck_generation,
            );
            parent_root.device_seq = parent.device_seq;
            return Ok(Some(parent_root));
        };
        current = DeviceRootRef::from_meta(parent.root_cid.clone(), meta.created_at, &meta);
    }
    Ok(None)
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

fn authorized_device_pubkeys(state: &ProfileState) -> Vec<String> {
    let mut app_actors: Vec<String> = state
        .app_keys
        .as_ref()
        .map(|snap| {
            snap.app_actors
                .iter()
                .map(|device| device.pubkey.clone())
                .collect()
        })
        .unwrap_or_default();
    if !app_actors.contains(&state.device_pubkey) {
        app_actors.push(state.device_pubkey.clone());
    }
    app_actors
}

#[must_use]
pub fn merge_device_pubkeys(account: &ProfileState, drive: &Drive) -> Vec<String> {
    let mut devices = authorized_device_pubkeys(account);
    for device_pubkey in drive.device_roots.keys() {
        if !devices.contains(device_pubkey) {
            devices.push(device_pubkey.clone());
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
    _remote_entry: Option<&crate::merge::DeviceFileEntry>,
    local_entry: Option<&crate::merge::DeviceFileEntry>,
    destination_was_imported: bool,
) -> bool {
    destination_was_imported || local_entry.is_none()
}

#[cfg(test)]
mod tests;
