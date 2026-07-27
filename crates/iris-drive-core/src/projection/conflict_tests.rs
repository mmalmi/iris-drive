use super::*;
use crate::merge::{
    MergedConflict, MergedConflictFile, MergedConflictKind, MergedEntry, MergedView,
};
use hashtree_core::to_hex;

fn entry(app_key_pubkey: &str, hash: u8) -> MergedEntry {
    MergedEntry {
        path: "docs/note.txt".to_string(),
        source_path: None,
        hash: [hash; 32],
        size: 4,
        whole_file_hash: None,
        modified_at: None,
        source_app_key_pubkey: app_key_pubkey.to_string(),
        published_at: i64::from(hash),
    }
}

fn conflict_file(app_key_pubkey: &str, hash: u8) -> MergedConflictFile {
    MergedConflictFile {
        app_key_pubkey: app_key_pubkey.to_string(),
        app_key_seq: 1,
        root_cid: format!("root-{hash}"),
        published_at: i64::from(hash),
        content_hash: to_hex(&[hash; 32]),
        content_cid_hash: to_hex(&[hash; 32]),
        size: 4,
        modified_at: None,
    }
}

#[test]
fn visible_write_conflicts_choose_the_same_winner_from_stable_provenance() {
    let low_key = "1111111111111111111111111111111111111111111111111111111111111111";
    let high_key = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    let conflict = MergedConflict {
        path: "docs/note.txt".to_string(),
        kind: MergedConflictKind::WriteWrite,
        files: vec![conflict_file(low_key, 1), conflict_file(high_key, 2)],
        tombstone: None,
    };
    let mut low_winner = MergedView {
        files: vec![entry(low_key, 1)],
        conflicts: vec!["docs/note.txt".to_string()],
        conflict_details: vec![conflict.clone()],
        ..MergedView::default()
    };
    let mut high_winner = MergedView {
        files: vec![entry(high_key, 2)],
        conflicts: vec!["docs/note.txt".to_string()],
        conflict_details: vec![conflict],
        ..MergedView::default()
    };

    add_visible_conflict_entries(&mut low_winner).unwrap();
    add_visible_conflict_entries(&mut high_winner).unwrap();

    assert_eq!(low_winner.files, high_winner.files);
    assert_eq!(
        low_winner
            .files
            .iter()
            .find(|entry| entry.path == "docs/note.txt")
            .unwrap()
            .source_app_key_pubkey,
        high_key
    );
}
