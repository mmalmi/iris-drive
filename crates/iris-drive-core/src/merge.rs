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

use hashtree_core::{Cid, HashTree, HashTreeError, LinkType, Store};
use serde::{Deserialize, Serialize};

use crate::config::DeviceRootRef;

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
    pub hash: [u8; 32],
    pub size: u64,
    pub source_device: String,
    pub published_at: i64,
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

    for snap in snapshots {
        if !allow.contains(snap.device_pubkey) {
            continue;
        }
        for f in &snap.files {
            let candidate = MergedEntry {
                path: f.path.clone(),
                hash: f.hash,
                size: f.size,
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
                    }
                    if write_candidate_wins(&candidate, existing) {
                        *existing = candidate.clone();
                    }
                })
                .or_insert(candidate);
        }
        for t in &snap.tombstones {
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
    view
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
    if candidate.entry.hash == existing.entry.hash && candidate.entry.size == existing.entry.size {
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
    observer_device: &str,
    observer: &DeviceRootRef,
    observed_device: &str,
    observed: &DeviceRootRef,
) -> bool {
    if observer.root_cid == observed.root_cid {
        return true;
    }
    if observer_device == observed_device
        && observer.device_seq > 0
        && observed.device_seq > 0
        && observer.device_seq > observed.device_seq
    {
        return true;
    }
    if observer.parents.iter().any(|parent| {
        parent.device_id == observed_device
            && (parent.root_cid == observed.root_cid
                || (observed.device_seq > 0 && parent.device_seq > observed.device_seq))
    }) {
        return true;
    }
    observer.observed.get(observed_device).is_some_and(|o| {
        o.root_cid == observed.root_cid
            || (observed.device_seq > 0 && o.device_seq > observed.device_seq)
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
                });
            }
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev_root(published_at: i64) -> DeviceRootRef {
        DeviceRootRef::legacy(format!("cid-{published_at}"), published_at, 1)
    }

    fn causal_root(
        root_cid: &str,
        published_at: i64,
        device_seq: u64,
        observed: &[(&str, u64, &str)],
    ) -> DeviceRootRef {
        DeviceRootRef {
            root_cid: root_cid.into(),
            published_at,
            dck_generation: 1,
            device_seq,
            parents: Vec::new(),
            observed: observed
                .iter()
                .map(|(device, seq, cid)| {
                    (
                        (*device).to_string(),
                        crate::RootObservation {
                            device_seq: *seq,
                            root_cid: (*cid).to_string(),
                        },
                    )
                })
                .collect(),
        }
    }

    fn file(path: &str, hash_byte: u8, size: u64) -> DeviceFileEntry {
        DeviceFileEntry {
            path: path.into(),
            hash: [hash_byte; 32],
            size,
        }
    }

    fn tomb(path: &str, at: i64) -> DeviceTombstone {
        DeviceTombstone {
            path: path.into(),
            tombstoned_at: at,
        }
    }

    fn snap<'a>(
        device: &'a str,
        root: &'a DeviceRootRef,
        files: Vec<DeviceFileEntry>,
        tombstones: Vec<DeviceTombstone>,
    ) -> DeviceSnapshot<'a> {
        DeviceSnapshot {
            device_pubkey: device,
            root,
            files,
            tombstones,
        }
    }

    #[test]
    fn empty_merge_is_empty() {
        let view = merge_drives(&[], &[]);
        assert!(view.files.is_empty());
        assert!(view.suppressed_by_tombstone.is_empty());
    }

    #[test]
    fn single_device_files_pass_through() {
        let r = dev_root(100);
        let view = merge_drives(
            &["dev-a"],
            &[snap(
                "dev-a",
                &r,
                vec![file("hello.txt", 1, 5), file("dir/x", 2, 3)],
                vec![],
            )],
        );
        assert_eq!(view.files.len(), 2);
        assert_eq!(view.files[0].path, "dir/x");
        assert_eq!(view.files[0].source_device, "dev-a");
        assert_eq!(view.files[1].path, "hello.txt");
    }

    #[test]
    fn unauthorized_device_is_ignored() {
        let r_ok = dev_root(100);
        let r_evil = dev_root(999);
        let view = merge_drives(
            &["dev-a"], // only dev-a authorized
            &[
                snap("dev-a", &r_ok, vec![file("ok.txt", 1, 1)], vec![]),
                snap(
                    "dev-evil",
                    &r_evil,
                    vec![file("ok.txt", 9, 1)], // tries to overwrite
                    vec![],
                ),
            ],
        );
        // dev-evil's write doesn't win because it isn't in the allow list.
        assert_eq!(view.files.len(), 1);
        assert_eq!(view.files[0].hash, [1u8; 32]);
        assert_eq!(view.files[0].source_device, "dev-a");
    }

    #[test]
    fn lww_picks_newer_publisher_for_same_path() {
        let r_old = dev_root(100);
        let r_new = dev_root(200);
        let view = merge_drives(
            &["dev-a", "dev-b"],
            &[
                snap("dev-a", &r_old, vec![file("x", 1, 1)], vec![]),
                snap("dev-b", &r_new, vec![file("x", 2, 1)], vec![]),
            ],
        );
        assert_eq!(view.files.len(), 1);
        assert_eq!(view.files[0].source_device, "dev-b");
        assert_eq!(view.files[0].hash, [2u8; 32]);
    }

    #[test]
    fn causal_descendant_wins_even_with_older_wall_clock() {
        let r_a = causal_root("cid-a", 300, 1, &[]);
        let r_b = causal_root("cid-b", 100, 1, &[("dev-a", 1, "cid-a")]);
        let view = merge_drives(
            &["dev-a", "dev-b"],
            &[
                snap("dev-a", &r_a, vec![file("x", 1, 1)], vec![]),
                snap("dev-b", &r_b, vec![file("x", 2, 1)], vec![]),
            ],
        );
        assert_eq!(view.files.len(), 1);
        assert_eq!(view.files[0].source_device, "dev-b");
        assert_eq!(view.files[0].hash, [2u8; 32]);
        assert!(view.conflicts.is_empty());
    }

    #[test]
    fn observed_same_sequence_with_different_root_is_not_descendant() {
        let r_a = causal_root("cid-a", 300, 1, &[]);
        let r_b = causal_root("cid-b", 100, 1, &[("dev-a", 1, "cid-a-fork")]);
        let view = merge_drives(
            &["dev-a", "dev-b"],
            &[
                snap("dev-a", &r_a, vec![file("x", 1, 1)], vec![]),
                snap("dev-b", &r_b, vec![file("x", 2, 1)], vec![]),
            ],
        );
        assert_eq!(view.files.len(), 1);
        assert_eq!(
            view.files[0].source_device, "dev-a",
            "timestamp fallback should win when ancestry is unknown"
        );
        assert_eq!(view.conflicts, vec!["x".to_string()]);
    }

    #[test]
    fn concurrent_different_writes_are_marked_as_conflicts() {
        let r_a = causal_root("cid-a", 100, 1, &[]);
        let r_b = causal_root("cid-b", 200, 1, &[]);
        let view = merge_drives(
            &["dev-a", "dev-b"],
            &[
                snap("dev-a", &r_a, vec![file("x", 1, 1)], vec![]),
                snap("dev-b", &r_b, vec![file("x", 2, 1)], vec![]),
            ],
        );
        assert_eq!(view.files.len(), 1);
        assert_eq!(view.conflicts, vec!["x".to_string()]);
    }

    #[test]
    fn concurrent_same_content_converges_without_conflict() {
        let r_a = causal_root("cid-a", 100, 1, &[]);
        let r_b = causal_root("cid-b", 200, 1, &[]);
        let view = merge_drives(
            &["dev-a", "dev-b"],
            &[
                snap("dev-a", &r_a, vec![file("x", 1, 1)], vec![]),
                snap("dev-b", &r_b, vec![file("x", 1, 1)], vec![]),
            ],
        );
        assert_eq!(view.files.len(), 1);
        assert!(view.conflicts.is_empty());
    }

    #[test]
    fn causal_tombstone_suppresses_observed_write_even_with_older_clock() {
        let r_a = causal_root("cid-a", 300, 1, &[]);
        let r_b = causal_root("cid-b", 100, 1, &[("dev-a", 1, "cid-a")]);
        let view = merge_drives(
            &["dev-a", "dev-b"],
            &[
                snap("dev-a", &r_a, vec![file("x", 1, 1)], vec![]),
                snap("dev-b", &r_b, vec![], vec![tomb("x", 100)]),
            ],
        );
        assert!(view.files.is_empty());
        assert_eq!(view.suppressed_by_tombstone, vec!["x".to_string()]);
        assert!(view.conflicts.is_empty());
    }

    #[test]
    fn concurrent_write_delete_is_marked_as_conflict() {
        let r_a = causal_root("cid-a", 100, 1, &[]);
        let r_b = causal_root("cid-b", 200, 1, &[]);
        let view = merge_drives(
            &["dev-a", "dev-b"],
            &[
                snap("dev-a", &r_a, vec![file("x", 1, 1)], vec![]),
                snap("dev-b", &r_b, vec![], vec![tomb("x", 200)]),
            ],
        );
        assert_eq!(view.conflicts, vec!["x".to_string()]);
    }

    #[test]
    fn disjoint_paths_all_appear() {
        let r_a = dev_root(100);
        let r_b = dev_root(200);
        let view = merge_drives(
            &["dev-a", "dev-b"],
            &[
                snap("dev-a", &r_a, vec![file("a.txt", 1, 1)], vec![]),
                snap("dev-b", &r_b, vec![file("b.txt", 2, 1)], vec![]),
            ],
        );
        assert_eq!(view.files.len(), 2);
        assert_eq!(view.files[0].path, "a.txt");
        assert_eq!(view.files[1].path, "b.txt");
    }

    #[test]
    fn newer_tombstone_suppresses_older_write() {
        let r_a = dev_root(100);
        let r_b = dev_root(200);
        let view = merge_drives(
            &["dev-a", "dev-b"],
            &[
                snap("dev-a", &r_a, vec![file("x", 1, 1)], vec![]),
                snap("dev-b", &r_b, vec![], vec![tomb("x", 200)]),
            ],
        );
        assert!(view.files.is_empty());
        assert_eq!(view.suppressed_by_tombstone, vec!["x".to_string()]);
    }

    #[test]
    fn newer_write_resurrects_after_older_tombstone() {
        let r_old = dev_root(100);
        let r_new = dev_root(200);
        let view = merge_drives(
            &["dev-a", "dev-b"],
            &[
                snap("dev-a", &r_old, vec![], vec![tomb("x", 100)]),
                snap("dev-b", &r_new, vec![file("x", 2, 1)], vec![]),
            ],
        );
        assert_eq!(view.files.len(), 1);
        assert_eq!(view.files[0].source_device, "dev-b");
    }

    #[test]
    fn same_timestamp_tombstone_wins_over_write() {
        // Deletion is conservative on ties.
        let r = dev_root(100);
        let view = merge_drives(
            &["dev-a", "dev-b"],
            &[
                snap("dev-a", &r, vec![file("x", 1, 1)], vec![]),
                snap("dev-b", &r, vec![], vec![tomb("x", 100)]),
            ],
        );
        assert!(view.files.is_empty());
        assert_eq!(view.suppressed_by_tombstone, vec!["x".to_string()]);
    }

    #[test]
    fn three_devices_converge() {
        let r_a = dev_root(100);
        let r_b = dev_root(200);
        let r_c = dev_root(300);
        let view = merge_drives(
            &["dev-a", "dev-b", "dev-c"],
            &[
                snap(
                    "dev-a",
                    &r_a,
                    vec![file("alpha", 1, 1), file("contested", 1, 1)],
                    vec![],
                ),
                snap(
                    "dev-b",
                    &r_b,
                    vec![file("beta", 2, 1), file("contested", 2, 1)],
                    vec![],
                ),
                snap(
                    "dev-c",
                    &r_c,
                    vec![file("gamma", 3, 1), file("contested", 3, 1)],
                    vec![],
                ),
            ],
        );
        assert_eq!(view.files.len(), 4);
        let paths: Vec<&str> = view.files.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["alpha", "beta", "contested", "gamma"]);
        // Contested file resolved to dev-c (latest).
        let contested = view.files.iter().find(|e| e.path == "contested").unwrap();
        assert_eq!(contested.source_device, "dev-c");
    }

    #[test]
    fn tombstone_in_one_device_wipes_across_others_when_newer() {
        let r_a = dev_root(100);
        let r_b = dev_root(150);
        let r_c = dev_root(200);
        let view = merge_drives(
            &["dev-a", "dev-b", "dev-c"],
            &[
                snap("dev-a", &r_a, vec![file("shared", 1, 1)], vec![]),
                snap("dev-b", &r_b, vec![file("shared", 2, 1)], vec![]),
                snap("dev-c", &r_c, vec![], vec![tomb("shared", 200)]),
            ],
        );
        assert!(view.files.is_empty());
        assert_eq!(view.suppressed_by_tombstone, vec!["shared".to_string()]);
    }

    #[test]
    fn output_is_sorted_lexicographic() {
        let r = dev_root(100);
        let view = merge_drives(
            &["dev-a"],
            &[snap(
                "dev-a",
                &r,
                vec![file("zeta", 1, 1), file("alpha", 2, 1), file("mid", 3, 1)],
                vec![],
            )],
        );
        let paths: Vec<&str> = view.files.iter().map(|e| e.path.as_str()).collect();
        assert_eq!(paths, vec!["alpha", "mid", "zeta"]);
    }

    #[test]
    fn tombstone_path_round_trip() {
        let original = "Photos/IMG_001.heic";
        let encoded = tombstone_path(original);
        assert_eq!(encoded, ".hashtree/tombstones/Photos/IMG_001.heic");
        assert_eq!(original_path_from_tombstone(&encoded), Some(original));
    }

    #[test]
    fn original_path_from_non_tombstone_is_none() {
        assert!(original_path_from_tombstone("notes.txt").is_none());
        assert!(original_path_from_tombstone(".hashtree/tombstones").is_none());
    }
}
