//! Apply the merged signed drive snapshot to a local working directory.
//!
//! Network sync gets remote root metadata and blocks into the local store. This
//! module performs the next step: make the user-visible folder match the merged
//! drive view, without overwriting unimported local edits.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hashtree_core::{Cid, CidParseError, HashTree, HashTreeError, LinkType, Store, sha256};
use hashtree_provider::{HashTreeProviderFs, ProviderError, ProviderFs};
use thiserror::Error;

use crate::PRIMARY_DRIVE_ID;
use crate::account::AccountState;
use crate::config::{AppConfig, DeviceRootRef};
use crate::indexer::{IndexError, read_root_meta};
use crate::merge::{
    DeviceFileEntry, DeviceSnapshot, MergedEntry, MergedView, merge_drives, walk_device_tree,
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
    let view = merge_drives(&authorized_refs, &snapshots);
    Ok(PrimaryMergedView {
        view,
        authorized_devices: authorized.len(),
        device_roots_present: snapshots.len(),
    })
}

/// Copy the merged primary-drive view into `working_dir`.
///
/// Existing files are overwritten only when they still match this device's
/// last imported root. If a user edited a file on disk and the daemon has not
/// imported that edit yet, materialization skips that path and lets the normal
/// scan/publish loop handle the local change.
pub async fn materialize_primary_drive<S>(
    tree: Arc<HashTree<S>>,
    config: &AppConfig,
    working_dir: &Path,
) -> Result<MaterializeReport, MaterializeError>
where
    S: Store + Send + Sync + 'static,
{
    std::fs::create_dir_all(working_dir)?;
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
    materialize_target_dirs(working_dir, &target_dirs, &local_entries, &mut report)?;
    materialize_target_files(
        tree,
        drive,
        working_dir,
        &target_by_path,
        &local_entries,
        &mut report,
    )
    .await?;
    delete_removed_local_files(working_dir, &target_by_path, &local_entries, &mut report)?;

    Ok(report)
}

fn materialize_target_dirs(
    working_dir: &Path,
    target_dirs: &BTreeSet<String>,
    local_entries: &BTreeMap<String, DeviceFileEntry>,
    report: &mut MaterializeReport,
) -> Result<(), MaterializeError> {
    for dir in target_dirs {
        let Some(relative) = safe_relative_path(dir) else {
            report.skipped += 1;
            continue;
        };
        let destination = working_dir.join(relative);
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

async fn materialize_target_files<S>(
    tree: Arc<HashTree<S>>,
    drive: &crate::config::Drive,
    working_dir: &Path,
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
        let destination = working_dir.join(relative);
        if destination.is_dir() {
            if !directory_matches_local_entries(working_dir, &entry.path, local_entries)? {
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
        let bytes = read_file_from_root(tree.clone(), &source_root.root_cid, &entry.path).await?;
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
    working_dir: &Path,
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
        let destination = working_dir.join(relative);
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
        current = DeviceRootRef::from_meta(parent.root_cid.clone(), current.published_at, &meta);
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
    working_dir: &Path,
    path: &str,
    local_entries: &BTreeMap<String, DeviceFileEntry>,
) -> Result<bool, MaterializeError> {
    let Some(relative) = safe_relative_path(path) else {
        return Ok(false);
    };
    let directory = working_dir.join(relative);
    let actual_files = collect_disk_file_paths(working_dir, &directory)?;
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
        let snapshot = file_snapshot(&working_dir.join(relative))?;
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
}
