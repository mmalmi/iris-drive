//! Apply the merged signed drive snapshot to a local working directory.
//!
//! Network sync gets remote root metadata and blocks into the local store. This
//! module performs the next step: make the user-visible folder match the merged
//! drive view, without overwriting unimported local edits.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hashtree_core::{
    Cid, CidParseError, DirEntry, HashTree, HashTreeError, LinkType, Store, from_hex, sha256,
    to_hex,
};
use hashtree_provider::{HashTreeProviderFs, ProviderError, ProviderFs};
use thiserror::Error;

use crate::PRIMARY_DRIVE_ID;
use crate::account::AccountState;
use crate::config::{AppConfig, DeviceRootRef};
use crate::conflict::conflict_filename;
use crate::indexer::{IndexError, read_root_meta};
use crate::merge::{
    DeviceFileEntry, DeviceSnapshot, MergedConflictFile, MergedConflictKind, MergedEntry,
    MergedView, merge_drives, walk_device_tree,
};

#[derive(Debug, Error)]
pub enum MaterializeError {
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
    #[error("provider: {0}")]
    Provider(#[from] ProviderError),
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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MaterializeReport {
    pub written: usize,
    pub updated: usize,
    pub deleted: usize,
    pub unchanged: usize,
    pub skipped: usize,
}

impl MaterializeReport {
    #[must_use]
    pub const fn changed(&self) -> bool {
        self.written > 0 || self.updated > 0 || self.deleted > 0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileSnapshot {
    size: u64,
    hash: [u8; 32],
}

/// Build the merged view for the primary drive from locally available signed
/// roots.
pub async fn primary_merged_view<S: Store>(
    tree: &HashTree<S>,
    config: &AppConfig,
) -> Result<PrimaryMergedView, MaterializeError> {
    let account = config.account.as_ref().ok_or(MaterializeError::NoAccount)?;
    let drive = config
        .drive(PRIMARY_DRIVE_ID)
        .ok_or(MaterializeError::PrimaryDriveMissing)?;
    let authorized = authorized_device_pubkeys(account);

    let mut snapshots_data = Vec::new();
    for device_pubkey in &authorized {
        let Some(root) = drive.device_roots.get(device_pubkey) else {
            continue;
        };
        let Some(root) = merge_root_for_device(tree, device_pubkey, root).await? else {
            continue;
        };
        let cid = Cid::parse(&root.root_cid).map_err(|source| MaterializeError::RootCid {
            device_id: device_pubkey.clone(),
            root_cid: root.root_cid.clone(),
            source,
        })?;
        let (files, tombstones) = walk_device_tree(tree, &cid).await?;
        snapshots_data.push((device_pubkey.clone(), root, files, tombstones));
    }

    let authorized_refs: Vec<&str> = authorized.iter().map(String::as_str).collect();
    let snapshots: Vec<DeviceSnapshot<'_>> = snapshots_data
        .iter()
        .map(|(device_pubkey, root, files, tombstones)| DeviceSnapshot {
            device_pubkey: device_pubkey.as_str(),
            root,
            files: files.clone(),
            tombstones: tombstones.clone(),
        })
        .collect();
    let mut view = merge_drives(&authorized_refs, &snapshots);
    add_visible_conflict_entries(&mut view)?;
    Ok(PrimaryMergedView {
        view,
        authorized_devices: authorized.len(),
        device_roots_present: snapshots.len(),
    })
}

fn add_visible_conflict_entries(view: &mut MergedView) -> Result<(), MaterializeError> {
    let winners_by_path = view
        .files
        .iter()
        .map(|entry| (entry.path.clone(), entry.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut occupied_paths = winners_by_path.keys().cloned().collect::<BTreeSet<_>>();
    let mut conflict_entries = Vec::new();

    for conflict in &view.conflict_details {
        if conflict.kind != MergedConflictKind::WriteWrite {
            continue;
        }
        let Some(winner) = winners_by_path.get(&conflict.path) else {
            continue;
        };

        for file in &conflict.files {
            if conflict_file_matches_entry(file, winner) {
                continue;
            }
            let path =
                next_visible_conflict_path(&conflict.path, &file.device_id, &mut occupied_paths);
            conflict_entries.push(MergedEntry {
                path,
                source_path: Some(conflict.path.clone()),
                hash: parse_conflict_hash(&file.content_cid_hash, &conflict.path)?,
                size: file.size,
                whole_file_hash: parse_conflict_whole_file_hash(file, &conflict.path)?,
                source_device: file.device_id.clone(),
                published_at: winner.published_at,
            });
        }
    }

    if !conflict_entries.is_empty() {
        view.files.extend(conflict_entries);
        view.files.sort_by(|a, b| a.path.cmp(&b.path));
    }

    Ok(())
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
        .map(|hash| to_hex(&hash))
        .unwrap_or_else(|| to_hex(&entry.hash))
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

fn parse_conflict_hash(hex: &str, path: &str) -> Result<[u8; 32], MaterializeError> {
    from_hex(hex).map_err(|error| {
        HashTreeError::Store(format!("invalid conflict hash for {path}: {error}")).into()
    })
}

fn parse_conflict_whole_file_hash(
    file: &MergedConflictFile,
    path: &str,
) -> Result<Option<[u8; 32]>, MaterializeError> {
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
) -> Result<PrimaryMergedRoot, MaterializeError> {
    let account = config.account.as_ref().ok_or(MaterializeError::NoAccount)?;
    let drive = config
        .drive(PRIMARY_DRIVE_ID)
        .ok_or(MaterializeError::PrimaryDriveMissing)?;
    let authorized = authorized_device_pubkeys(account);
    let merged = primary_merged_view(tree, config).await?;
    let mut root = tree.put_directory(Vec::new()).await?;

    let target_dirs = merged_user_directory_paths(tree, drive, &authorized).await?;
    for dir in &target_dirs {
        root = ensure_visible_dir(tree, root, dir).await?;
    }

    for entry in &merged.view.files {
        root = ensure_visible_parent_dirs(tree, root, &entry.path).await?;
        let source = source_entry_for_merged_entry(tree, drive, entry).await?;
        let cid = Cid {
            hash: source.hash,
            key: source.key,
        };
        let (parent, name) = split_visible_path(&entry.path)?;
        root = set_visible_entry_with_meta(tree, &root, &parent, name, &cid, &source).await?;
    }

    Ok(PrimaryMergedRoot {
        root_cid: root,
        file_count: merged.file_count(),
        top_level_entries: merged.top_level_entries(),
    })
}

/// Copy the merged primary-drive view into `target_dir`.
///
/// Existing files are overwritten only when they still match this device's
/// last imported root. If the target has diverged, materialization skips that
/// path so the caller can decide how to handle the local change.
pub async fn materialize_primary_drive<S>(
    tree: Arc<HashTree<S>>,
    config: &AppConfig,
    target_dir: &Path,
) -> Result<MaterializeReport, MaterializeError>
where
    S: Store + Send + Sync + 'static,
{
    std::fs::create_dir_all(target_dir)?;
    let account = config.account.as_ref().ok_or(MaterializeError::NoAccount)?;
    let drive = config
        .drive(PRIMARY_DRIVE_ID)
        .ok_or(MaterializeError::PrimaryDriveMissing)?;
    let merged = primary_merged_view(tree.as_ref(), config).await?;
    let target_by_path: BTreeMap<String, MergedEntry> = merged
        .view
        .files
        .iter()
        .map(|entry| (entry.path.clone(), entry.clone()))
        .collect();
    let local_entries =
        current_device_entries(tree.as_ref(), drive, &account.device_pubkey).await?;
    let target_dirs =
        merged_user_directory_paths(tree.as_ref(), drive, &authorized_device_pubkeys(account))
            .await?
            .into_iter()
            .filter(|path| !target_by_path.contains_key(path))
            .collect::<BTreeSet<_>>();
    let mut report = MaterializeReport::default();
    materialize_target_dirs(target_dir, &target_dirs, &local_entries, &mut report)?;
    materialize_target_files(
        tree,
        drive,
        target_dir,
        &target_by_path,
        &local_entries,
        &mut report,
    )
    .await?;
    delete_removed_local_files(target_dir, &target_by_path, &local_entries, &mut report)?;

    Ok(report)
}

fn materialize_target_dirs(
    target_dir: &Path,
    target_dirs: &BTreeSet<String>,
    local_entries: &BTreeMap<String, DeviceFileEntry>,
    report: &mut MaterializeReport,
) -> Result<(), MaterializeError> {
    for dir in target_dirs {
        let Some(relative) = safe_relative_path(dir) else {
            report.skipped += 1;
            continue;
        };
        let destination = target_dir.join(relative);
        if destination.is_dir() {
            report.unchanged += 1;
            continue;
        }
        if destination.exists() {
            if !may_replace_file_destination_with_directory(&destination, local_entries.get(dir))? {
                report.skipped += 1;
                continue;
            }
            std::fs::remove_file(&destination)?;
        }
        std::fs::create_dir_all(&destination)?;
        report.written += 1;
    }
    Ok(())
}

fn split_visible_path(path: &str) -> Result<(Vec<&str>, &str), MaterializeError> {
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
) -> Result<Cid, MaterializeError> {
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
) -> Result<Cid, MaterializeError> {
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
) -> Result<Cid, MaterializeError> {
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
    drive: &crate::config::Drive,
    entry: &MergedEntry,
) -> Result<hashtree_core::TreeEntry, MaterializeError> {
    let root = drive
        .device_roots
        .get(&entry.source_device)
        .ok_or_else(|| HashTreeError::PathNotFound(entry.source_device.clone()))?;
    let root = Cid::parse(&root.root_cid).map_err(|source| MaterializeError::RootCid {
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
) -> Result<hashtree_core::TreeEntry, MaterializeError> {
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
) -> Result<Cid, MaterializeError> {
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
        meta: source.meta.clone(),
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

async fn materialize_target_files<S>(
    tree: Arc<HashTree<S>>,
    drive: &crate::config::Drive,
    target_dir: &Path,
    target_by_path: &BTreeMap<String, MergedEntry>,
    local_entries: &BTreeMap<String, DeviceFileEntry>,
    report: &mut MaterializeReport,
) -> Result<(), MaterializeError>
where
    S: Store + Send + Sync + 'static,
{
    for entry in target_by_path.values() {
        let Some(relative) = safe_relative_path(&entry.path) else {
            report.skipped += 1;
            continue;
        };
        let destination = target_dir.join(relative);
        if destination.is_dir() {
            if !directory_matches_local_entries(target_dir, &entry.path, local_entries)? {
                report.skipped += 1;
                continue;
            }
            std::fs::remove_dir_all(&destination)?;
        }
        let destination_snapshot = file_snapshot(&destination)?;
        if destination_snapshot.is_some_and(|snapshot| snapshot_matches_entry(snapshot, entry)) {
            report.unchanged += 1;
            continue;
        }
        if !may_replace_destination(
            destination_snapshot,
            local_entries.get(&entry.path),
            destination.exists(),
        ) {
            report.skipped += 1;
            continue;
        }
        let Some(source_root) = drive.device_roots.get(&entry.source_device) else {
            report.skipped += 1;
            continue;
        };
        let bytes = read_file_from_root(
            tree.clone(),
            &source_root.root_cid,
            merged_entry_source_path(entry),
        )
        .await?;
        if destination_snapshot.is_some_and(|snapshot| snapshot.hash == sha256(&bytes)) {
            report.unchanged += 1;
            continue;
        }
        write_file(&destination, &bytes)?;
        if destination_snapshot.is_some() {
            report.updated += 1;
        } else {
            report.written += 1;
        }
    }
    Ok(())
}

fn delete_removed_local_files(
    target_dir: &Path,
    target_by_path: &BTreeMap<String, MergedEntry>,
    local_entries: &BTreeMap<String, DeviceFileEntry>,
    report: &mut MaterializeReport,
) -> Result<(), MaterializeError> {
    for (path, local_entry) in local_entries {
        if target_by_path.contains_key(path) {
            continue;
        }
        let Some(relative) = safe_relative_path(path) else {
            report.skipped += 1;
            continue;
        };
        let destination = target_dir.join(relative);
        let snapshot = file_snapshot(&destination)?;
        if snapshot.is_none() {
            report.unchanged += 1;
            continue;
        }
        if snapshot.is_some_and(|snapshot| snapshot_matches_device_entry(snapshot, local_entry)) {
            std::fs::remove_file(destination)?;
            report.deleted += 1;
        } else {
            report.skipped += 1;
        }
    }
    Ok(())
}

async fn merged_user_directory_paths<S: Store>(
    tree: &HashTree<S>,
    drive: &crate::config::Drive,
    authorized_devices: &[String],
) -> Result<BTreeSet<String>, MaterializeError> {
    let mut dirs = BTreeSet::new();
    for device_pubkey in authorized_devices {
        let Some(root) = drive.device_roots.get(device_pubkey) else {
            continue;
        };
        let Some(root) = merge_root_for_device(tree, device_pubkey, root).await? else {
            continue;
        };
        let cid = Cid::parse(&root.root_cid).map_err(|source| MaterializeError::RootCid {
            device_id: device_pubkey.clone(),
            root_cid: root.root_cid.clone(),
            source,
        })?;
        collect_user_directory_paths(tree, &cid, "", &mut dirs).await?;
    }
    Ok(dirs)
}

async fn merge_root_for_device<S: Store>(
    tree: &HashTree<S>,
    device_pubkey: &str,
    root: &DeviceRootRef,
) -> Result<Option<DeviceRootRef>, MaterializeError> {
    let mut current = root.clone();
    for _ in 0..32 {
        if !current.materialized_only {
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
            Cid::parse(&parent.root_cid).map_err(|source| MaterializeError::RootCid {
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

fn authorized_device_pubkeys(state: &AccountState) -> Vec<String> {
    let mut devices: Vec<String> = state
        .app_keys
        .as_ref()
        .map(|snap| {
            snap.devices
                .iter()
                .map(|device| device.pubkey.clone())
                .collect()
        })
        .unwrap_or_default();
    if !devices.contains(&state.device_pubkey) {
        devices.push(state.device_pubkey.clone());
    }
    devices
}

async fn current_device_entries<S: Store>(
    tree: &HashTree<S>,
    drive: &crate::config::Drive,
    device_pubkey: &str,
) -> Result<BTreeMap<String, DeviceFileEntry>, MaterializeError> {
    let Some(root) = drive.device_roots.get(device_pubkey) else {
        return Ok(BTreeMap::new());
    };
    let cid = Cid::parse(&root.root_cid).map_err(|source| MaterializeError::RootCid {
        device_id: device_pubkey.to_string(),
        root_cid: root.root_cid.clone(),
        source,
    })?;
    let (files, _tombstones) = walk_device_tree(tree, &cid).await?;
    Ok(files
        .into_iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect())
}

async fn read_file_from_root<S>(
    tree: Arc<HashTree<S>>,
    root_cid: &str,
    path: &str,
) -> Result<Vec<u8>, MaterializeError>
where
    S: Store + Send + Sync + 'static,
{
    let root = Cid::parse(root_cid).map_err(|source| MaterializeError::RootCid {
        device_id: String::new(),
        root_cid: root_cid.to_string(),
        source,
    })?;
    let provider = HashTreeProviderFs::open(tree, root).await?;
    let id = path.to_string();
    let item = provider.item(&id).await?;
    if item.size == 0 {
        return Ok(Vec::new());
    }
    Ok(provider.read(&id, 0, item.size).await?)
}

fn may_replace_destination(
    destination: Option<FileSnapshot>,
    local_entry: Option<&DeviceFileEntry>,
    destination_exists: bool,
) -> bool {
    let Some(destination) = destination else {
        return !destination_exists && local_entry.is_none();
    };
    local_entry.is_some_and(|entry| snapshot_matches_device_entry(destination, entry))
}

fn snapshot_matches_entry(snapshot: FileSnapshot, entry: &MergedEntry) -> bool {
    if snapshot.size != entry.size {
        return false;
    }
    entry
        .whole_file_hash
        .is_some_and(|hash| hash == snapshot.hash)
        || entry.hash == snapshot.hash
}

fn snapshot_matches_device_entry(snapshot: FileSnapshot, entry: &DeviceFileEntry) -> bool {
    if snapshot.size != entry.size {
        return false;
    }
    entry
        .whole_file_hash
        .is_some_and(|hash| hash == snapshot.hash)
        || entry.hash == snapshot.hash
}

fn file_snapshot(path: &Path) -> Result<Option<FileSnapshot>, MaterializeError> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if !metadata.is_file() {
        return Ok(Some(FileSnapshot {
            size: metadata.len(),
            hash: [0; 32],
        }));
    }
    let bytes = std::fs::read(path)?;
    Ok(Some(FileSnapshot {
        size: metadata.len(),
        hash: sha256(&bytes),
    }))
}

fn may_replace_file_destination_with_directory(
    destination: &Path,
    local_entry: Option<&DeviceFileEntry>,
) -> Result<bool, MaterializeError> {
    let Some(local_entry) = local_entry else {
        return Ok(false);
    };
    Ok(file_snapshot(destination)?
        .is_some_and(|snapshot| snapshot_matches_device_entry(snapshot, local_entry)))
}

fn directory_matches_local_entries(
    target_dir: &Path,
    path: &str,
    local_entries: &BTreeMap<String, DeviceFileEntry>,
) -> Result<bool, MaterializeError> {
    let Some(relative) = safe_relative_path(path) else {
        return Ok(false);
    };
    let directory = target_dir.join(relative);
    let actual_files = collect_disk_file_paths(target_dir, &directory)?;
    let prefix = format!("{path}/");
    let local_subtree = local_entries
        .iter()
        .filter(|(entry_path, _)| entry_path.starts_with(&prefix))
        .collect::<BTreeMap<_, _>>();
    if actual_files.len() != local_subtree.len() {
        return Ok(false);
    }
    for actual_path in actual_files {
        let Some(local_entry) = local_entries.get(&actual_path) else {
            return Ok(false);
        };
        let Some(relative) = safe_relative_path(&actual_path) else {
            return Ok(false);
        };
        let snapshot = file_snapshot(&target_dir.join(relative))?;
        if !snapshot.is_some_and(|snapshot| snapshot_matches_device_entry(snapshot, local_entry)) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn collect_disk_file_paths(root: &Path, dir: &Path) -> Result<BTreeSet<String>, MaterializeError> {
    let mut paths = BTreeSet::new();
    collect_disk_file_paths_inner(root, dir, &mut paths)?;
    Ok(paths)
}

fn collect_disk_file_paths_inner(
    root: &Path,
    dir: &Path,
    paths: &mut BTreeSet<String>,
) -> Result<(), MaterializeError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_disk_file_paths_inner(root, &path, paths)?;
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| std::io::Error::other(error.to_string()))?
                .iter()
                .map(|segment| segment.to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            paths.insert(relative);
        }
    }
    Ok(())
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<(), MaterializeError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

fn safe_relative_path(path: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    let mut saw_segment = false;
    for segment in path.split('/') {
        if !safe_path_segment(segment) {
            return None;
        }
        saw_segment = true;
        out.push(segment);
    }
    saw_segment.then_some(out)
}

fn safe_path_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.contains('\\')
        && !segment.contains('\0')
        && !is_windows_reserved_name(segment)
}

fn is_windows_reserved_name(segment: &str) -> bool {
    segment.contains(['<', '>', ':', '"', '|', '?', '*'])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::Account;
    use crate::app_keys::DeviceEntry;
    use crate::config::{AppConfig, DeviceRootRef, Drive};
    use crate::indexer::index_dir_with_history_and_meta;
    use crate::root_meta::{DriveRootMeta, RootParent};
    use hashtree_core::{HashTreeConfig, MemoryStore};
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn safe_relative_path_rejects_traversal() {
        assert!(safe_relative_path("notes/today.txt").is_some());
        assert!(safe_relative_path("../today.txt").is_none());
        assert!(safe_relative_path("notes/../../today.txt").is_none());
        assert!(safe_relative_path("notes\\today.txt").is_none());
        assert!(safe_relative_path("").is_none());
    }

    #[test]
    fn may_replace_destination_preserves_unimported_deletions() {
        let local_entry = DeviceFileEntry {
            path: "note.txt".to_string(),
            hash: [1; 32],
            size: 5,
            whole_file_hash: None,
        };

        assert!(may_replace_destination(None, None, false));
        assert!(!may_replace_destination(None, Some(&local_entry), false));
    }

    #[tokio::test]
    async fn primary_merged_root_builds_visible_mount_root_without_metadata() {
        let cfg_dir = tempdir().unwrap();
        let account = Account::create(cfg_dir.path(), Some("mount-test".into())).unwrap();
        let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

        let source = tempdir().unwrap();
        std::fs::create_dir(source.path().join("empty")).unwrap();
        std::fs::create_dir(source.path().join("docs")).unwrap();
        std::fs::write(source.path().join("docs").join("note.txt"), b"mounted").unwrap();
        let meta = DriveRootMeta {
            schema: DriveRootMeta::SCHEMA,
            drive_id: PRIMARY_DRIVE_ID.to_string(),
            device_id: account.state.device_pubkey.clone(),
            device_seq: 1,
            dck_generation: 1,
            materialized_only: false,
            parents: Vec::new(),
            observed: BTreeMap::new(),
            created_at: 1,
        };
        let source_root =
            index_dir_with_history_and_meta(&tree, source.path(), None, 1, Some(&meta))
                .await
                .unwrap();

        let mut config = AppConfig {
            account: Some(account.state.clone()),
            ..AppConfig::default()
        };
        let mut drive = Drive::primary(account.state.owner_pubkey.clone());
        drive.device_roots.insert(
            account.state.device_pubkey.clone(),
            DeviceRootRef::from_meta(source_root.to_string(), 1, &meta),
        );
        config.upsert_drive(drive);

        let merged = primary_merged_root(&tree, &config).await.unwrap();
        let top = tree.list_directory(&merged.root_cid).await.unwrap();
        let top_names = top
            .iter()
            .map(|entry| entry.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(top_names, vec!["docs", "empty"]);
        assert_eq!(merged.file_count, 1);
        assert_eq!(merged.top_level_entries, 1);

        let note = tree
            .resolve(&merged.root_cid, "docs/note.txt")
            .await
            .unwrap()
            .expect("note exists");
        let bytes = tree.get(&note, None).await.unwrap().unwrap();
        assert_eq!(bytes, b"mounted");
        let (files, _) = walk_device_tree(&tree, &merged.root_cid).await.unwrap();
        let note_entry = files
            .iter()
            .find(|entry| entry.path == "docs/note.txt")
            .expect("note is visible to merge walker");
        assert_eq!(note_entry.whole_file_hash, Some(sha256(b"mounted")));
        assert!(
            tree.resolve(&merged.root_cid, ".hashtree")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn primary_merged_root_surfaces_concurrent_write_conflict_copy() {
        let cfg_dir = tempdir().unwrap();
        let account = Account::create(cfg_dir.path(), Some("owner".into())).unwrap();
        let peer_device =
            "2222222222222222222222222222222222222222222222222222222222222222".to_string();
        let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

        let owner_root =
            index_device_note_root(&tree, &account.state.device_pubkey, b"owner edit", 1, 10).await;
        let peer_root = index_device_note_root(&tree, &peer_device, b"peer edit", 1, 11).await;

        let mut account_state = account.state.clone();
        account_state
            .app_keys
            .as_mut()
            .expect("created account has app keys")
            .devices
            .push(DeviceEntry {
                pubkey: peer_device.clone(),
                added_at: 1,
                label: Some("peer".into()),
            });

        let mut config = AppConfig {
            account: Some(account_state),
            ..AppConfig::default()
        };
        let mut drive = Drive::primary(account.state.owner_pubkey.clone());
        drive.device_roots.insert(
            account.state.device_pubkey.clone(),
            DeviceRootRef::from_meta(owner_root.0.to_string(), 10, &owner_root.1),
        );
        drive.device_roots.insert(
            peer_device,
            DeviceRootRef::from_meta(peer_root.0.to_string(), 11, &peer_root.1),
        );
        config.upsert_drive(drive);

        let view = primary_merged_view(&tree, &config).await.unwrap();
        assert_eq!(view.view.conflicts, vec!["docs/note.txt"]);
        assert_eq!(view.file_count(), 2);
        assert!(
            view.view
                .files
                .iter()
                .any(|entry| entry.path == "docs/note.txt")
        );
        assert!(view.view.files.iter().any(|entry| {
            entry.path.starts_with("docs/note (conflict from ")
                && entry.path.ends_with(").txt")
                && entry.source_path.as_deref() == Some("docs/note.txt")
        }));

        let merged = primary_merged_root(&tree, &config).await.unwrap();
        assert_eq!(merged.file_count, 2);
        let docs_cid = tree
            .resolve(&merged.root_cid, "docs")
            .await
            .unwrap()
            .expect("docs exists");
        let names = tree
            .list_directory(&docs_cid)
            .await
            .unwrap()
            .into_iter()
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 2);
        assert!(names.iter().any(|name| name == "note.txt"));
        assert!(
            names
                .iter()
                .any(|name| name.starts_with("note (conflict from ") && name.ends_with(").txt"))
        );

        let mut contents = Vec::new();
        for name in names {
            let cid = tree
                .resolve(&merged.root_cid, &format!("docs/{name}"))
                .await
                .unwrap()
                .expect("visible file exists");
            contents.push(String::from_utf8(tree.get(&cid, None).await.unwrap().unwrap()).unwrap());
        }
        contents.sort();
        assert_eq!(contents, vec!["owner edit", "peer edit"]);
    }

    #[tokio::test]
    async fn primary_merged_view_ignores_materialized_root_publish_time() {
        let cfg_dir = tempdir().unwrap();
        let account = Account::create(cfg_dir.path(), Some("owner".into())).unwrap();
        let peer_device =
            "3333333333333333333333333333333333333333333333333333333333333333".to_string();
        let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

        let owner_source =
            index_device_note_root(&tree, &account.state.device_pubkey, b"owner source", 1, 10)
                .await;
        let peer_source = index_device_note_root(&tree, &peer_device, b"peer source", 1, 11).await;
        let owner_mirror_meta = DriveRootMeta {
            schema: DriveRootMeta::SCHEMA,
            drive_id: PRIMARY_DRIVE_ID.to_string(),
            device_id: account.state.device_pubkey.clone(),
            device_seq: 2,
            dck_generation: 1,
            materialized_only: true,
            parents: vec![RootParent {
                device_id: account.state.device_pubkey.clone(),
                device_seq: 1,
                root_cid: owner_source.0.to_string(),
            }],
            observed: BTreeMap::new(),
            created_at: 20,
        };
        let owner_mirror = index_device_note_root_with_meta(
            &tree,
            b"materialized mirror",
            20,
            owner_mirror_meta.clone(),
        )
        .await;

        let mut account_state = account.state.clone();
        account_state
            .app_keys
            .as_mut()
            .expect("created account has app keys")
            .devices
            .push(DeviceEntry {
                pubkey: peer_device.clone(),
                added_at: 1,
                label: Some("peer".into()),
            });

        let mut config = AppConfig {
            account: Some(account_state),
            ..AppConfig::default()
        };
        let mut drive = Drive::primary(account.state.owner_pubkey.clone());
        drive.device_roots.insert(
            account.state.device_pubkey.clone(),
            DeviceRootRef::from_meta(owner_mirror.to_string(), 20, &owner_mirror_meta),
        );
        drive.device_roots.insert(
            peer_device.clone(),
            DeviceRootRef::from_meta(peer_source.0.to_string(), 11, &peer_source.1),
        );
        config.upsert_drive(drive);

        let view = primary_merged_view(&tree, &config).await.unwrap();
        let original = view
            .view
            .files
            .iter()
            .find(|entry| entry.path == "docs/note.txt")
            .expect("original path remains visible");
        assert_eq!(original.source_device, peer_device);
    }

    async fn index_device_note_root(
        tree: &HashTree<MemoryStore>,
        device_id: &str,
        bytes: &[u8],
        device_seq: u64,
        published_at: i64,
    ) -> (Cid, DriveRootMeta) {
        let meta = DriveRootMeta {
            schema: DriveRootMeta::SCHEMA,
            drive_id: PRIMARY_DRIVE_ID.to_string(),
            device_id: device_id.to_string(),
            device_seq,
            dck_generation: 1,
            materialized_only: false,
            parents: Vec::new(),
            observed: BTreeMap::new(),
            created_at: published_at,
        };
        let root = index_device_note_root_with_meta(tree, bytes, published_at, meta.clone()).await;
        (root, meta)
    }

    async fn index_device_note_root_with_meta(
        tree: &HashTree<MemoryStore>,
        bytes: &[u8],
        published_at: i64,
        meta: DriveRootMeta,
    ) -> Cid {
        let source = tempdir().unwrap();
        std::fs::create_dir(source.path().join("docs")).unwrap();
        std::fs::write(source.path().join("docs").join("note.txt"), bytes).unwrap();
        let root =
            index_dir_with_history_and_meta(tree, source.path(), None, published_at, Some(&meta))
                .await
                .unwrap();
        root
    }
}
