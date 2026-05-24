//! Multi-device drive merge.
//!
//! Each authorized device publishes its own htree root tree carrying
//! that device's contribution to the drive. The merged drive view is
//! computed by walking every authorized device's tree and resolving
//! per-path conflicts with causal root metadata. Publication time is
//! only a fallback for legacy roots without `device_seq` / observations.
//!
//! Tombstones are stored alongside regular files under a reserved
//! `.hashtree/tombstones/` subtree in each device's root. A tombstone
//! is a leaf whose path is `.hashtree/tombstones/<mirror of original
//! path>` and whose content is the unix-seconds timestamp at which
//! the file was removed. Tombstones participate in the same causal
//! ordering as writes; timestamp ordering is only the legacy fallback.
//!
//! This module is pure logic — it takes pre-fetched per-device entry
//! and tombstone lists and produces a `MergedView`. The actual htree
//! traversal happens in the caller; this keeps the algorithm trivially
//! testable against synthetic inputs.

use std::collections::BTreeMap;

use hashtree_core::{Cid, HashTree, HashTreeError, LinkType, Store, from_hex, to_hex};
use serde::{Deserialize, Serialize};

use crate::config::DeviceRootRef;
use crate::indexer::{path_has_ignored_component, should_ignore_name};

/// Reserved top-level subdirectory inside any hashtree directory for
/// htree-format metadata. Everything iris-drive (and future htree
/// consumers) stashes structurally goes under here, so only one name
/// is ever reserved at the user-visible top level. Currently used for:
///
/// - `.hashtree/prev` — back-link to the prior version of this dir
/// - `.hashtree/root.json` — causal metadata for this root snapshot
/// - `.hashtree/tombstones/<path>` — deletion markers
/// - `.hashtree/conflicts/<id>.json` — conflict provenance records
pub const META_DIR: &str = ".hashtree";

/// Reserved entry path for the root-level causal metadata record.
pub const ROOT_META_PATH: &str = ".hashtree/root.json";

/// Reserved path prefix for the tombstone subtree (inside `META_DIR`).
/// Files written under this prefix by the indexer are not part of the
/// user-visible drive; only the merge engine reads them.
pub const TOMBSTONE_PREFIX: &str = ".hashtree/tombstones";

/// Reserved path prefix for durable conflict provenance records. These
/// records are snapshot metadata and must not appear as user files.
pub const CONFLICTS_PREFIX: &str = ".hashtree/conflicts";

/// Directory-entry metadata key carrying SHA-256 of the whole file
/// plaintext. This is distinct from `hash`, which may be a chunk-tree
/// CID hash or encrypted node hash.
pub const WHOLE_FILE_HASH_META_KEY: &str = "whole_file_hash";

/// Reserved entry path for the directory-revision back-link. A
/// directory whose contents have a prior version stores that previous
/// version's `Cid` (hash + key) at this path. Walking the chain
/// backwards through history is just following `.hashtree/prev` from
/// each `TreeNode` to the next.
///
/// The capability follows naturally: the link's `Cid` carries the
/// decryption key for the prior `TreeNode`, so any reader who can
/// decrypt the current root can decrypt all of history (until either
/// the chain terminates or a block is GC'd).
pub const PREV_LINK_PATH: &str = ".hashtree/prev";

/// One entry from a device's tree, as observed by the merge engine.
/// Hash + size are enough to identify content; the merge does not
/// need to inspect bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceFileEntry {
    pub path: String,
    pub hash: [u8; 32],
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whole_file_hash: Option<[u8; 32]>,
}

/// One tombstone from a device's tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceTombstone {
    /// Original path that was removed (the `.hashtree/tombstones/`
    /// prefix has been stripped).
    pub path: String,
    /// Unix-seconds when this device removed the file.
    pub tombstoned_at: i64,
}

/// What a single device contributes to a merge.
#[derive(Debug, Clone)]
pub struct DeviceSnapshot<'a> {
    pub device_pubkey: &'a str,
    pub root: &'a DeviceRootRef,
    pub files: Vec<DeviceFileEntry>,
    pub tombstones: Vec<DeviceTombstone>,
}

/// One file in the merged view. `source_device` is the device whose
/// write currently wins for this path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedEntry {
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    pub hash: [u8; 32],
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub whole_file_hash: Option<[u8; 32]>,
    pub source_device: String,
    pub published_at: i64,
}

/// Why a path is conflicted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergedConflictKind {
    WriteWrite,
    WriteDelete,
}

/// File-producing side of a conflicted path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedConflictFile {
    pub device_id: String,
    pub device_seq: u64,
    pub root_cid: String,
    pub content_hash: String,
    pub content_cid_hash: String,
    pub size: u64,
}

/// Tombstone-producing side of a conflicted path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedConflictTombstone {
    pub device_id: String,
    pub device_seq: u64,
    pub root_cid: String,
    pub tombstoned_at: i64,
}

/// Provenance for a conflicted path. Callers can turn these sides into
/// visible conflict files plus `.hashtree/conflicts/<id>.json` records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergedConflict {
    pub path: String,
    pub kind: MergedConflictKind,
    pub files: Vec<MergedConflictFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tombstone: Option<MergedConflictTombstone>,
}

/// The full merged drive view: every path that is currently present,
/// plus a paths-that-are-tombstoned list for diagnostics.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MergedView {
    pub files: Vec<MergedEntry>,
    /// Paths suppressed by tombstones (would have been files but a
    /// tombstone is newer than the newest write). Useful in test and
    /// debug output.
    pub suppressed_by_tombstone: Vec<String>,
    /// Paths where concurrent roots disagree. The merged view still
    /// picks a deterministic visible entry for now, but callers can
    /// surface a conflict record/copy for these paths.
    pub conflicts: Vec<String>,
    /// Detailed provenance for each path in `conflicts`.
    pub conflict_details: Vec<MergedConflict>,
}

#[derive(Debug, Clone)]
struct WriteCandidate {
    entry: MergedEntry,
    device_pubkey: String,
    root: DeviceRootRef,
}

#[derive(Debug, Clone)]
struct TombstoneCandidate {
    tombstoned_at: i64,
    device_pubkey: String,
    root: DeviceRootRef,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootRelation {
    Same,
    LeftDescends,
    RightDescends,
    Concurrent,
}

/// Merge across all authorized device snapshots. `authorized_devices`
/// is the device-pubkey allowlist from the current `AppKeys` roster;
/// any snapshot whose `device_pubkey` is not in the allowlist is
/// silently ignored.
///
/// Conflict resolution per path:
///   - causal descendants win over ancestors
///   - newest publication time wins only when causality is unknown
///   - if a tombstone is newer than the newest write, the path is
///     suppressed
///   - if the latest write and a tombstone share the same timestamp,
///     the tombstone wins (deletion is conservative)
#[must_use]
pub fn merge_drives(authorized_devices: &[&str], snapshots: &[DeviceSnapshot<'_>]) -> MergedView {
    let allow: std::collections::BTreeSet<&str> = authorized_devices.iter().copied().collect();

    let mut writes: BTreeMap<String, WriteCandidate> = BTreeMap::new();
    let mut tombstones: BTreeMap<String, TombstoneCandidate> = BTreeMap::new();
    let mut conflicts: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut conflict_details: BTreeMap<String, MergedConflict> = BTreeMap::new();

    for snap in snapshots {
        if snap.root.materialized_only {
            continue;
        }
        if !allow.contains(snap.device_pubkey) {
            continue;
        }
        for f in &snap.files {
            if path_has_ignored_component(&f.path) {
                continue;
            }
            let candidate = MergedEntry {
                path: f.path.clone(),
                source_path: None,
                hash: f.hash,
                size: f.size,
                whole_file_hash: f.whole_file_hash,
                source_device: snap.device_pubkey.to_string(),
                published_at: snap.root.published_at,
            };
            let candidate = WriteCandidate {
                entry: candidate,
                device_pubkey: snap.device_pubkey.to_string(),
                root: snap.root.clone(),
            };
            writes
                .entry(f.path.clone())
                .and_modify(|existing| {
                    if should_mark_write_conflict(&candidate, existing) {
                        conflicts.insert(candidate.entry.path.clone());
                        record_write_conflict(&mut conflict_details, &candidate, existing);
                    }
                    if write_candidate_wins(&candidate, existing) {
                        *existing = candidate.clone();
                    }
                })
                .or_insert(candidate);
        }
        for t in &snap.tombstones {
            if path_has_ignored_component(&t.path) {
                continue;
            }
            let candidate = TombstoneCandidate {
                tombstoned_at: t.tombstoned_at,
                device_pubkey: snap.device_pubkey.to_string(),
                root: snap.root.clone(),
            };
            tombstones
                .entry(t.path.clone())
                .and_modify(|cur| {
                    if tombstone_candidate_wins(&candidate, cur) {
                        *cur = candidate.clone();
                    }
                })
                .or_insert(candidate);
        }
    }

    let mut view = MergedView::default();
    for (path, write) in &writes {
        let tombstone = tombstones.get(path);
        if tombstone.is_some_and(|t| should_mark_write_delete_conflict(write, t)) {
            conflicts.insert(path.clone());
            record_write_delete_conflict(
                &mut conflict_details,
                path.as_str(),
                write,
                tombstone.expect("checked is_some"),
            );
        }
        let suppressed = tombstone.is_some_and(|t| tombstone_suppresses_write(t, write));
        if suppressed {
            view.suppressed_by_tombstone.push(path.clone());
        } else {
            view.files.push(write.entry.clone());
        }
    }
    // Tombstones that don't have a surviving write anywhere should also
    // show up as evidence the path was deleted. Without this, a
    // single-device delete (no concurrent write to suppress) would
    // silently vanish from both lists.
    for path in tombstones.keys() {
        if !writes.contains_key(path) {
            view.suppressed_by_tombstone.push(path.clone());
        }
    }
    view.files.sort_by(|a, b| a.path.cmp(&b.path));
    view.suppressed_by_tombstone.sort();
    view.suppressed_by_tombstone.dedup();
    view.conflicts = conflicts.into_iter().collect();
    view.conflict_details = conflict_details.into_values().collect();
    view
}

fn record_write_conflict(
    conflicts: &mut BTreeMap<String, MergedConflict>,
    candidate: &WriteCandidate,
    existing: &WriteCandidate,
) {
    let path = candidate.entry.path.clone();
    let detail = conflicts
        .entry(path.clone())
        .or_insert_with(|| MergedConflict {
            path,
            kind: MergedConflictKind::WriteWrite,
            files: Vec::new(),
            tombstone: None,
        });
    detail.kind = MergedConflictKind::WriteWrite;
    insert_conflict_file(&mut detail.files, conflict_file_side(candidate));
    insert_conflict_file(&mut detail.files, conflict_file_side(existing));
}

fn record_write_delete_conflict(
    conflicts: &mut BTreeMap<String, MergedConflict>,
    path: &str,
    write: &WriteCandidate,
    tombstone: &TombstoneCandidate,
) {
    let detail = conflicts
        .entry(path.to_string())
        .or_insert_with(|| MergedConflict {
            path: path.to_string(),
            kind: MergedConflictKind::WriteDelete,
            files: Vec::new(),
            tombstone: None,
        });
    detail.kind = MergedConflictKind::WriteDelete;
    insert_conflict_file(&mut detail.files, conflict_file_side(write));
    detail.tombstone = Some(MergedConflictTombstone {
        device_id: tombstone.device_pubkey.clone(),
        device_seq: tombstone.root.device_seq,
        root_cid: tombstone.root.root_cid.clone(),
        tombstoned_at: tombstone.tombstoned_at,
    });
}

fn insert_conflict_file(files: &mut Vec<MergedConflictFile>, file: MergedConflictFile) {
    if !files.iter().any(|f| {
        f.device_id == file.device_id
            && f.device_seq == file.device_seq
            && f.root_cid == file.root_cid
            && f.content_hash == file.content_hash
    }) {
        files.push(file);
        files.sort_by(|a, b| {
            a.device_id
                .cmp(&b.device_id)
                .then(a.device_seq.cmp(&b.device_seq))
                .then(a.root_cid.cmp(&b.root_cid))
                .then(a.content_hash.cmp(&b.content_hash))
        });
    }
}

fn conflict_file_side(write: &WriteCandidate) -> MergedConflictFile {
    MergedConflictFile {
        device_id: write.device_pubkey.clone(),
        device_seq: write.root.device_seq,
        root_cid: write.root.root_cid.clone(),
        content_hash: to_hex(&identity_hash(&write.entry)),
        content_cid_hash: to_hex(&write.entry.hash),
        size: write.entry.size,
    }
}

fn write_candidate_wins(candidate: &WriteCandidate, existing: &WriteCandidate) -> bool {
    match root_relation(
        &candidate.device_pubkey,
        &candidate.root,
        &existing.device_pubkey,
        &existing.root,
    ) {
        RootRelation::Same | RootRelation::LeftDescends => true,
        RootRelation::RightDescends => false,
        RootRelation::Concurrent => fallback_newer(
            candidate.entry.published_at,
            &candidate.device_pubkey,
            existing.entry.published_at,
            &existing.device_pubkey,
        ),
    }
}

fn tombstone_candidate_wins(candidate: &TombstoneCandidate, existing: &TombstoneCandidate) -> bool {
    match root_relation(
        &candidate.device_pubkey,
        &candidate.root,
        &existing.device_pubkey,
        &existing.root,
    ) {
        RootRelation::Same | RootRelation::LeftDescends => true,
        RootRelation::RightDescends => false,
        RootRelation::Concurrent => fallback_newer(
            candidate.tombstoned_at,
            &candidate.device_pubkey,
            existing.tombstoned_at,
            &existing.device_pubkey,
        ),
    }
}

fn tombstone_suppresses_write(tombstone: &TombstoneCandidate, write: &WriteCandidate) -> bool {
    match root_relation(
        &tombstone.device_pubkey,
        &tombstone.root,
        &write.device_pubkey,
        &write.root,
    ) {
        RootRelation::Same | RootRelation::LeftDescends => true,
        RootRelation::RightDescends => false,
        RootRelation::Concurrent => tombstone.tombstoned_at >= write.entry.published_at,
    }
}

fn should_mark_write_conflict(candidate: &WriteCandidate, existing: &WriteCandidate) -> bool {
    if file_identity_matches(&candidate.entry, &existing.entry) {
        return false;
    }
    roots_are_causal(&candidate.root)
        && roots_are_causal(&existing.root)
        && matches!(
            root_relation(
                &candidate.device_pubkey,
                &candidate.root,
                &existing.device_pubkey,
                &existing.root,
            ),
            RootRelation::Concurrent
        )
}

fn file_identity_matches(left: &MergedEntry, right: &MergedEntry) -> bool {
    left.size == right.size && identity_hash(left) == identity_hash(right)
}

fn identity_hash(entry: &MergedEntry) -> [u8; 32] {
    entry.whole_file_hash.unwrap_or(entry.hash)
}

fn should_mark_write_delete_conflict(
    write: &WriteCandidate,
    tombstone: &TombstoneCandidate,
) -> bool {
    roots_are_causal(&write.root)
        && roots_are_causal(&tombstone.root)
        && matches!(
            root_relation(
                &write.device_pubkey,
                &write.root,
                &tombstone.device_pubkey,
                &tombstone.root,
            ),
            RootRelation::Concurrent
        )
}

fn root_relation(
    left_device: &str,
    left: &DeviceRootRef,
    right_device: &str,
    right: &DeviceRootRef,
) -> RootRelation {
    if left.root_cid == right.root_cid {
        return RootRelation::Same;
    }
    let left_descends = root_observes(left_device, left, right_device, right);
    let right_descends = root_observes(right_device, right, left_device, left);
    match (left_descends, right_descends) {
        (true, false) => RootRelation::LeftDescends,
        (false, true) => RootRelation::RightDescends,
        _ => RootRelation::Concurrent,
    }
}

fn root_observes(
    newer_device_id: &str,
    newer_root: &DeviceRootRef,
    candidate_device_id: &str,
    candidate_root: &DeviceRootRef,
) -> bool {
    if newer_root.root_cid == candidate_root.root_cid {
        return true;
    }
    if newer_device_id == candidate_device_id
        && newer_root.device_seq > 0
        && candidate_root.device_seq > 0
        && newer_root.device_seq > candidate_root.device_seq
    {
        return true;
    }
    if newer_root.parents.iter().any(|parent| {
        parent.device_id == candidate_device_id
            && (parent.root_cid == candidate_root.root_cid
                || (candidate_root.device_seq > 0 && parent.device_seq > candidate_root.device_seq))
    }) {
        return true;
    }
    newer_root
        .observed
        .get(candidate_device_id)
        .is_some_and(|o| {
            o.root_cid == candidate_root.root_cid
                || (candidate_root.device_seq > 0 && o.device_seq > candidate_root.device_seq)
        })
}

fn roots_are_causal(root: &DeviceRootRef) -> bool {
    root.device_seq > 0 || !root.parents.is_empty() || !root.observed.is_empty()
}

fn fallback_newer(left_time: i64, left_device: &str, right_time: i64, right_device: &str) -> bool {
    left_time > right_time || (left_time == right_time && left_device > right_device)
}

/// Encode a file path into the path under `.hashtree/tombstones/`
/// used to store the tombstone leaf in htree.
#[must_use]
pub fn tombstone_path(file_path: &str) -> String {
    format!("{TOMBSTONE_PREFIX}/{file_path}")
}

/// Inverse of `tombstone_path`. Returns `None` if the input is not
/// under the tombstone prefix.
#[must_use]
pub fn original_path_from_tombstone(tombstone_path: &str) -> Option<&str> {
    tombstone_path
        .strip_prefix(TOMBSTONE_PREFIX)
        .and_then(|rest| rest.strip_prefix('/'))
}

/// Walk an htree directory root and partition its contents into
/// regular files and tombstones. Tombstone leaves (under `.tombstones/`)
/// are decoded by parsing their content as a unix-seconds integer; any
/// leaf whose content can't be parsed is silently skipped.
pub async fn walk_device_tree<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
) -> Result<(Vec<DeviceFileEntry>, Vec<DeviceTombstone>), HashTreeError> {
    let mut files = Vec::new();
    let mut tombstones = Vec::new();
    walk_dir_recursive(tree, root, "", &mut files, &mut tombstones).await?;
    Ok((files, tombstones))
}

/// Walk inside `.hashtree/` collecting tombstones and ignoring other
/// structural metadata.
/// Lifted out so the main walker doesn't need to know `META_DIR`
/// internals.
fn walk_meta_dir<'a, S: Store>(
    tree: &'a HashTree<S>,
    dir_cid: &'a Cid,
    prefix: &'a str,
    files: &'a mut Vec<DeviceFileEntry>,
    tombstones: &'a mut Vec<DeviceTombstone>,
) -> futures::future::BoxFuture<'a, Result<(), HashTreeError>> {
    Box::pin(async move {
        let entries = tree.list_directory(dir_cid).await?;
        for entry in entries {
            let path = format!("{prefix}/{}", entry.name);
            // Structural metadata is not user-visible content. Only
            // the tombstone subtree is intentionally traversed.
            if path == PREV_LINK_PATH || path == ROOT_META_PATH || path == CONFLICTS_PREFIX {
                continue;
            }
            let child_cid = Cid {
                hash: entry.hash,
                key: entry.key,
            };
            if entry.link_type == LinkType::Dir && path == TOMBSTONE_PREFIX {
                // The tombstones subtree mirrors original paths; recurse
                // and let the tombstone-leaf check below pick them up.
                walk_dir_recursive(tree, &child_cid, &path, files, tombstones).await?;
            }
        }
        Ok(())
    })
}

fn walk_dir_recursive<'a, S: Store>(
    tree: &'a HashTree<S>,
    dir_cid: &'a Cid,
    prefix: &'a str,
    files: &'a mut Vec<DeviceFileEntry>,
    tombstones: &'a mut Vec<DeviceTombstone>,
) -> futures::future::BoxFuture<'a, Result<(), HashTreeError>> {
    Box::pin(async move {
        let entries = tree.list_directory(dir_cid).await?;
        for entry in entries {
            let path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{prefix}/{}", entry.name)
            };
            let child_cid = Cid {
                hash: entry.hash,
                key: entry.key,
            };
            if entry.name == META_DIR && prefix.is_empty() {
                // `.hashtree/` subtree carries htree-format metadata
                // (revision back-link, tombstones, ...). Recurse so the
                // tombstone walker can pick up entries under
                // `.hashtree/tombstones/`, but skip the `prev` link.
                walk_meta_dir(tree, &child_cid, META_DIR, files, tombstones).await?;
                continue;
            }
            if should_ignore_name(&entry.name) {
                continue;
            }
            if entry.link_type == LinkType::Dir {
                walk_dir_recursive(tree, &child_cid, &path, files, tombstones).await?;
            } else if let Some(orig_path) = original_path_from_tombstone(&path) {
                let raw = tree.get(&child_cid, None).await?.unwrap_or_default();
                let ts_str = String::from_utf8_lossy(&raw);
                if let Ok(tombstoned_at) = ts_str.trim().parse::<i64>() {
                    tombstones.push(DeviceTombstone {
                        path: orig_path.to_string(),
                        tombstoned_at,
                    });
                } else {
                    tracing::warn!("malformed tombstone at {path}: {ts_str:?}");
                }
            } else {
                files.push(DeviceFileEntry {
                    path,
                    hash: entry.hash,
                    size: entry.size,
                    whole_file_hash: whole_file_hash_from_meta(entry.meta.as_ref()),
                });
            }
        }
        Ok(())
    })
}

fn whole_file_hash_from_meta(
    meta: Option<&std::collections::HashMap<String, serde_json::Value>>,
) -> Option<[u8; 32]> {
    meta.and_then(|m| m.get(WHOLE_FILE_HASH_META_KEY))
        .and_then(serde_json::Value::as_str)
        .and_then(|s| from_hex(s).ok())
}

#[cfg(test)]
mod tests;
