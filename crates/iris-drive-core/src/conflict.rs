//! File-level conflict resolution.
//!
//! Iris Drive uses last-writer-wins by published timestamp with a rename
//! conflict copy on irreconcilable conflicts — same model Drive and Dropbox
//! ship. CRDTs are not used in v1.
//!
//! `resolve` is a pure function: given snapshots of (base, local, remote)
//! for the same path, it returns the action the sync engine should take.
//! All I/O is the caller's problem; this keeps the algorithm testable.

use hashtree_core::{sha256, to_hex};
use serde::{Deserialize, Serialize};

use crate::merge::{
    MergedConflict, MergedConflictFile, MergedConflictKind, MergedConflictTombstone,
};

/// One side of a durable conflict record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictSide {
    pub app_key_pubkey: String,
    pub app_key_seq: u64,
    pub root_cid: String,
    pub whole_file_hash: String,
}

/// Deleted side of a durable write/delete conflict record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictDeletedSide {
    pub app_key_pubkey: String,
    pub app_key_seq: u64,
    pub root_cid: String,
    pub tombstoned_at: i64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote: Option<ConflictSide>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<ConflictDeletedSide>,
    pub state: ConflictState,
    pub created_at: i64,
}

impl ConflictRecord {
    pub const SCHEMA: u32 = 1;
}

/// Build durable conflict records from merge provenance. The first file
/// side in deterministic ordering is treated as the original-path side;
/// each other file or tombstone becomes one record.
#[must_use]
pub fn conflict_records_from_merge(
    conflict: &MergedConflict,
    created_at: i64,
) -> Vec<ConflictRecord> {
    let mut files = conflict.files.clone();
    files.sort_by(|a, b| {
        a.app_key_pubkey
            .cmp(&b.app_key_pubkey)
            .then(a.app_key_seq.cmp(&b.app_key_seq))
            .then(a.root_cid.cmp(&b.root_cid))
            .then(a.content_hash.cmp(&b.content_hash))
    });
    let Some(local_file) = files.first() else {
        return Vec::new();
    };
    let local = conflict_side_from_file(local_file);

    match conflict.kind {
        MergedConflictKind::WriteWrite => files
            .iter()
            .skip(1)
            .map(|remote_file| {
                let remote = conflict_side_from_file(remote_file);
                ConflictRecord {
                    schema: ConflictRecord::SCHEMA,
                    conflict_id: conflict_id(&conflict.path, &local, Some(&remote), None),
                    path: conflict.path.clone(),
                    visible_conflict_path: conflict_filename(
                        &conflict.path,
                        &remote.app_key_pubkey,
                    ),
                    local: local.clone(),
                    remote: Some(remote),
                    deleted: None,
                    state: ConflictState::Unresolved,
                    created_at,
                }
            })
            .collect(),
        MergedConflictKind::WriteDelete => conflict
            .tombstone
            .as_ref()
            .map(|tombstone| {
                let deleted = conflict_deleted_side_from_tombstone(tombstone);
                ConflictRecord {
                    schema: ConflictRecord::SCHEMA,
                    conflict_id: conflict_id(&conflict.path, &local, None, Some(&deleted)),
                    path: conflict.path.clone(),
                    visible_conflict_path: conflict_filename(&conflict.path, &local.app_key_pubkey),
                    local,
                    remote: None,
                    deleted: Some(deleted),
                    state: ConflictState::Unresolved,
                    created_at,
                }
            })
            .into_iter()
            .collect(),
    }
}

fn conflict_side_from_file(file: &MergedConflictFile) -> ConflictSide {
    ConflictSide {
        app_key_pubkey: file.app_key_pubkey.clone(),
        app_key_seq: file.app_key_seq,
        root_cid: file.root_cid.clone(),
        whole_file_hash: file.content_hash.clone(),
    }
}

fn conflict_deleted_side_from_tombstone(
    tombstone: &MergedConflictTombstone,
) -> ConflictDeletedSide {
    ConflictDeletedSide {
        app_key_pubkey: tombstone.app_key_pubkey.clone(),
        app_key_seq: tombstone.app_key_seq,
        root_cid: tombstone.root_cid.clone(),
        tombstoned_at: tombstone.tombstoned_at,
    }
}

fn conflict_id(
    path: &str,
    local: &ConflictSide,
    remote: Option<&ConflictSide>,
    deleted: Option<&ConflictDeletedSide>,
) -> String {
    let peer = if let Some(remote) = remote {
        format!(
            "file|{}|{}|{}|{}",
            remote.app_key_pubkey, remote.app_key_seq, remote.root_cid, remote.whole_file_hash
        )
    } else if let Some(deleted) = deleted {
        format!(
            "deleted|{}|{}|{}|{}",
            deleted.app_key_pubkey, deleted.app_key_seq, deleted.root_cid, deleted.tombstoned_at
        )
    } else {
        "none".to_string()
    };
    let seed = format!(
        "{}|{}|{}|{}|{}|{}|{}",
        ConflictRecord::SCHEMA,
        path,
        local.app_key_pubkey,
        local.app_key_seq,
        local.root_cid,
        local.whole_file_hash,
        peer
    );
    to_hex(&sha256(seed.as_bytes()))
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
    /// Local deleted, remote still matches the base. Propagate the local
    /// deletion to the remote.
    DeleteRemote,
    /// Both sides changed; preserve both. The local stays at its
    /// current path; the remote is renamed to `conflict_name`.
    Conflict {
        remote: FileSnapshot,
        conflict_name: String,
    },
}

/// Resolve one file's state.
///
/// `app_key_label` is a short string identifying the device whose copy is
/// being renamed in a conflict (e.g. "macbook"). It does not influence
/// non-conflict outcomes.
#[must_use]
pub fn resolve(
    path: &str,
    base: Option<&FileSnapshot>,
    local: Option<&FileSnapshot>,
    remote: Option<&FileSnapshot>,
    app_key_label: &str,
) -> SyncAction {
    match (base, local, remote) {
        // Both sides absent — nothing to sync, regardless of base.
        (_, None, None) => SyncAction::NoOp,
        (None, None, Some(r)) => SyncAction::ApplyRemote { new: r.clone() },
        (None, Some(_), None) => SyncAction::KeepLocal,
        (None, Some(l), Some(r)) => same_or_conflict(l, r, path, app_key_label),
        (Some(b), None, Some(r)) => {
            if r.content_hash == b.content_hash {
                // Remote is still at base; we (locally) deleted.
                SyncAction::DeleteRemote
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
                (true, true) => same_or_conflict(l, r, path, app_key_label),
            }
        }
    }
}

fn same_or_conflict(
    local: &FileSnapshot,
    remote: &FileSnapshot,
    path: &str,
    app_key_label: &str,
) -> SyncAction {
    if local.content_hash == remote.content_hash {
        return SyncAction::NoOp;
    }
    // Pick the newer to mention in the conflict-rename label, mirroring
    // Drive's "Alice's conflicted copy" phrasing.
    SyncAction::Conflict {
        remote: remote.clone(),
        conflict_name: conflict_filename(path, app_key_label),
    }
}

/// Produce a Dropbox-style conflict filename: `name (conflict from X).ext`.
#[must_use]
pub fn conflict_filename(original: &str, app_key_label: &str) -> String {
    const MAX_COMPONENT_BYTES: usize = 240;

    let (dir, name) = split_dir_name(original);
    let (stem, ext) = split_stem_ext(name);
    let ext = truncate_utf8(ext, ext.len().min(48));
    let marker_prefix = " (conflict from ";
    let marker_suffix = ")";
    let fixed_bytes = marker_prefix.len() + marker_suffix.len() + ext.len();
    let available = MAX_COMPONENT_BYTES.saturating_sub(fixed_bytes);
    let label_budget = app_key_label.len().min(available.saturating_sub(1));
    let label = truncate_utf8(app_key_label, label_budget);
    let stem_budget = available.saturating_sub(label.len());
    let stem = truncate_utf8(stem, stem_budget);

    if ext.is_empty() {
        format!("{dir}{stem}{marker_prefix}{label}{marker_suffix}")
    } else {
        format!("{dir}{stem}{marker_prefix}{label}{marker_suffix}{ext}")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedConflictFilename {
    pub original_path: String,
    pub app_key_label: String,
}

/// Parse a filename generated by [`conflict_filename`].
#[must_use]
pub fn parse_conflict_filename(path: &str) -> Option<ParsedConflictFilename> {
    const PREFIX: &str = " (conflict from ";
    const SUFFIX: &str = ")";

    let (dir, name) = split_dir_name(path);
    let (stem, ext) = split_stem_ext(name);
    let without_suffix = stem.strip_suffix(SUFFIX)?;
    let marker = without_suffix.rfind(PREFIX)?;
    let original_stem = &without_suffix[..marker];
    let app_key_label = &without_suffix[marker + PREFIX.len()..];
    if original_stem.is_empty() || app_key_label.is_empty() {
        return None;
    }

    Some(ParsedConflictFilename {
        original_path: format!("{dir}{original_stem}{ext}"),
        app_key_label: app_key_label.to_string(),
    })
}

fn split_dir_name(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(idx) => (&path[..=idx], &path[idx + 1..]),
        None => ("", path),
    }
}

fn split_stem_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(idx) if idx > 0 && idx < name.len() - 1 => {
            (&name[..idx], &name[idx..]) // ext includes the dot
        }
        _ => (name, ""),
    }
}

fn truncate_utf8(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn merged_file(app_key_pubkey: &str, seq: u64, root: &str, hash: &str) -> MergedConflictFile {
        MergedConflictFile {
            app_key_pubkey: app_key_pubkey.into(),
            app_key_seq: seq,
            root_cid: root.into(),
            content_hash: hash.into(),
            content_cid_hash: format!("cid-{hash}"),
            size: 10,
            modified_at: None,
        }
    }

    fn snap(hash: &str, mtime: i64) -> FileSnapshot {
        FileSnapshot {
            content_hash: hash.into(),
            mtime,
        }
    }

    #[test]
    fn write_write_merge_conflict_builds_durable_record() {
        let conflict = MergedConflict {
            path: "report.pdf".into(),
            kind: MergedConflictKind::WriteWrite,
            files: vec![
                merged_file("dev-a", 2, "cid-a", &"aa".repeat(32)),
                merged_file("dev-b", 7, "cid-b", &"bb".repeat(32)),
            ],
            tombstone: None,
        };

        let records = conflict_records_from_merge(&conflict, 1234);
        assert_eq!(records, conflict_records_from_merge(&conflict, 1234));
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.schema, ConflictRecord::SCHEMA);
        assert!(!record.conflict_id.contains('/'));
        assert_eq!(record.path, "report.pdf");
        assert_eq!(
            record.visible_conflict_path,
            "report (conflict from dev-b).pdf"
        );
        assert_eq!(record.local.app_key_pubkey, "dev-a");
        assert_eq!(record.remote.as_ref().unwrap().app_key_pubkey, "dev-b");
        assert!(record.deleted.is_none());
        assert_eq!(record.state, ConflictState::Unresolved);
        assert_eq!(record.created_at, 1234);
    }

    #[test]
    fn write_delete_merge_conflict_records_deleted_side() {
        let conflict = MergedConflict {
            path: "report.pdf".into(),
            kind: MergedConflictKind::WriteDelete,
            files: vec![merged_file("dev-a", 2, "cid-a", &"aa".repeat(32))],
            tombstone: Some(MergedConflictTombstone {
                app_key_pubkey: "dev-b".into(),
                app_key_seq: 7,
                root_cid: "cid-b".into(),
                tombstoned_at: 555,
            }),
        };

        let records = conflict_records_from_merge(&conflict, 1234);
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(
            record.visible_conflict_path,
            "report (conflict from dev-a).pdf"
        );
        assert_eq!(record.local.app_key_pubkey, "dev-a");
        assert!(record.remote.is_none());
        let deleted = record.deleted.as_ref().unwrap();
        assert_eq!(deleted.app_key_pubkey, "dev-b");
        assert_eq!(deleted.app_key_seq, 7);
        assert_eq!(deleted.root_cid, "cid-b");
        assert_eq!(deleted.tombstoned_at, 555);
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
    fn local_delete_remote_unchanged_deletes_remote() {
        let b = snap("base", 1);
        let r = snap("base", 1);
        assert_eq!(
            resolve("x", Some(&b), None, Some(&r), "dev"),
            SyncAction::DeleteRemote
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

    #[test]
    fn conflict_filename_ignores_dots_in_parent_dirs() {
        assert_eq!(
            conflict_filename("docs.v1/README", "phone"),
            "docs.v1/README (conflict from phone)"
        );
    }

    #[test]
    fn conflict_filename_caps_component_length() {
        let name = format!("{}.txt", "a".repeat(260));
        let path = conflict_filename(&name, &"d".repeat(64));
        let component = path.rsplit('/').next().unwrap();
        assert!(component.len() <= 240, "{component}");
        assert_eq!(
            std::path::Path::new(component)
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("txt")
        );
        assert!(component.contains(" (conflict from "));
    }

    #[test]
    fn conflict_filename_truncates_at_utf8_boundary() {
        let path = conflict_filename(&format!("{}.txt", "å".repeat(180)), "phone");
        let component = path.rsplit('/').next().unwrap();
        assert!(component.len() <= 240, "{component}");
        assert_eq!(
            std::path::Path::new(component)
                .extension()
                .and_then(|extension| extension.to_str()),
            Some("txt")
        );
    }

    #[test]
    fn parses_generated_conflict_filename() {
        let parsed = parse_conflict_filename("docs/report (conflict from phone).pdf").unwrap();
        assert_eq!(parsed.original_path, "docs/report.pdf");
        assert_eq!(parsed.app_key_label, "phone");
    }

    #[test]
    fn parses_generated_conflict_dotfile() {
        let parsed = parse_conflict_filename(".env (conflict from phone)").unwrap();
        assert_eq!(parsed.original_path, ".env");
        assert_eq!(parsed.app_key_label, "phone");
    }

    #[test]
    fn rejects_plain_or_malformed_conflict_filename() {
        assert!(parse_conflict_filename("report.pdf").is_none());
        assert!(parse_conflict_filename("report (conflict from ).pdf").is_none());
        assert!(parse_conflict_filename("report (conflict from phone.pdf").is_none());
    }
}
