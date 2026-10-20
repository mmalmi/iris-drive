use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use hashtree_core::{
    BlobReply, BlobRequest, Cid, DirEntry, HashTree, HashTreeConfig, LinkType, MemoryStore, Store,
};
use nostr_sdk::{Keys, nips::nip44};

use super::super::{Exchange, parse_hash};
use crate::config::{AppKeyRootRef, Drive};
use crate::friend_backup::{backup_keys, routes::LocalBackupRoute, storage::FriendBackupStore};
use crate::{AppConfig, AppKey, Daemon, Profile};

async fn fixture(config_dir: &Path) -> (Exchange, Profile, String) {
    let mut profile = Profile::create(config_dir, Some("snapshot test".into())).unwrap();
    let remote = Keys::generate().public_key().to_hex();
    profile.approve_app_key(&remote, None).unwrap();
    let mut config = AppConfig {
        profile: Some(profile.state.clone()),
        ..AppConfig::default()
    };
    let mut drive = Drive::primary(profile.state.root_scope_id());
    config.upsert_drive(drive.clone());
    config
        .save(crate::paths::config_path_in(config_dir))
        .unwrap();
    let daemon = Daemon::open(config_dir).unwrap();
    for (writer, name, contents) in [
        (
            &profile.state.app_key_pubkey,
            "local.txt",
            b"local bytes".as_slice(),
        ),
        (&remote, "remote.txt", b"remote bytes".as_slice()),
    ] {
        let (file, size) = daemon.tree().put(contents).await.unwrap();
        let root = daemon
            .tree()
            .put_directory(vec![
                DirEntry::new(name, file.hash)
                    .with_key(file.key.unwrap())
                    .with_size(size)
                    .with_link_type(LinkType::File),
            ])
            .await
            .unwrap();
        drive.app_key_roots.insert(
            writer.clone(),
            AppKeyRootRef::legacy(root.to_string(), 1, 1),
        );
        if name == "local.txt" {
            drive.last_root_cid = Some(root.to_string());
        }
    }
    config.upsert_drive(drive);
    config
        .save(crate::paths::config_path_in(config_dir))
        .unwrap();
    let app_key = AppKey::load(crate::paths::key_path_in(config_dir)).unwrap();
    (
        Exchange {
            config_dir: config_dir.to_path_buf(),
            keys: backup_keys(&app_key).unwrap(),
            store: FriendBackupStore::open(config_dir.join("friend-backups")).unwrap(),
            local_route: Arc::new(LocalBackupRoute::default()),
            prepared: None,
            last_offer: None,
            status: BTreeMap::new(),
        },
        profile,
        remote,
    )
}

#[tokio::test]
async fn exported_snapshot_restores_files_from_every_active_drive_root() {
    let dir = tempfile::tempdir().unwrap();
    let (mut exchange, _, _) = fixture(dir.path()).await;
    let config_path = crate::paths::config_path_in(dir.path());
    let original_config = std::fs::read(&config_path).unwrap();
    exchange.refresh_snapshot().await.unwrap();
    let exported = exchange.prepared.as_ref().unwrap();
    let restored_store = Arc::new(MemoryStore::new());
    for entry in &exported.backup.manifest.entries {
        let hash = parse_hash(&entry.hash).unwrap();
        let BlobReply::Data(bytes) = exported
            .backup
            .route()
            .route(BlobRequest { hash, htl: 0 })
            .await
            .unwrap()
        else {
            panic!("export did not return a selected block");
        };
        restored_store.put(hash, bytes).await.unwrap();
    }
    let root_text = nip44::decrypt(
        exchange.keys.secret_key(),
        &exchange.keys.public_key(),
        &exported.backup.manifest.encrypted_root_cid,
    )
    .unwrap();
    let root = Cid::parse(&root_text).unwrap();
    let restored = HashTree::new(HashTreeConfig::new(restored_store));
    for (name, contents) in [
        ("local.txt", b"local bytes".as_slice()),
        ("remote.txt", b"remote bytes".as_slice()),
    ] {
        let file = restored
            .resolve(&root, name)
            .await
            .unwrap()
            .expect("visible Drive file must be backed up");
        assert_eq!(restored.get(&file, None).await.unwrap().unwrap(), contents);
    }
    assert_eq!(
        std::fs::read(config_path).unwrap(),
        original_config,
        "backup must not publish or rewrite an AppKey root"
    );
}

#[tokio::test]
async fn unchanged_sources_skip_tree_walk_and_failed_changes_retry() {
    let dir = tempfile::tempdir().unwrap();
    let (mut exchange, _, remote) = fixture(dir.path()).await;
    exchange.refresh_snapshot().await.unwrap();
    let manifest_hash = exchange.prepared.as_ref().unwrap().backup.manifest_hash;
    let daemon = Daemon::open(dir.path()).unwrap();
    let mut config = daemon.config().clone();
    let remote_root = Cid::parse(&config.drives[0].app_key_roots[&remote].root_cid).unwrap();
    let store = daemon.tree().get_store();
    let bytes = store.get(&remote_root.hash).await.unwrap().unwrap();
    store.delete(&remote_root.hash).await.unwrap();
    exchange.refresh_snapshot().await.unwrap();
    assert_eq!(
        exchange.prepared.as_ref().unwrap().backup.manifest_hash,
        manifest_hash
    );

    config.drives[0].display_name = "Renamed Drive".into();
    config
        .save(crate::paths::config_path_in(dir.path()))
        .unwrap();
    assert!(exchange.refresh_snapshot().await.is_err());
    assert!(
        exchange.refresh_snapshot().await.is_err(),
        "failed preparation must not update the source cache"
    );
    store.put(remote_root.hash, bytes).await.unwrap();
    exchange.refresh_snapshot().await.unwrap();
    assert_eq!(
        exchange.prepared.as_ref().unwrap().backup.manifest_hash,
        manifest_hash
    );
}

#[tokio::test]
async fn initial_missing_blocks_retry_and_authorization_changes_refresh_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let (mut exchange, mut profile, remote) = fixture(dir.path()).await;
    let daemon = Daemon::open(dir.path()).unwrap();
    let mut config = daemon.config().clone();
    let remote_root = Cid::parse(&config.drives[0].app_key_roots[&remote].root_cid).unwrap();
    let store = daemon.tree().get_store();
    let bytes = store.get(&remote_root.hash).await.unwrap().unwrap();
    store.delete(&remote_root.hash).await.unwrap();
    assert!(exchange.refresh_snapshot().await.is_err());
    assert!(exchange.prepared.is_none());
    store.put(remote_root.hash, bytes).await.unwrap();
    exchange.refresh_snapshot().await.unwrap();
    let original_root = exchange.prepared.as_ref().unwrap().root.clone();

    profile.revoke_app_key(&remote).unwrap();
    config.profile = Some(profile.state);
    config
        .save(crate::paths::config_path_in(dir.path()))
        .unwrap();
    exchange.refresh_snapshot().await.unwrap();
    let root = Cid::parse(&exchange.prepared.as_ref().unwrap().root).unwrap();
    assert_ne!(root.to_string(), original_root);
    assert!(
        daemon
            .tree()
            .resolve(&root, "local.txt")
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        daemon
            .tree()
            .resolve(&root, "remote.txt")
            .await
            .unwrap()
            .is_none()
    );
}
