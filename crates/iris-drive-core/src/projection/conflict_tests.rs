use super::*;
use crate::config::{AppConfig, AppKeyRootRef, Drive};
use crate::indexer::index_dir_with_history_and_meta;
use crate::merge::{
    MergedConflict, MergedConflictFile, MergedConflictKind, MergedEntry, MergedView,
};
use crate::profile::Profile;
use crate::root_meta::DriveRootMeta;
use hashtree_core::{HashTreeConfig, MemoryStore, to_hex};
use std::sync::Arc;
use tempfile::tempdir;

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

#[tokio::test]
async fn root_republish_times_do_not_change_concurrent_conflict_projection() {
    let config_dir = tempdir().unwrap();
    let mut profile = Profile::create(config_dir.path(), Some("owner".into())).unwrap();
    let peer = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
    profile.approve_app_key(peer, Some("peer".into())).unwrap();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

    let owner_root =
        indexed_conflict_root(&tree, &profile.state.app_key_pubkey, b"owner edit", 1, 10).await;
    let peer_root = indexed_conflict_root(&tree, peer, b"peer edit", 1, 20).await;

    let mut first = conflict_projection_config(&profile, peer, &owner_root, &peer_root);
    let mut second = first.clone();
    let first_drive = first
        .drives
        .iter_mut()
        .find(|drive| drive.drive_id == PRIMARY_DRIVE_ID)
        .unwrap();
    first_drive
        .app_key_roots
        .get_mut(&profile.state.app_key_pubkey)
        .unwrap()
        .published_at = 100;
    first_drive
        .app_key_roots
        .get_mut(peer)
        .unwrap()
        .published_at = 300;
    let second_drive = second
        .drives
        .iter_mut()
        .find(|drive| drive.drive_id == PRIMARY_DRIVE_ID)
        .unwrap();
    second_drive
        .app_key_roots
        .get_mut(&profile.state.app_key_pubkey)
        .unwrap()
        .published_at = 400;
    second_drive
        .app_key_roots
        .get_mut(peer)
        .unwrap()
        .published_at = 200;

    let first_view = primary_merged_view(&tree, &first).await.unwrap();
    let second_view = primary_merged_view(&tree, &second).await.unwrap();

    assert_eq!(first_view.view.files, second_view.view.files);
}

async fn indexed_conflict_root(
    tree: &HashTree<MemoryStore>,
    app_key_pubkey: &str,
    bytes: &[u8],
    app_key_seq: u64,
    created_at: i64,
) -> (Cid, DriveRootMeta) {
    let source = tempdir().unwrap();
    std::fs::write(source.path().join("note.txt"), bytes).unwrap();
    let meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: app_key_pubkey.to_string(),
        app_key_seq,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at,
    };
    let root = index_dir_with_history_and_meta(tree, source.path(), None, created_at, Some(&meta))
        .await
        .unwrap();
    (root, meta)
}

fn conflict_projection_config(
    profile: &Profile,
    peer: &str,
    owner_root: &(Cid, DriveRootMeta),
    peer_root: &(Cid, DriveRootMeta),
) -> AppConfig {
    let mut config = AppConfig {
        profile: Some(profile.state.clone()),
        ..AppConfig::default()
    };
    let mut drive = Drive::primary(profile.state.root_scope_id());
    drive.app_key_roots.insert(
        profile.state.app_key_pubkey.clone(),
        AppKeyRootRef::from_meta(
            owner_root.0.to_string(),
            owner_root.1.created_at,
            &owner_root.1,
        ),
    );
    drive.app_key_roots.insert(
        peer.to_string(),
        AppKeyRootRef::from_meta(
            peer_root.0.to_string(),
            peer_root.1.created_at,
            &peer_root.1,
        ),
    );
    config.upsert_drive(drive);
    config
}
