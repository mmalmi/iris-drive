use super::*;
use std::collections::{BTreeMap, BTreeSet};

use crate::config::Drive;
use crate::conflict::{ConflictRecord, ConflictSide, ConflictState};
use crate::identity::Identity;
use crate::root_meta::DriveRootMeta;
use tempfile::tempdir;

mod convergence;

fn init_config(dir: &Path) -> Identity {
    let identity = Identity::generate(key_path_in(dir));
    identity.save().unwrap();
    let mut cfg = AppConfig::default();
    cfg.upsert_drive(Drive::primary(crate::NostrIdentityId::new_v4().to_string()));
    cfg.save(config_path_in(dir)).unwrap();
    identity
}

/// Spin up a real `Profile` via the create flow, then save the
/// `ProfileState` into `AppConfig`. Used to exercise the per-device
/// root code path.
fn init_config_with_account(dir: &Path) -> crate::profile::Profile {
    let account = crate::profile::Profile::create(dir, Some("test-device".into())).unwrap();
    let mut cfg = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(account.state.root_scope_id()));
    cfg.save(config_path_in(dir)).unwrap();
    account
}

fn approve_remote_app_key(
    dir: &Path,
    account: &mut crate::profile::Profile,
    remote: &str,
    label: &str,
) {
    account
        .approve_app_key(remote, Some(label.to_string()))
        .unwrap();
    let mut config = AppConfig::load_or_default(config_path_in(dir)).unwrap();
    config.profile = Some(account.state.clone());
    config.save(config_path_in(dir)).unwrap();
}

#[tokio::test]
async fn import_does_not_configure_plain_directory_mode() {
    let cfg_dir = tempdir().unwrap();
    init_config_with_account(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

    let work = tempdir().unwrap();
    std::fs::write(work.path().join("a.txt"), b"a").unwrap();
    daemon.import_source_dir(work.path()).await.unwrap();

    drop(daemon);
    let saved = std::fs::read_to_string(config_path_in(cfg_dir.path())).unwrap();
    assert!(!saved.contains("working_dir"));
}

#[tokio::test]
async fn import_visible_root_records_mount_deletions_as_tombstones() {
    let cfg_dir = tempdir().unwrap();
    let account = init_config_with_account(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

    let first = tempdir().unwrap();
    std::fs::write(first.path().join("removed.txt"), b"gone from mount").unwrap();
    std::fs::write(first.path().join("kept.txt"), b"still mounted").unwrap();
    daemon.import_source_dir(first.path()).await.unwrap();

    let visible = tempdir().unwrap();
    std::fs::write(visible.path().join("kept.txt"), b"still mounted").unwrap();
    let visible_root = crate::indexer::index_dir(daemon.tree(), visible.path())
        .await
        .unwrap();
    let report = daemon.import_visible_root(visible_root).await.unwrap();

    let root = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap();
    assert_eq!(report.root_cid, root.root_cid);
    assert!(!root.local_only);
    assert_eq!(root.app_key_seq, 2);

    let root_cid = Cid::parse(&root.root_cid).unwrap();
    let (files, tombstones) = crate::merge::walk_app_key_tree(daemon.tree(), &root_cid)
        .await
        .unwrap();
    assert_eq!(
        files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["kept.txt"]
    );
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].path, "removed.txt");
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn import_visible_root_tombstones_deleted_foreign_visible_files() {
    let cfg_dir = tempdir().unwrap();
    let mut account = init_config_with_account(cfg_dir.path());
    let remote = Identity::generate(cfg_dir.path().join("remote.key")).pubkey_hex();
    approve_remote_app_key(cfg_dir.path(), &mut account, &remote, "remote");

    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let remote_dir = tempdir().unwrap();
    std::fs::write(
        remote_dir.path().join("foreign.txt"),
        b"from another device",
    )
    .unwrap();
    let remote_meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.into(),
        app_key_pubkey: remote.clone(),
        app_key_seq: 1,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at: 100,
    };
    let remote_root = crate::indexer::index_dir_with_history_and_meta(
        daemon.tree(),
        remote_dir.path(),
        None,
        100,
        Some(&remote_meta),
    )
    .await
    .unwrap();
    drop(daemon);

    let mut config = AppConfig::load_or_default(config_path_in(cfg_dir.path())).unwrap();
    let mut drive = config.drive(PRIMARY_DRIVE_ID).unwrap().clone();
    drive.app_key_roots.insert(
        remote.clone(),
        AppKeyRootRef::from_meta(
            remote_root.to_string(),
            remote_meta.created_at,
            &remote_meta,
        ),
    );
    config.upsert_drive(drive);
    config.save(config_path_in(cfg_dir.path())).unwrap();

    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    let visible = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap();
    assert!(
        daemon
            .tree()
            .resolve(&visible.root_cid, "foreign.txt")
            .await
            .unwrap()
            .is_some()
    );

    let edited_visible_root = daemon.tree().put_directory(Vec::new()).await.unwrap();
    daemon
        .import_visible_root_with_tombstone_base(
            edited_visible_root,
            Some(visible.root_cid.clone()),
        )
        .await
        .unwrap();

    let root = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap();
    let root_cid = Cid::parse(&root.root_cid).unwrap();
    let (files, tombstones) = crate::merge::walk_app_key_tree(daemon.tree(), &root_cid)
        .await
        .unwrap();
    assert!(files.is_empty());
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].path, "foreign.txt");

    let merged = crate::primary_merged_view(daemon.tree(), daemon.config())
        .await
        .unwrap();
    assert!(
        merged
            .view
            .files
            .iter()
            .all(|entry| entry.path != "foreign.txt"),
        "foreign file should be suppressed by the local tombstone"
    );
}

#[tokio::test]
async fn scoped_visible_root_import_only_tombstones_changed_paths() {
    let cfg_dir = tempdir().unwrap();
    let mut account = init_config_with_account(cfg_dir.path());
    let remote = Identity::generate(cfg_dir.path().join("remote.key")).pubkey_hex();
    approve_remote_app_key(cfg_dir.path(), &mut account, &remote, "remote");

    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let remote_dir = tempdir().unwrap();
    std::fs::write(remote_dir.path().join("explicit-delete.txt"), b"delete me").unwrap();
    std::fs::write(remote_dir.path().join("projection-gap.txt"), b"keep me").unwrap();
    let remote_meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.into(),
        app_key_pubkey: remote.clone(),
        app_key_seq: 1,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at: 100,
    };
    let remote_root = crate::indexer::index_dir_with_history_and_meta(
        daemon.tree(),
        remote_dir.path(),
        None,
        100,
        Some(&remote_meta),
    )
    .await
    .unwrap();
    drop(daemon);

    let mut config = AppConfig::load_or_default(config_path_in(cfg_dir.path())).unwrap();
    let mut drive = config.drive(PRIMARY_DRIVE_ID).unwrap().clone();
    drive.app_key_roots.insert(
        remote.clone(),
        AppKeyRootRef::from_meta(
            remote_root.to_string(),
            remote_meta.created_at,
            &remote_meta,
        ),
    );
    config.upsert_drive(drive);
    config.save(config_path_in(cfg_dir.path())).unwrap();

    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    let visible = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap();
    let edited_visible_root = daemon.tree().put_directory(Vec::new()).await.unwrap();
    let tombstone_paths = BTreeSet::from(["explicit-delete.txt".to_string()]);
    daemon
        .import_visible_root_with_tombstone_base_and_paths(
            edited_visible_root,
            Some(visible.root_cid.clone()),
            Some(&tombstone_paths),
        )
        .await
        .unwrap();

    let root = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap();
    let root_cid = Cid::parse(&root.root_cid).unwrap();
    let (files, tombstones) = crate::merge::walk_app_key_tree(daemon.tree(), &root_cid)
        .await
        .unwrap();
    assert!(files.is_empty());
    assert_eq!(tombstones.len(), 1);
    assert_eq!(tombstones[0].path, "explicit-delete.txt");

    let merged = crate::primary_merged_view(daemon.tree(), daemon.config())
        .await
        .unwrap();
    let visible_paths = merged
        .view
        .files
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(visible_paths, vec!["projection-gap.txt"]);
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn mounted_visible_import_does_not_claim_unchanged_foreign_files() {
    let cfg_dir = tempdir().unwrap();
    let mut account = init_config_with_account(cfg_dir.path());
    let remote = Identity::generate(cfg_dir.path().join("remote.key")).pubkey_hex();
    approve_remote_app_key(cfg_dir.path(), &mut account, &remote, "remote");

    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let remote_dir = tempdir().unwrap();
    std::fs::write(remote_dir.path().join("foreign.txt"), b"from remote").unwrap();
    let remote_meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.into(),
        app_key_pubkey: remote.clone(),
        app_key_seq: 1,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at: 100,
    };
    let remote_root = crate::indexer::index_dir_with_history_and_meta(
        daemon.tree(),
        remote_dir.path(),
        None,
        100,
        Some(&remote_meta),
    )
    .await
    .unwrap();
    drop(daemon);

    let mut config = AppConfig::load_or_default(config_path_in(cfg_dir.path())).unwrap();
    let mut drive = config.drive(PRIMARY_DRIVE_ID).unwrap().clone();
    drive.app_key_roots.insert(
        remote.clone(),
        AppKeyRootRef::from_meta(
            remote_root.to_string(),
            remote_meta.created_at,
            &remote_meta,
        ),
    );
    config.upsert_drive(drive);
    config.save(config_path_in(cfg_dir.path())).unwrap();

    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    let visible = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap();
    assert!(
        daemon
            .tree()
            .resolve(&visible.root_cid, "foreign.txt")
            .await
            .unwrap()
            .is_some()
    );
    let (local_file, local_size) = daemon.tree().put(b"from local").await.unwrap();
    let edited_visible_root = daemon
        .tree()
        .set_entry(
            &visible.root_cid,
            &[],
            "local.txt",
            &local_file,
            local_size,
            hashtree_core::LinkType::Blob,
        )
        .await
        .unwrap();

    daemon
        .import_visible_root_with_tombstone_base(
            edited_visible_root,
            Some(visible.root_cid.clone()),
        )
        .await
        .unwrap();

    let root = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap();
    let root_cid = Cid::parse(&root.root_cid).unwrap();
    let (files, tombstones) = crate::merge::walk_app_key_tree(daemon.tree(), &root_cid)
        .await
        .unwrap();
    assert_eq!(
        files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["local.txt"]
    );
    assert!(tombstones.is_empty());
}

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn mounted_visible_import_does_not_claim_foreign_files_projected_after_base() {
    let cfg_dir = tempdir().unwrap();
    let mut account = init_config_with_account(cfg_dir.path());
    let remote = Identity::generate(cfg_dir.path().join("remote.key")).pubkey_hex();
    approve_remote_app_key(cfg_dir.path(), &mut account, &remote, "remote");

    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let old_visible = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap();
    let remote_dir = tempdir().unwrap();
    std::fs::write(remote_dir.path().join("foreign-new.txt"), b"from remote").unwrap();
    let remote_meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.into(),
        app_key_pubkey: remote.clone(),
        app_key_seq: 1,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at: 100,
    };
    let remote_root = crate::indexer::index_dir_with_history_and_meta(
        daemon.tree(),
        remote_dir.path(),
        None,
        100,
        Some(&remote_meta),
    )
    .await
    .unwrap();
    drop(daemon);

    let mut config = AppConfig::load_or_default(config_path_in(cfg_dir.path())).unwrap();
    let mut drive = config.drive(PRIMARY_DRIVE_ID).unwrap().clone();
    drive.app_key_roots.insert(
        remote.clone(),
        AppKeyRootRef::from_meta(
            remote_root.to_string(),
            remote_meta.created_at,
            &remote_meta,
        ),
    );
    config.upsert_drive(drive);
    config.save(config_path_in(cfg_dir.path())).unwrap();

    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    let latest_visible = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap();
    assert!(
        daemon
            .tree()
            .resolve(&latest_visible.root_cid, "foreign-new.txt")
            .await
            .unwrap()
            .is_some()
    );
    let (local_file, local_size) = daemon.tree().put(b"from local").await.unwrap();
    let edited_visible_root = daemon
        .tree()
        .set_entry(
            &latest_visible.root_cid,
            &[],
            "local.txt",
            &local_file,
            local_size,
            hashtree_core::LinkType::Blob,
        )
        .await
        .unwrap();

    daemon
        .import_visible_root_with_tombstone_base(
            edited_visible_root,
            Some(old_visible.root_cid.clone()),
        )
        .await
        .unwrap();

    let root = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap();
    let root_cid = Cid::parse(&root.root_cid).unwrap();
    let (files, tombstones) = crate::merge::walk_app_key_tree(daemon.tree(), &root_cid)
        .await
        .unwrap();
    assert_eq!(
        files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["local.txt"]
    );
    assert!(tombstones.is_empty());
}

#[tokio::test]
async fn import_persists_rebuildable_sync_cache_with_base_state() {
    let cfg_dir = tempdir().unwrap();
    let account = init_config_with_account(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

    let work = tempdir().unwrap();
    std::fs::write(work.path().join("note.txt"), b"hello cache").unwrap();
    let report = daemon.import_source_dir(work.path()).await.unwrap();

    let cache =
        crate::sync_cache::SyncCache::load(crate::paths::sync_cache_path_in(cfg_dir.path()))
            .unwrap();
    assert_eq!(cache.schema, crate::sync_cache::SyncCache::SCHEMA);
    assert_eq!(cache.roots.len(), 1);
    assert_eq!(cache.roots[0].app_key_pubkey, account.state.app_key_pubkey);
    assert_eq!(cache.roots[0].root_cid, report.root_cid);
    assert_eq!(cache.path_state.len(), 1);
    assert_eq!(cache.path_state[0].path, "note.txt");
    assert_eq!(cache.path_state[0].root_cid, report.root_cid);
    assert!(cache.path_state[0].whole_file_hash.is_some());
    assert_eq!(cache.base_state.len(), 1);
    assert_eq!(cache.base_state[0].path, "note.txt");
    assert_eq!(cache.base_state[0].base_root_cid, report.root_cid);
    assert_eq!(
        cache.base_anchor_for_drive(PRIMARY_DRIVE_ID),
        Some(report.root_cid.as_str())
    );

    std::fs::remove_file(crate::paths::sync_cache_path_in(cfg_dir.path())).unwrap();
    let rebuilt = daemon.rebuild_sync_cache().await.unwrap();
    assert_eq!(rebuilt.roots.len(), 1);
    assert_eq!(rebuilt.path_state.len(), 1);
    assert_eq!(rebuilt.path_state[0].path, "note.txt");
    assert!(
        rebuilt.base_state.is_empty(),
        "rebuilds restore current state but not historical base quality"
    );
}

#[tokio::test]
async fn corrupt_sync_cache_rebuilds_from_signed_roots() {
    let cfg_dir = tempdir().unwrap();
    init_config_with_account(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

    let work = tempdir().unwrap();
    std::fs::write(work.path().join("note.txt"), b"hello cache").unwrap();
    let report = daemon.import_source_dir(work.path()).await.unwrap();

    let cache_path = crate::paths::sync_cache_path_in(cfg_dir.path());
    std::fs::write(&cache_path, b"{ definitely not json").unwrap();
    let rebuilt = daemon.load_or_rebuild_sync_cache().await.unwrap();

    assert_eq!(rebuilt.roots.len(), 1);
    assert_eq!(rebuilt.path_state.len(), 1);
    assert_eq!(rebuilt.path_state[0].path, "note.txt");
    assert_eq!(rebuilt.path_state[0].root_cid, report.root_cid);
    assert!(rebuilt.base_state.is_empty());

    let loaded = crate::sync_cache::SyncCache::load(cache_path).unwrap();
    assert_eq!(loaded.path_state, rebuilt.path_state);
}

#[tokio::test]
async fn import_records_per_app_key_root_when_account_present() {
    let cfg_dir = tempdir().unwrap();
    let account = init_config_with_account(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

    let work = tempdir().unwrap();
    std::fs::write(work.path().join("hello.txt"), b"hi").unwrap();
    let report = daemon.import_source_dir(work.path()).await.unwrap();

    let drive = daemon.config().drive(PRIMARY_DRIVE_ID).unwrap();
    assert_eq!(drive.app_key_roots.len(), 1);
    let entry = drive
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .expect("per-AppKey root for this AppKey");
    assert_eq!(entry.root_cid, report.root_cid);
    assert!(entry.published_at > 0);
    assert_eq!(entry.dck_generation, 1); // create-flow seeds DCK gen 1
    assert_eq!(entry.app_key_seq, 1);
    assert!(entry.parents.is_empty());
}

#[tokio::test]
async fn import_embeds_root_meta_and_advances_app_key_sequence() {
    let cfg_dir = tempdir().unwrap();
    let account = init_config_with_account(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

    let work = tempdir().unwrap();
    std::fs::write(work.path().join("note.txt"), b"one").unwrap();
    let first = daemon.import_source_dir(work.path()).await.unwrap();
    let first_cid = Cid::parse(&first.root_cid).unwrap();
    let first_meta = crate::indexer::read_root_meta(daemon.tree(), &first_cid)
        .await
        .unwrap()
        .expect("first root metadata");
    assert_eq!(first_meta.schema, crate::DriveRootMeta::SCHEMA);
    assert_eq!(first_meta.drive_id, PRIMARY_DRIVE_ID);
    assert_eq!(first_meta.app_key_pubkey, account.state.app_key_pubkey);
    assert_eq!(first_meta.app_key_seq, 1);
    assert!(first_meta.parents.is_empty());

    std::fs::write(work.path().join("note.txt"), b"two").unwrap();
    let second = daemon.import_source_dir(work.path()).await.unwrap();
    let second_cid = Cid::parse(&second.root_cid).unwrap();
    let second_meta = crate::indexer::read_root_meta(daemon.tree(), &second_cid)
        .await
        .unwrap()
        .expect("second root metadata");
    assert_eq!(second_meta.app_key_seq, 2);
    assert_eq!(second_meta.parents.len(), 1);
    assert_eq!(second_meta.parents[0].app_key_seq, 1);
    assert_eq!(second_meta.parents[0].root_cid, first.root_cid);

    let entry = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap();
    assert_eq!(entry.root_cid, second.root_cid);
    assert_eq!(entry.app_key_seq, 2);
    assert_eq!(entry.parents, second_meta.parents);
}

#[tokio::test]
async fn import_publish_timestamps_advance_past_previous_root() {
    let cfg_dir = tempdir().unwrap();
    let account = init_config_with_account(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

    let work = tempdir().unwrap();
    std::fs::write(work.path().join("note.txt"), b"one").unwrap();
    daemon.import_source_dir(work.path()).await.unwrap();

    let future_published_at = unix_now() + 120;
    let drive = daemon
        .config
        .drives
        .iter_mut()
        .find(|drive| drive.drive_id == PRIMARY_DRIVE_ID)
        .unwrap();
    drive
        .app_key_roots
        .get_mut(&account.state.app_key_pubkey)
        .unwrap()
        .published_at = future_published_at;

    std::fs::write(work.path().join("note.txt"), b"two").unwrap();
    let visible_root = crate::indexer::index_dir(daemon.tree(), work.path())
        .await
        .unwrap();
    let second = daemon.import_visible_root(visible_root).await.unwrap();

    let entry = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap();
    assert_eq!(entry.published_at, future_published_at + 1);

    let second_cid = Cid::parse(&second.root_cid).unwrap();
    let second_meta = crate::indexer::read_root_meta(daemon.tree(), &second_cid)
        .await
        .unwrap()
        .expect("second root metadata");
    assert_eq!(second_meta.created_at, future_published_at + 1);
}

#[tokio::test]
async fn import_uses_encrypted_private_hashtree_blocks() {
    let cfg_dir = tempdir().unwrap();
    init_config_with_account(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

    let work = tempdir().unwrap();
    let secret = b"secret contents that must not appear as plaintext in blobs";
    std::fs::write(work.path().join("secret.txt"), secret).unwrap();
    let report = daemon.import_source_dir(work.path()).await.unwrap();

    let cid = Cid::parse(&report.root_cid).unwrap();
    assert!(
        cid.key.is_some(),
        "persistent drive roots must carry a CHK key"
    );

    let mut stack = vec![daemon.blocks_dir().to_path_buf()];
    let mut saw_blob = false;
    while let Some(path) = stack.pop() {
        for entry in std::fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else {
                saw_blob = true;
                let bytes = std::fs::read(&path).unwrap();
                assert!(
                    !bytes.windows(secret.len()).any(|window| window == secret),
                    "stored blob {} leaked plaintext",
                    path.display()
                );
            }
        }
    }
    assert!(saw_blob, "import should write blobs");
}

#[tokio::test]
async fn resolve_conflict_record_marks_record_resolved_and_advances_root() {
    let cfg_dir = tempdir().unwrap();
    init_config_with_account(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

    let work = tempdir().unwrap();
    std::fs::write(work.path().join("report.pdf"), b"chosen").unwrap();
    let imported = daemon.import_source_dir(work.path()).await.unwrap();
    let imported_root = Cid::parse(&imported.root_cid).unwrap();
    let record = conflict_record("conflict-a");

    let mut root_with_conflict =
        crate::indexer::layer_conflict_records(daemon.tree(), imported_root.clone(), &[record])
            .await
            .unwrap();
    root_with_conflict =
        crate::indexer::layer_prev_link(daemon.tree(), root_with_conflict, &imported_root)
            .await
            .unwrap();
    let now = unix_now();
    let root_meta = daemon.root_meta_for_import(now).unwrap();
    root_with_conflict =
        crate::indexer::layer_root_meta(daemon.tree(), root_with_conflict, &root_meta)
            .await
            .unwrap();
    daemon
        .update_primary_drive(&root_with_conflict, Some(&root_meta), now)
        .unwrap();

    let report = daemon.resolve_conflict_record("conflict-a").await.unwrap();

    assert!(report.changed);
    assert_eq!(report.previous_root_cid, root_with_conflict.to_string());
    assert_ne!(report.root_cid, report.previous_root_cid);
    let resolved_root = Cid::parse(&report.root_cid).unwrap();
    let records = crate::indexer::read_conflict_records(daemon.tree(), &resolved_root)
        .await
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].state, ConflictState::Resolved);
    let resolved_meta = crate::indexer::read_root_meta(daemon.tree(), &resolved_root)
        .await
        .unwrap()
        .expect("resolved root metadata");
    assert_eq!(
        resolved_meta.parents[0].root_cid,
        root_with_conflict.to_string()
    );
}

fn conflict_record(conflict_id: &str) -> ConflictRecord {
    ConflictRecord {
        schema: ConflictRecord::SCHEMA,
        conflict_id: conflict_id.into(),
        path: "report.pdf".into(),
        visible_conflict_path: "report (conflict from phone).pdf".into(),
        local: ConflictSide {
            app_key_pubkey: "laptop".into(),
            app_key_seq: 2,
            root_cid: "cid-local".into(),
            whole_file_hash: "hash-local".into(),
        },
        remote: Some(ConflictSide {
            app_key_pubkey: "phone".into(),
            app_key_seq: 7,
            root_cid: "cid-remote".into(),
            whole_file_hash: "hash-remote".into(),
        }),
        deleted: None,
        state: ConflictState::Unresolved,
        created_at: 1234,
    }
}

#[tokio::test]
async fn open_uninitialized_errors() {
    let dir = tempdir().unwrap();
    match Daemon::open(dir.path()) {
        Err(DaemonError::Uninitialized) => {}
        Err(other) => panic!("expected Uninitialized, got {other:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

#[tokio::test]
async fn import_persists_root_cid_to_config() {
    let cfg_dir = tempdir().unwrap();
    init_config(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    assert!(daemon.primary_root().is_none());

    let work = tempdir().unwrap();
    std::fs::write(work.path().join("hello.txt"), b"hi there").unwrap();
    let report = daemon.import_source_dir(work.path()).await.unwrap();
    assert_eq!(report.top_level_entries, 1);
    assert!(!report.root_cid.is_empty());

    // primary drive's last_root_cid is set.
    let recorded = daemon.primary_root().unwrap();
    assert_eq!(recorded, report.root_cid);

    // a fresh open sees the same state.
    let reopened = Daemon::open(cfg_dir.path()).unwrap();
    assert_eq!(reopened.primary_root(), Some(report.root_cid.as_str()));
}

#[tokio::test]
async fn import_survives_across_daemon_restarts() {
    let cfg_dir = tempdir().unwrap();
    init_config(cfg_dir.path());

    let work = tempdir().unwrap();
    std::fs::write(work.path().join("a.txt"), b"alpha").unwrap();

    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    let report = daemon.import_source_dir(work.path()).await.unwrap();
    let root_cid = report.root_cid.clone();
    drop(daemon);

    // Re-open and confirm we can still list the persisted root.
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let cid = Cid::parse(&root_cid).unwrap();
    let listing = daemon.tree().list_directory(&cid).await.unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].name, "a.txt");
}

#[tokio::test]
async fn re_import_records_new_root() {
    let cfg_dir = tempdir().unwrap();
    init_config(cfg_dir.path());
    let work = tempdir().unwrap();

    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    std::fs::write(work.path().join("a.txt"), b"first").unwrap();
    let first = daemon.import_source_dir(work.path()).await.unwrap();

    std::fs::write(work.path().join("b.txt"), b"second").unwrap();
    let second = daemon.import_source_dir(work.path()).await.unwrap();

    assert_ne!(first.root_cid, second.root_cid);
    assert_eq!(daemon.primary_root().unwrap(), second.root_cid);
}

#[tokio::test]
async fn import_without_primary_drive_errors() {
    let cfg_dir = tempdir().unwrap();
    // identity present but no drives in config
    let identity = Identity::generate(key_path_in(cfg_dir.path()));
    identity.save().unwrap();
    AppConfig::default()
        .save(config_path_in(cfg_dir.path()))
        .unwrap();

    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    let work = tempdir().unwrap();
    match daemon.import_source_dir(work.path()).await {
        Err(DaemonError::PrimaryDriveMissing) => {}
        other => panic!("expected PrimaryDriveMissing, got {other:?}"),
    }
}

#[tokio::test]
async fn blocks_dir_is_under_config_dir() {
    let cfg_dir = tempdir().unwrap();
    init_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    assert!(daemon.blocks_dir().starts_with(cfg_dir.path()));
    assert!(daemon.blocks_dir().ends_with("blocks"));
    // Directory exists on disk.
    assert!(daemon.blocks_dir().is_dir());
}

#[test]
fn embedded_hashtree_state_uses_app_data_sibling_for_native_layout() {
    assert_eq!(
        embedded_hashtree_state_root_in(Path::new("/tmp/IrisDrive/AppData/Config")),
        PathBuf::from("/tmp/IrisDrive/AppData/Hashtree")
    );
}

#[test]
fn embedded_hashtree_state_uses_config_child_for_plain_cli_layout() {
    assert_eq!(
        embedded_hashtree_state_root_in(Path::new("/tmp/iris-drive")),
        PathBuf::from("/tmp/iris-drive/Hashtree")
    );
}

#[test]
fn embedded_browser_relays_include_hashtree_resolver_bootstrap_relays() {
    let mut config = AppConfig {
        relays: vec![
            "wss://custom.example".to_owned(),
            "wss://relay.snort.social/".to_owned(),
        ],
        ..AppConfig::default()
    };

    let relays = embedded_browser_nostr_relays(&config);

    assert_eq!(relays[0], "wss://custom.example");
    assert!(relays.iter().any(|relay| relay == "wss://relay.damus.io"));
    assert!(relays.iter().any(|relay| relay == "wss://relay.primal.net"));
    assert_eq!(
        relays
            .iter()
            .filter(|relay| same_relay(relay, "wss://relay.snort.social"))
            .count(),
        1
    );

    config.relays.clear();
    let relays = embedded_browser_nostr_relays(&config);
    assert!(relays.iter().any(|relay| relay == "wss://relay.damus.io"));
    assert!(relays.iter().any(|relay| relay == "wss://relay.primal.net"));
}

#[test]
fn embedded_browser_settings_allow_iris_sites_portal_plaintext_reads() {
    let settings = embedded_browser_settings(&AppConfig::default());

    assert_eq!(
        settings["allowedNpubs"].as_array().unwrap(),
        &[serde_json::json!(crate::gateway::IRIS_SITES_PORTAL_NPUB)]
    );
    assert_eq!(settings["publicWrites"], false);
    assert_eq!(settings["publicPlaintextReads"], false);
}

#[test]
fn embedded_browser_does_not_pin_iris_sites_bootstrap_root() {
    let source = include_str!("../../daemon.rs");

    assert!(!source.contains("IRIS_SITES_PORTAL_BOOTSTRAP_NHASH"));
    assert!(!source.contains("with_initial_tree_roots"));
}
