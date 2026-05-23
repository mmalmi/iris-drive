//! Bidirectional sync engine.
//!
//! Reconciles two `ProviderFs` instances by feeding per-path snapshots
//! through the conflict resolver from [`crate::conflict`].
//!
//! `sync_with_cache` uses provider anchors and `changes_since(Some(_))`
//! after the first accepted base; full enumeration remains the first-sync
//! and explicit fallback path.
//!
//! Conflict policy: keep both sides. Local stays at the original path;
//! the remote's bytes land in `name (conflict from peer).ext` on the
//! local side. The symmetric rename happens when the peer runs its own
//! sync — which makes the algorithm deterministic without coordination.

use std::collections::{BTreeMap, BTreeSet};

use hashtree_core::to_hex;
use hashtree_provider::{EntryInfo, PathChange, ProviderError, ProviderFs, SyncAnchor};
use thiserror::Error;

use crate::conflict::{
    FileSnapshot, SyncAction, conflict_filename, parse_conflict_filename, resolve,
};
use crate::sync_cache::{CachedBaseState, SyncCache};

const MAX_CONFLICT_COPIES_PER_PATH: usize = 32;

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

pub type SyncBaseState = BTreeMap<String, FileSnapshot>;

/// Run one full bidirectional sync.
///
/// `device_label` is used in the conflict filename. The peer's label is
/// always rendered as `"peer"` in local-side renames, since the peer's
/// own label is unknown here.
pub async fn sync<L, R>(local: &L, remote: &R, device_label: &str) -> Result<SyncReport, SyncError>
where
    L: ProviderFs<ItemId = String>,
    R: ProviderFs<ItemId = String>,
{
    let base = SyncBaseState::new();
    let _ = device_label;
    sync_with_base(local, remote, &base, "peer").await
}

/// Run one bidirectional sync using durable base snapshots to distinguish
/// deletes from additions and unchanged peer copies.
pub async fn sync_with_base<L, R>(
    local: &L,
    remote: &R,
    base_state: &SyncBaseState,
    remote_label: &str,
) -> Result<SyncReport, SyncError>
where
    L: ProviderFs<ItemId = String>,
    R: ProviderFs<ItemId = String>,
{
    let local_entries = enumerate_files(local).await?;
    let remote_entries = enumerate_files(remote).await?;

    let mut report = SyncReport::default();
    let all_paths: BTreeSet<String> = local_entries
        .keys()
        .chain(remote_entries.keys())
        .chain(base_state.keys())
        .cloned()
        .collect();
    let mut occupied_paths = all_paths.clone();

    for path in &all_paths {
        let l = local_entries.get(path);
        let r = remote_entries.get(path);
        let local_snapshot = l.map(snapshot_from_entry);
        let remote_snapshot = r.map(snapshot_from_entry);
        match resolve(
            path,
            base_state.get(path),
            local_snapshot.as_ref(),
            remote_snapshot.as_ref(),
            remote_label,
        ) {
            SyncAction::NoOp => {}
            SyncAction::ApplyRemote { new } => {
                let _ = new;
                if r.is_some() && !l.zip(r).is_some_and(|(l, r)| hashes_match(l, r)) {
                    copy_file(remote, path, local, path).await?;
                    occupied_paths.insert(path.clone());
                    report.downloaded.push(path.clone());
                }
            }
            SyncAction::KeepLocal => {
                if l.is_some() && !l.zip(r).is_some_and(|(l, r)| hashes_match(l, r)) {
                    copy_file(local, path, remote, path).await?;
                    occupied_paths.insert(path.clone());
                    report.uploaded.push(path.clone());
                }
            }
            SyncAction::DeleteLocal => {
                if l.is_some() {
                    remove_file(local, path).await?;
                    occupied_paths.remove(path);
                    report.deleted_local.push(path.clone());
                }
            }
            SyncAction::DeleteRemote => {
                if r.is_some() {
                    remove_file(remote, path).await?;
                    report.deleted_remote.push(path.clone());
                }
            }
            SyncAction::Conflict {
                remote: remote_file,
                conflict_name,
            } => {
                let _ = (remote_file, conflict_name);
                let original_path = parse_conflict_filename(path)
                    .map_or_else(|| path.clone(), |parsed| parsed.original_path);
                let conflict_path =
                    next_conflict_filename(&original_path, remote_label, &occupied_paths)?;
                copy_file(remote, path, local, &conflict_path).await?;
                occupied_paths.insert(conflict_path.clone());
                report.conflicts.push(ConflictResolution {
                    original_path,
                    renamed_to: conflict_path,
                });
                // Also push local's version to remote so they converge on
                // the original path. The remote will, on its own sync,
                // observe local-vs-its-old-content as a separate diff and
                // produce its own conflict rename labelled with its own
                // device name.
                copy_file(local, path, remote, path).await?;
                occupied_paths.insert(path.clone());
                report.uploaded.push(path.clone());
            }
        }
    }

    Ok(report)
}

/// Run one bidirectional sync from a known base provider anchor.
///
/// This applies only paths that changed since `base_anchor`; unchanged
/// side state is supplied by `base_state`. File bytes are read only for
/// paths that actually need to be copied or preserved as conflicts.
pub async fn sync_with_base_anchor<L, R>(
    local: &L,
    remote: &R,
    base_state: &SyncBaseState,
    base_anchor: &SyncAnchor,
    remote_label: &str,
) -> Result<SyncReport, SyncError>
where
    L: ProviderFs<ItemId = String>,
    R: ProviderFs<ItemId = String>,
{
    let local_changes = file_changes_since(local, base_anchor).await?;
    let remote_changes = file_changes_since(remote, base_anchor).await?;

    let all_paths: BTreeSet<String> = local_changes
        .keys()
        .chain(remote_changes.keys())
        .cloned()
        .collect();
    let mut occupied_paths: BTreeSet<String> = base_state.keys().cloned().collect();
    reserve_present_changes(&mut occupied_paths, &local_changes);
    reserve_present_changes(&mut occupied_paths, &remote_changes);

    let mut report = SyncReport::default();
    for path in &all_paths {
        let local_snapshot = snapshot_after_change(path, &local_changes, base_state);
        let remote_snapshot = snapshot_after_change(path, &remote_changes, base_state);
        match resolve(
            path,
            base_state.get(path),
            local_snapshot.as_ref(),
            remote_snapshot.as_ref(),
            remote_label,
        ) {
            SyncAction::NoOp => {}
            SyncAction::ApplyRemote { new } => {
                let _ = new;
                if remote_snapshot.is_some() && local_snapshot != remote_snapshot {
                    copy_file(remote, path, local, path).await?;
                    occupied_paths.insert(path.clone());
                    report.downloaded.push(path.clone());
                }
            }
            SyncAction::KeepLocal => {
                if local_snapshot.is_some() && local_snapshot != remote_snapshot {
                    copy_file(local, path, remote, path).await?;
                    occupied_paths.insert(path.clone());
                    report.uploaded.push(path.clone());
                }
            }
            SyncAction::DeleteLocal => {
                if local_snapshot.is_some() {
                    remove_file(local, path).await?;
                    occupied_paths.remove(path);
                    report.deleted_local.push(path.clone());
                }
            }
            SyncAction::DeleteRemote => {
                if remote_snapshot.is_some() {
                    remove_file(remote, path).await?;
                    report.deleted_remote.push(path.clone());
                }
            }
            SyncAction::Conflict {
                remote: remote_file,
                conflict_name,
            } => {
                let _ = (remote_file, conflict_name);
                let original_path = parse_conflict_filename(path)
                    .map_or_else(|| path.clone(), |parsed| parsed.original_path);
                let conflict_path =
                    next_conflict_filename(&original_path, remote_label, &occupied_paths)?;
                copy_file(remote, path, local, &conflict_path).await?;
                occupied_paths.insert(conflict_path.clone());
                report.conflicts.push(ConflictResolution {
                    original_path,
                    renamed_to: conflict_path,
                });
                copy_file(local, path, remote, path).await?;
                occupied_paths.insert(path.clone());
                report.uploaded.push(path.clone());
            }
        }
    }

    Ok(report)
}

/// Run one sync using the cache's durable base state, then refresh the
/// accepted base from the converged local provider state when no conflicts
/// were produced. Callers remain responsible for saving the cache.
pub async fn sync_with_cache<L, R>(
    local: &L,
    remote: &R,
    cache: &mut SyncCache,
    drive_id: &str,
    remote_label: &str,
) -> Result<SyncReport, SyncError>
where
    L: ProviderFs<ItemId = String>,
    R: ProviderFs<ItemId = String>,
{
    let base = cache.base_snapshots_for_drive(drive_id);
    let base_anchor = base_anchor_for_drive(cache, drive_id);
    let report = if let Some(base_anchor) = base_anchor.as_ref() {
        sync_with_base_anchor(local, remote, &base, base_anchor, remote_label).await?
    } else {
        sync_with_base(local, remote, &base, remote_label).await?
    };
    if report.conflicts.is_empty() {
        if let Some(base_anchor) = base_anchor.as_ref() {
            refresh_base_state_from_anchor(local, cache, drive_id, base_anchor).await?;
        } else {
            refresh_base_state_from_full_enumeration(local, cache, drive_id).await?;
        }
    }
    Ok(report)
}

fn base_anchor_for_drive(cache: &SyncCache, drive_id: &str) -> Option<SyncAnchor> {
    cache
        .base_anchor_for_drive(drive_id)
        .map(|anchor| SyncAnchor(anchor.to_string()))
}

async fn refresh_base_state_from_full_enumeration<P>(
    local: &P,
    cache: &mut SyncCache,
    drive_id: &str,
) -> Result<(), SyncError>
where
    P: ProviderFs<ItemId = String>,
{
    let anchor = local.anchor().await;
    let base_root_cid = anchor.as_str().to_string();
    let rows = enumerate_files(local)
        .await?
        .into_iter()
        .map(|(path, entry)| cached_base_row_from_entry(drive_id, path, &base_root_cid, &entry));
    cache.replace_base_state_for_drive_at_anchor(drive_id, &base_root_cid, rows);
    Ok(())
}

async fn refresh_base_state_from_anchor<P>(
    local: &P,
    cache: &mut SyncCache,
    drive_id: &str,
    base_anchor: &SyncAnchor,
) -> Result<(), SyncError>
where
    P: ProviderFs<ItemId = String>,
{
    let anchor = local.anchor().await;
    let base_root_cid = anchor.as_str().to_string();
    let mut rows: BTreeMap<String, CachedBaseState> = cache
        .base_state
        .iter()
        .filter(|row| row.drive_id == drive_id)
        .map(|row| {
            let mut row = row.clone();
            row.base_root_cid.clone_from(&base_root_cid);
            (row.path.clone(), row)
        })
        .collect();

    for change in local.changes_since(Some(base_anchor)).await? {
        match change {
            PathChange::Added { path, entry }
            | PathChange::Modified {
                path, new: entry, ..
            } if entry.link_type != hashtree_core::LinkType::Dir => {
                rows.insert(
                    path.clone(),
                    cached_base_row_from_entry(drive_id, path, &base_root_cid, &entry),
                );
            }
            PathChange::Removed { path, entry }
                if entry.link_type != hashtree_core::LinkType::Dir =>
            {
                rows.remove(&path);
            }
            _ => {}
        }
    }

    cache.replace_base_state_for_drive_at_anchor(drive_id, &base_root_cid, rows.into_values());
    Ok(())
}

fn cached_base_row_from_entry(
    drive_id: &str,
    path: String,
    base_root_cid: &str,
    entry: &EntryInfo,
) -> CachedBaseState {
    CachedBaseState {
        drive_id: drive_id.to_string(),
        path,
        base_root_cid: base_root_cid.to_string(),
        whole_file_hash: None,
        content_cid_hash: to_hex(&entry.hash),
        size: entry.size,
    }
}

fn hashes_match(l: &EntryInfo, r: &EntryInfo) -> bool {
    l.hash == r.hash && l.size == r.size
}

fn snapshot_from_entry(entry: &EntryInfo) -> FileSnapshot {
    FileSnapshot {
        content_hash: to_hex(&entry.hash),
        mtime: 0,
    }
}

async fn file_changes_since<P: ProviderFs<ItemId = String>>(
    fs: &P,
    anchor: &SyncAnchor,
) -> Result<BTreeMap<String, Option<EntryInfo>>, SyncError> {
    let changes = fs.changes_since(Some(anchor)).await?;
    let mut out = BTreeMap::new();
    for change in changes {
        match change {
            PathChange::Added { path, entry }
            | PathChange::Modified {
                path, new: entry, ..
            } if entry.link_type != hashtree_core::LinkType::Dir => {
                out.insert(path, Some(entry));
            }
            PathChange::Removed { path, entry }
                if entry.link_type != hashtree_core::LinkType::Dir =>
            {
                out.insert(path, None);
            }
            _ => {}
        }
    }
    Ok(out)
}

fn snapshot_after_change(
    path: &str,
    changes: &BTreeMap<String, Option<EntryInfo>>,
    base_state: &SyncBaseState,
) -> Option<FileSnapshot> {
    match changes.get(path) {
        Some(Some(entry)) => Some(snapshot_from_entry(entry)),
        Some(None) => None,
        None => base_state.get(path).cloned(),
    }
}

fn reserve_present_changes(
    occupied_paths: &mut BTreeSet<String>,
    changes: &BTreeMap<String, Option<EntryInfo>>,
) {
    for (path, entry) in changes {
        if entry.is_some() {
            occupied_paths.insert(path.clone());
        }
    }
}

fn next_conflict_filename(
    original_path: &str,
    device_label: &str,
    occupied_paths: &BTreeSet<String>,
) -> Result<String, SyncError> {
    for copy_index in 1..=MAX_CONFLICT_COPIES_PER_PATH {
        let label = if copy_index == 1 {
            device_label.to_string()
        } else {
            format!("{device_label} {copy_index}")
        };
        let candidate = conflict_filename(original_path, &label);
        if !occupied_paths.contains(&candidate) {
            return Ok(candidate);
        }
    }

    Err(SyncError::ConflictApply {
        path: original_path.to_string(),
        reason: format!("more than {MAX_CONFLICT_COPIES_PER_PATH} conflict copies exist"),
    })
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

async fn read_full<P: ProviderFs<ItemId = String>>(
    fs: &P,
    path: &str,
) -> Result<Vec<u8>, SyncError> {
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

async fn remove_file<P: ProviderFs<ItemId = String>>(fs: &P, path: &str) -> Result<(), SyncError> {
    let (parent, name) = split_path(path);
    let parent_id = lookup_dir(fs, &parent).await?;
    match fs.remove(&parent_id, name).await {
        Ok(()) | Err(ProviderError::NotFound) => Ok(()),
        Err(e) => Err(SyncError::Provider(e)),
    }
}

async fn ensure_parents<P: ProviderFs<ItemId = String>>(
    fs: &P,
    path: &str,
) -> Result<(), SyncError> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if segments.len() <= 1 {
        return Ok(());
    }
    let mut cursor = fs.root().await;
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

async fn lookup_dir<P: ProviderFs<ItemId = String>>(
    fs: &P,
    path: &str,
) -> Result<String, SyncError> {
    let mut cursor = fs.root().await;
    for seg in path.split('/').filter(|s| !s.is_empty()) {
        cursor = fs.lookup(&cursor, seg).await?.id;
    }
    Ok(cursor)
}

fn split_path(path: &str) -> (String, &str) {
    match path.rfind('/') {
        Some(i) => (path[..i].to_string(), &path[i + 1..]),
        None => (String::new(), path),
    }
}

#[cfg(test)]
mod tests;
