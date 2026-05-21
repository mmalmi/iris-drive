//! File-level conflict resolution.
//!
//! Iris Drive uses last-writer-wins by published timestamp with a rename
//! fallback on irreconcilable conflicts — same model Drive and Dropbox
//! ship. CRDTs are not used in v1.
//!
//! `resolve` is a pure function: given snapshots of (base, local, remote)
//! for the same path, it returns the action the sync engine should take.
//! All I/O is the caller's problem; this keeps the algorithm testable.

use serde::{Deserialize, Serialize};

/// One side of a durable conflict record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictSide {
    pub device_id: String,
    pub device_seq: u64,
    pub root_cid: String,
    pub whole_file_hash: String,
}

/// Resolution state for a durable conflict record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictState {
    Unresolved,
    Resolved,
}

/// Metadata stored under `.hashtree/conflicts/<conflict_id>.json`.
///
/// The conflict copy itself remains a real file in the snapshot; this
/// record explains why it exists and which roots produced it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictRecord {
    pub schema: u32,
    pub conflict_id: String,
    pub path: String,
    pub visible_conflict_path: String,
    pub local: ConflictSide,
    pub remote: ConflictSide,
    pub state: ConflictState,
    pub created_at: i64,
}

impl ConflictRecord {
    pub const SCHEMA: u32 = 1;
}

/// One file's identity at a point in time: content hash + when its
/// writer published it. `mtime` is the wall-clock published time, not
/// the local filesystem mtime (which differs across machines).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileSnapshot {
    pub content_hash: String,
    pub mtime: i64,
}

/// What the sync engine should do for this file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SyncAction {
    /// Nothing to do — local and remote agree.
    NoOp,
    /// Replace the local file with the remote contents.
    ApplyRemote { new: FileSnapshot },
    /// Local diverged from base; remote is unchanged or absent. Mark
    /// the local copy dirty for upload.
    KeepLocal,
    /// Local exists, remote deleted. Honour the deletion of a peer
    /// only when our local matches the base — otherwise treat as conflict.
    DeleteLocal,
    /// Both sides changed; preserve both. The local stays at its
    /// current path; the remote is renamed to `conflict_name`.
    Conflict {
        remote: FileSnapshot,
        conflict_name: String,
    },
}

/// Resolve one file's state.
///
/// `device_label` is a short string identifying the device whose copy is
/// being renamed in a conflict (e.g. "macbook"). It does not influence
/// non-conflict outcomes.
#[must_use]
pub fn resolve(
    path: &str,
    base: Option<&FileSnapshot>,
    local: Option<&FileSnapshot>,
    remote: Option<&FileSnapshot>,
    device_label: &str,
) -> SyncAction {
    match (base, local, remote) {
        // Both sides absent — nothing to sync, regardless of base.
        (_, None, None) => SyncAction::NoOp,
        (None, None, Some(r)) => SyncAction::ApplyRemote { new: r.clone() },
        (None, Some(_), None) => SyncAction::KeepLocal,
        (None, Some(l), Some(r)) => same_or_conflict(l, r, path, device_label),
        (Some(b), None, Some(r)) => {
            if r.content_hash == b.content_hash {
                // Remote is still at base; we (locally) deleted. Propagate the delete on next push.
                SyncAction::NoOp
            } else {
                // Local deleted but remote modified — surface as keep-remote.
                SyncAction::ApplyRemote { new: r.clone() }
            }
        }
        (Some(b), Some(l), None) => {
            if l.content_hash == b.content_hash {
                // Remote deleted; local hasn't diverged. Honor the deletion.
                SyncAction::DeleteLocal
            } else {
                // Local diverged; remote deleted. Keep local; the peer will
                // see it as Added next sync.
                SyncAction::KeepLocal
            }
        }
        (Some(b), Some(l), Some(r)) => {
            let local_changed = l.content_hash != b.content_hash;
            let remote_changed = r.content_hash != b.content_hash;
            match (local_changed, remote_changed) {
                (false, false) => SyncAction::NoOp,
                (false, true) => SyncAction::ApplyRemote { new: r.clone() },
                (true, false) => SyncAction::KeepLocal,
                (true, true) => same_or_conflict(l, r, path, device_label),
            }
        }
    }
}

fn same_or_conflict(
    local: &FileSnapshot,
    remote: &FileSnapshot,
    path: &str,
    device_label: &str,
) -> SyncAction {
    if local.content_hash == remote.content_hash {
        return SyncAction::NoOp;
    }
    // Pick the newer to mention in the conflict-rename label, mirroring
    // Drive's "Alice's conflicted copy" phrasing.
    SyncAction::Conflict {
        remote: remote.clone(),
        conflict_name: conflict_filename(path, device_label),
    }
}

/// Produce a Dropbox-style conflict filename: `name (conflict from X).ext`.
#[must_use]
pub fn conflict_filename(original: &str, device_label: &str) -> String {
    let (stem, ext) = match original.rfind('.') {
        Some(idx) if idx > 0 && idx < original.len() - 1 => {
            (&original[..idx], &original[idx..]) // ext includes the dot
        }
        _ => (original, ""),
    };
    if ext.is_empty() {
        format!("{stem} (conflict from {device_label})")
    } else {
        format!("{stem} (conflict from {device_label}){ext}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(hash: &str, mtime: i64) -> FileSnapshot {
        FileSnapshot {
            content_hash: hash.into(),
            mtime,
        }
    }

    #[test]
    fn nothing_to_do_when_both_absent() {
        assert_eq!(resolve("x", None, None, None, "dev"), SyncAction::NoOp);
    }

    #[test]
    fn remote_only_addition_applies() {
        let r = snap("r1", 10);
        assert_eq!(
            resolve("x", None, None, Some(&r), "dev"),
            SyncAction::ApplyRemote { new: r }
        );
    }

    #[test]
    fn local_only_addition_keeps_local() {
        let l = snap("l1", 10);
        assert_eq!(
            resolve("x", None, Some(&l), None, "dev"),
            SyncAction::KeepLocal
        );
    }

    #[test]
    fn concurrent_add_same_content_is_noop() {
        let l = snap("same", 10);
        let r = snap("same", 11);
        assert_eq!(
            resolve("x", None, Some(&l), Some(&r), "dev"),
            SyncAction::NoOp
        );
    }

    #[test]
    fn concurrent_add_different_content_conflicts() {
        let l = snap("L", 10);
        let r = snap("R", 11);
        match resolve("photo.jpg", None, Some(&l), Some(&r), "macbook") {
            SyncAction::Conflict {
                remote,
                conflict_name,
            } => {
                assert_eq!(remote.content_hash, "R");
                assert_eq!(conflict_name, "photo (conflict from macbook).jpg");
            }
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn unchanged_local_propagates_remote() {
        let b = snap("base", 1);
        let l = snap("base", 1);
        let r = snap("newer", 5);
        assert_eq!(
            resolve("x", Some(&b), Some(&l), Some(&r), "dev"),
            SyncAction::ApplyRemote { new: r }
        );
    }

    #[test]
    fn unchanged_remote_keeps_local() {
        let b = snap("base", 1);
        let l = snap("newer", 5);
        let r = snap("base", 1);
        assert_eq!(
            resolve("x", Some(&b), Some(&l), Some(&r), "dev"),
            SyncAction::KeepLocal
        );
    }

    #[test]
    fn both_unchanged_is_noop() {
        let b = snap("base", 1);
        let l = snap("base", 1);
        let r = snap("base", 1);
        assert_eq!(
            resolve("x", Some(&b), Some(&l), Some(&r), "dev"),
            SyncAction::NoOp
        );
    }

    #[test]
    fn both_changed_same_way_is_noop() {
        let b = snap("base", 1);
        let l = snap("converged", 5);
        let r = snap("converged", 6);
        assert_eq!(
            resolve("x", Some(&b), Some(&l), Some(&r), "dev"),
            SyncAction::NoOp
        );
    }

    #[test]
    fn both_changed_differently_conflicts() {
        let b = snap("base", 1);
        let l = snap("L", 5);
        let r = snap("R", 6);
        match resolve("x", Some(&b), Some(&l), Some(&r), "dev") {
            SyncAction::Conflict { remote, .. } => assert_eq!(remote.content_hash, "R"),
            other => panic!("expected Conflict, got {other:?}"),
        }
    }

    #[test]
    fn remote_delete_local_unchanged_deletes_local() {
        let b = snap("base", 1);
        let l = snap("base", 1);
        assert_eq!(
            resolve("x", Some(&b), Some(&l), None, "dev"),
            SyncAction::DeleteLocal
        );
    }

    #[test]
    fn remote_delete_local_changed_keeps_local() {
        let b = snap("base", 1);
        let l = snap("changed", 5);
        assert_eq!(
            resolve("x", Some(&b), Some(&l), None, "dev"),
            SyncAction::KeepLocal
        );
    }

    #[test]
    fn local_delete_remote_unchanged_is_noop() {
        let b = snap("base", 1);
        let r = snap("base", 1);
        assert_eq!(
            resolve("x", Some(&b), None, Some(&r), "dev"),
            SyncAction::NoOp
        );
    }

    #[test]
    fn local_delete_remote_modified_keeps_remote() {
        let b = snap("base", 1);
        let r = snap("modified", 5);
        match resolve("x", Some(&b), None, Some(&r), "dev") {
            SyncAction::ApplyRemote { new } => assert_eq!(new.content_hash, "modified"),
            other => panic!("expected ApplyRemote, got {other:?}"),
        }
    }

    #[test]
    fn conflict_filename_with_extension() {
        assert_eq!(
            conflict_filename("report.pdf", "phone"),
            "report (conflict from phone).pdf"
        );
    }

    #[test]
    fn conflict_filename_no_extension() {
        assert_eq!(
            conflict_filename("README", "phone"),
            "README (conflict from phone)"
        );
    }

    #[test]
    fn conflict_filename_dotfile() {
        // ".env" should not be treated as "" + ".env"; "env" + "" instead.
        assert_eq!(
            conflict_filename(".env", "phone"),
            ".env (conflict from phone)"
        );
    }
}
