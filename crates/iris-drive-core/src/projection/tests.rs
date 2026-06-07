#[allow(clippy::wildcard_imports)]
use super::*;
use crate::app_keys::AppActorEntry;
use crate::config::{AppConfig, AppKeyRootRef, Drive};
use crate::indexer::index_dir_with_history_and_meta;
use crate::merge::AppKeyFileEntry;
use crate::profile::Profile;
use crate::root_meta::{DriveRootMeta, RootObservation, RootParent};
use hashtree_core::{DirEntry, HashTreeConfig, LinkType, MemoryStore, sha256};
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
    let local_entry = AppKeyFileEntry {
        path: "note.txt".to_string(),
        hash: [1; 32],
        size: 5,
        whole_file_hash: None,
        modified_at: None,
    };

    assert!(may_replace_destination(None, None, false));
    assert!(!may_replace_destination(None, Some(&local_entry), false));
}

#[tokio::test]
async fn primary_merged_root_builds_visible_mount_root_without_metadata() {
    let cfg_dir = tempdir().unwrap();
    let account = Profile::create(cfg_dir.path(), Some("mount-test".into())).unwrap();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

    let source = tempdir().unwrap();
    std::fs::create_dir(source.path().join("empty")).unwrap();
    std::fs::create_dir(source.path().join("docs")).unwrap();
    std::fs::write(source.path().join("docs").join("note.txt"), b"mounted").unwrap();
    let meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: account.state.app_key_pubkey.clone(),
        app_key_seq: 1,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at: 1,
    };
    let source_root = index_dir_with_history_and_meta(&tree, source.path(), None, 1, Some(&meta))
        .await
        .unwrap();

    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    let mut drive = Drive::primary(account.state.root_scope_id());
    drive.app_key_roots.insert(
        account.state.app_key_pubkey.clone(),
        AppKeyRootRef::from_meta(source_root.to_string(), 1, &meta),
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
    let (files, _) = walk_app_key_tree(&tree, &merged.root_cid).await.unwrap();
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
async fn primary_merged_root_projects_share_shortcut_from_source_path() {
    let cfg_dir = tempdir().unwrap();
    let account = Profile::create(cfg_dir.path(), Some("mount-test".into())).unwrap();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

    let source = tempdir().unwrap();
    std::fs::create_dir(source.path().join("tst")).unwrap();
    std::fs::create_dir(source.path().join("tst").join("empty")).unwrap();
    std::fs::write(source.path().join("tst").join("note.txt"), b"shared").unwrap();
    let meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: account.state.app_key_pubkey.clone(),
        app_key_seq: 1,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at: 1,
    };
    let source_root = index_dir_with_history_and_meta(&tree, source.path(), None, 1, Some(&meta))
        .await
        .unwrap();

    let folder = crate::create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        "tst",
        "best",
        account.state.app_key_label.clone(),
        Vec::new(),
        2,
    )
    .unwrap();
    let shortcut = crate::ShareShortcut::new(folder.share_id, "best", "").unwrap();

    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    let mut drive = Drive::primary(account.state.root_scope_id());
    drive.app_key_roots.insert(
        account.state.app_key_pubkey.clone(),
        AppKeyRootRef::from_meta(source_root.to_string(), 1, &meta),
    );
    config.upsert_drive(drive);
    config.upsert_shared_folder(folder);
    config.upsert_share_shortcut(shortcut);

    let view = primary_merged_view(&tree, &config).await.unwrap();
    let shortcut_entry = view
        .view
        .files
        .iter()
        .find(|entry| entry.path == "best/note.txt")
        .expect("shortcut projects source file");
    assert_eq!(shortcut_entry.source_path.as_deref(), Some("tst/note.txt"));

    let merged = primary_merged_root(&tree, &config).await.unwrap();
    let shortcut_file = tree
        .resolve(&merged.root_cid, "best/note.txt")
        .await
        .unwrap()
        .expect("shortcut file exists");
    let bytes = tree.get(&shortcut_file, None).await.unwrap().unwrap();
    assert_eq!(bytes, b"shared");
    assert!(
        tree.resolve(&merged.root_cid, "best/empty")
            .await
            .unwrap()
            .is_some(),
        "shortcut preserves empty source directories"
    );
}

#[tokio::test]
async fn primary_merged_root_does_not_synthesize_missing_modified_at() {
    let cfg_dir = tempdir().unwrap();
    let account = Profile::create(cfg_dir.path(), Some("mount-test".into())).unwrap();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());
    let (file_cid, file_size) = tree.put_file(b"legacy").await.unwrap();
    let source_root = tree
        .put_directory(vec![
            DirEntry::from_cid("legacy.txt", &file_cid)
                .with_size(file_size)
                .with_link_type(LinkType::File),
        ])
        .await
        .unwrap();
    let meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: account.state.app_key_pubkey.clone(),
        app_key_seq: 1,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at: 1_800_000_000,
    };

    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    let mut drive = Drive::primary(account.state.root_scope_id());
    drive.app_key_roots.insert(
        account.state.app_key_pubkey.clone(),
        AppKeyRootRef::from_meta(source_root.to_string(), meta.created_at, &meta),
    );
    config.upsert_drive(drive);

    let view = primary_merged_view(&tree, &config).await.unwrap();
    assert_eq!(view.view.files[0].modified_at, None);

    let merged = primary_merged_root(&tree, &config).await.unwrap();
    let entries = tree.list_directory(&merged.root_cid).await.unwrap();
    let legacy = entries
        .iter()
        .find(|entry| entry.name == "legacy.txt")
        .expect("legacy file remains visible");
    assert!(
        legacy
            .meta
            .as_ref()
            .and_then(|meta| meta.get(MODIFIED_AT_META_KEY))
            .is_none(),
        "legacy entry should not get a synthetic modified_at: {legacy:#?}"
    );
}

#[tokio::test]
async fn primary_merged_root_hides_tombstoned_foreign_directory() {
    let cfg_dir = tempdir().unwrap();
    let account = Profile::create(cfg_dir.path(), Some("mount-test".into())).unwrap();
    let remote_device =
        "5555555555555555555555555555555555555555555555555555555555555555".to_string();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

    let remote_source = tempdir().unwrap();
    std::fs::create_dir_all(remote_source.path().join("codex-lab").join("empty")).unwrap();
    std::fs::write(
        remote_source.path().join("codex-lab").join("note.txt"),
        b"remote",
    )
    .unwrap();
    let remote_meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: remote_device.clone(),
        app_key_seq: 1,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at: 100,
    };
    let remote_root =
        index_dir_with_history_and_meta(&tree, remote_source.path(), None, 100, Some(&remote_meta))
            .await
            .unwrap();

    let local_meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: account.state.app_key_pubkey.clone(),
        app_key_seq: 1,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::from([(
            remote_device.clone(),
            RootObservation {
                app_key_seq: 1,
                root_cid: remote_root.to_string(),
            },
        )]),
        created_at: 101,
    };
    let empty_root = tree.put_directory(Vec::new()).await.unwrap();
    let tombstone_paths = BTreeSet::from(["codex-lab".to_string()]);
    let local_root = crate::indexer::layer_history_and_meta_on_root_with_tombstone_base_and_paths(
        &tree,
        empty_root,
        None,
        Some(&remote_root),
        101,
        Some(&local_meta),
        Some(&tombstone_paths),
    )
    .await
    .unwrap();

    let mut account_state = account.state.clone();
    account_state
        .app_keys
        .as_mut()
        .expect("created account has app keys")
        .app_actors
        .push(AppActorEntry::member(
            remote_device.clone(),
            1,
            Some("remote".into()),
        ));

    let mut config = AppConfig {
        profile: Some(account_state),
        ..AppConfig::default()
    };
    let mut drive = Drive::primary(account.state.root_scope_id());
    drive.app_key_roots.insert(
        account.state.app_key_pubkey.clone(),
        AppKeyRootRef::from_meta(local_root.to_string(), 101, &local_meta),
    );
    drive.app_key_roots.insert(
        remote_device,
        AppKeyRootRef::from_meta(remote_root.to_string(), 100, &remote_meta),
    );
    config.upsert_drive(drive);

    let view = primary_merged_view(&tree, &config).await.unwrap();
    assert!(view.view.files.is_empty());
    assert_eq!(
        view.view.suppressed_by_tombstone,
        vec!["codex-lab".to_string(), "codex-lab/note.txt".to_string()]
    );

    let merged = primary_merged_root(&tree, &config).await.unwrap();
    assert!(
        tree.resolve(&merged.root_cid, "codex-lab")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn primary_merged_root_hides_ignored_legacy_directories() {
    let cfg_dir = tempdir().unwrap();
    let account = Profile::create(cfg_dir.path(), Some("mount-test".into())).unwrap();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

    let source = tempdir().unwrap();
    std::fs::write(source.path().join("keep.txt"), b"keep").unwrap();
    std::fs::create_dir_all(source.path().join(".Trash-1000").join("files")).unwrap();
    let meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: account.state.app_key_pubkey.clone(),
        app_key_seq: 1,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at: 1,
    };
    let source_root = index_dir_with_history_and_meta(&tree, source.path(), None, 1, Some(&meta))
        .await
        .unwrap();
    let trash_dir = tree.put_directory(Vec::new()).await.unwrap();
    let source_root = tree
        .set_entry(
            &source_root,
            &[],
            ".Trash-1000",
            &trash_dir,
            0,
            LinkType::Dir,
        )
        .await
        .unwrap();

    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    let mut drive = Drive::primary(account.state.root_scope_id());
    drive.app_key_roots.insert(
        account.state.app_key_pubkey.clone(),
        AppKeyRootRef::from_meta(source_root.to_string(), 1, &meta),
    );
    config.upsert_drive(drive);

    let merged = primary_merged_root(&tree, &config).await.unwrap();
    let top = tree.list_directory(&merged.root_cid).await.unwrap();
    let top_names = top
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(top_names, vec!["keep.txt"]);
}

#[tokio::test]
async fn primary_merged_view_keeps_previously_accepted_roots_after_device_relink() {
    let cfg_dir = tempdir().unwrap();
    let mut account = Profile::create(cfg_dir.path(), Some("owner".into())).unwrap();
    let old_pixel = nostr_sdk::Keys::generate().public_key().to_hex();
    let new_pixel = nostr_sdk::Keys::generate().public_key().to_hex();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

    account
        .approve_app_key(&old_pixel, Some("Pixel".into()))
        .unwrap();
    let owner_root = index_device_file_root(
        &tree,
        &account.state.app_key_pubkey,
        "mac.txt",
        b"from mac",
        1,
        10,
    )
    .await;
    let old_pixel_root =
        index_device_file_root(&tree, &old_pixel, "pixel.txt", b"from old pixel", 1, 11).await;

    account.revoke_app_key(&old_pixel).unwrap();
    account
        .approve_app_key(&new_pixel, Some("Pixel".into()))
        .unwrap();

    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    let mut drive = Drive::primary(account.state.root_scope_id());
    drive.app_key_roots.insert(
        account.state.app_key_pubkey.clone(),
        AppKeyRootRef::from_meta(owner_root.0.to_string(), 10, &owner_root.1),
    );
    drive.app_key_roots.insert(
        old_pixel,
        AppKeyRootRef::from_meta(old_pixel_root.0.to_string(), 11, &old_pixel_root.1),
    );
    config.upsert_drive(drive);

    let view = primary_merged_view(&tree, &config).await.unwrap();
    let paths = view
        .view
        .files
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["mac.txt", "pixel.txt"]);
}

#[tokio::test]
async fn primary_merged_root_surfaces_concurrent_write_conflict_copy() {
    let cfg_dir = tempdir().unwrap();
    let account = Profile::create(cfg_dir.path(), Some("owner".into())).unwrap();
    let peer_device =
        "2222222222222222222222222222222222222222222222222222222222222222".to_string();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

    let owner_root =
        index_device_note_root(&tree, &account.state.app_key_pubkey, b"owner edit", 1, 10).await;
    let peer_root = index_device_note_root(&tree, &peer_device, b"peer edit", 1, 11).await;

    let mut account_state = account.state.clone();
    account_state
        .app_keys
        .as_mut()
        .expect("created account has app keys")
        .app_actors
        .push(AppActorEntry::member(
            peer_device.clone(),
            1,
            Some("peer".into()),
        ));

    let mut config = AppConfig {
        profile: Some(account_state),
        ..AppConfig::default()
    };
    let mut drive = Drive::primary(account.state.root_scope_id());
    drive.app_key_roots.insert(
        account.state.app_key_pubkey.clone(),
        AppKeyRootRef::from_meta(owner_root.0.to_string(), 10, &owner_root.1),
    );
    drive.app_key_roots.insert(
        peer_device,
        AppKeyRootRef::from_meta(peer_root.0.to_string(), 11, &peer_root.1),
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

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn primary_merged_root_surfaces_concurrent_write_delete_conflict_copy() {
    let cfg_dir = tempdir().unwrap();
    let account = Profile::create(cfg_dir.path(), Some("owner".into())).unwrap();
    let peer_device =
        "4444444444444444444444444444444444444444444444444444444444444444".to_string();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

    let owner_edit_meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: account.state.app_key_pubkey.clone(),
        app_key_seq: 2,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at: 10,
    };
    let owner_edit_root =
        index_device_note_root_with_meta(&tree, b"owner edit", 10, owner_edit_meta.clone()).await;
    let peer_base =
        index_device_note_root(&tree, &peer_device, b"baseline before delete", 1, 9).await;
    let peer_delete_meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: peer_device.clone(),
        app_key_seq: 2,
        dck_generation: 1,
        local_only: false,
        parents: vec![RootParent {
            app_key_pubkey: peer_device.clone(),
            app_key_seq: 1,
            root_cid: peer_base.0.to_string(),
        }],
        observed: BTreeMap::new(),
        created_at: 11,
    };
    let empty = tempdir().unwrap();
    let peer_delete_root = index_dir_with_history_and_meta(
        &tree,
        empty.path(),
        Some(&peer_base.0),
        11,
        Some(&peer_delete_meta),
    )
    .await
    .unwrap();

    let mut account_state = account.state.clone();
    account_state
        .app_keys
        .as_mut()
        .expect("created account has app keys")
        .app_actors
        .push(AppActorEntry::member(
            peer_device.clone(),
            1,
            Some("peer".into()),
        ));

    let mut config = AppConfig {
        profile: Some(account_state),
        ..AppConfig::default()
    };
    let mut drive = Drive::primary(account.state.root_scope_id());
    drive.app_key_roots.insert(
        account.state.app_key_pubkey.clone(),
        AppKeyRootRef::from_meta(owner_edit_root.to_string(), 10, &owner_edit_meta),
    );
    drive.app_key_roots.insert(
        peer_device,
        AppKeyRootRef::from_meta(peer_delete_root.to_string(), 11, &peer_delete_meta),
    );
    config.upsert_drive(drive);

    let view = primary_merged_view(&tree, &config).await.unwrap();
    assert_eq!(view.view.conflicts, vec!["docs/note.txt"]);
    assert_eq!(view.file_count(), 1);
    assert!(
        view.view
            .files
            .iter()
            .all(|entry| entry.path != "docs/note.txt")
    );
    assert!(view.view.files.iter().any(|entry| {
        entry.path.starts_with("docs/note (conflict from ")
            && entry.path.ends_with(").txt")
            && entry.source_path.as_deref() == Some("docs/note.txt")
    }));

    let merged = primary_merged_root(&tree, &config).await.unwrap();
    assert_eq!(merged.file_count, 1);
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
    assert_eq!(names.len(), 1);
    assert!(names[0].starts_with("note (conflict from "));
    assert!(names[0].ends_with(").txt"));
    let cid = tree
        .resolve(&merged.root_cid, &format!("docs/{}", names[0]))
        .await
        .unwrap()
        .expect("visible conflict copy exists");
    let bytes = tree.get(&cid, None).await.unwrap().unwrap();
    assert_eq!(bytes, b"owner edit");
}

#[tokio::test]
async fn primary_merged_view_ignores_local_only_root_publish_time() {
    let cfg_dir = tempdir().unwrap();
    let account = Profile::create(cfg_dir.path(), Some("owner".into())).unwrap();
    let peer_device =
        "3333333333333333333333333333333333333333333333333333333333333333".to_string();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

    let owner_source =
        index_device_note_root(&tree, &account.state.app_key_pubkey, b"owner source", 1, 10).await;
    let peer_source = index_device_note_root(&tree, &peer_device, b"peer source", 1, 11).await;
    let owner_mirror_meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: account.state.app_key_pubkey.clone(),
        app_key_seq: 2,
        dck_generation: 1,
        local_only: true,
        parents: vec![RootParent {
            app_key_pubkey: account.state.app_key_pubkey.clone(),
            app_key_seq: 1,
            root_cid: owner_source.0.to_string(),
        }],
        observed: BTreeMap::new(),
        created_at: 20,
    };
    let owner_mirror = index_device_note_root_with_meta(
        &tree,
        b"local-only mirror",
        20,
        owner_mirror_meta.clone(),
    )
    .await;

    let mut account_state = account.state.clone();
    account_state
        .app_keys
        .as_mut()
        .expect("created account has app keys")
        .app_actors
        .push(AppActorEntry::member(
            peer_device.clone(),
            1,
            Some("peer".into()),
        ));

    let mut config = AppConfig {
        profile: Some(account_state),
        ..AppConfig::default()
    };
    let mut drive = Drive::primary(account.state.root_scope_id());
    drive.app_key_roots.insert(
        account.state.app_key_pubkey.clone(),
        AppKeyRootRef::from_meta(owner_mirror.to_string(), 20, &owner_mirror_meta),
    );
    drive.app_key_roots.insert(
        peer_device.clone(),
        AppKeyRootRef::from_meta(peer_source.0.to_string(), 11, &peer_source.1),
    );
    config.upsert_drive(drive);

    let view = primary_merged_view(&tree, &config).await.unwrap();
    let original = view
        .view
        .files
        .iter()
        .find(|entry| entry.path == "docs/note.txt")
        .expect("original path remains visible");
    assert_eq!(original.source_app_key_pubkey, peer_device);
}

#[tokio::test]
async fn primary_merged_root_reads_conflict_bytes_from_local_only_parent() {
    let cfg_dir = tempdir().unwrap();
    let account = Profile::create(cfg_dir.path(), Some("owner".into())).unwrap();
    let peer_device =
        "5555555555555555555555555555555555555555555555555555555555555555".to_string();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

    let owner_source =
        index_device_note_root(&tree, &account.state.app_key_pubkey, b"owner source", 1, 10).await;
    let peer_source = index_device_note_root(&tree, &peer_device, b"peer source", 1, 11).await;
    let owner_mirror_meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: account.state.app_key_pubkey.clone(),
        app_key_seq: 2,
        dck_generation: 1,
        local_only: true,
        parents: vec![RootParent {
            app_key_pubkey: account.state.app_key_pubkey.clone(),
            app_key_seq: 1,
            root_cid: owner_source.0.to_string(),
        }],
        observed: BTreeMap::new(),
        created_at: 20,
    };
    let owner_mirror =
        index_device_note_root_with_meta(&tree, b"peer source", 20, owner_mirror_meta.clone())
            .await;

    let mut account_state = account.state.clone();
    account_state
        .app_keys
        .as_mut()
        .expect("created account has app keys")
        .app_actors
        .push(AppActorEntry::member(
            peer_device.clone(),
            1,
            Some("peer".into()),
        ));

    let mut config = AppConfig {
        profile: Some(account_state),
        ..AppConfig::default()
    };
    let mut drive = Drive::primary(account.state.root_scope_id());
    drive.app_key_roots.insert(
        account.state.app_key_pubkey.clone(),
        AppKeyRootRef::from_meta(owner_mirror.to_string(), 20, &owner_mirror_meta),
    );
    drive.app_key_roots.insert(
        peer_device,
        AppKeyRootRef::from_meta(peer_source.0.to_string(), 11, &peer_source.1),
    );
    config.upsert_drive(drive);

    let merged = primary_merged_root(&tree, &config).await.unwrap();
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
    assert_eq!(contents, vec!["owner source", "peer source"]);
}

async fn index_device_note_root(
    tree: &HashTree<MemoryStore>,
    app_key_pubkey: &str,
    bytes: &[u8],
    app_key_seq: u64,
    published_at: i64,
) -> (Cid, DriveRootMeta) {
    let meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: app_key_pubkey.to_string(),
        app_key_seq,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at: published_at,
    };
    let root = index_device_note_root_with_meta(tree, bytes, published_at, meta.clone()).await;
    (root, meta)
}

async fn index_device_file_root(
    tree: &HashTree<MemoryStore>,
    app_key_pubkey: &str,
    path: &str,
    bytes: &[u8],
    app_key_seq: u64,
    published_at: i64,
) -> (Cid, DriveRootMeta) {
    let meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: app_key_pubkey.to_string(),
        app_key_seq,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at: published_at,
    };
    let source = tempdir().unwrap();
    std::fs::write(source.path().join(path), bytes).unwrap();
    let root =
        index_dir_with_history_and_meta(tree, source.path(), None, published_at, Some(&meta))
            .await
            .unwrap();
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
    index_dir_with_history_and_meta(tree, source.path(), None, published_at, Some(&meta))
        .await
        .unwrap()
}
