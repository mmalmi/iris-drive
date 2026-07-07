use super::*;

#[allow(clippy::too_many_lines)]
#[tokio::test]
async fn mounted_visible_import_ignores_previous_local_only_projection_files() {
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
    daemon
        .materialize_primary_merged_root()
        .await
        .unwrap()
        .expect("materialized local-only projection root");
    let local_only = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap()
        .clone();
    assert!(local_only.local_only);
    let local_only_cid = Cid::parse(&local_only.root_cid).unwrap();
    let (local_only_files, _) = crate::merge::walk_app_key_tree(daemon.tree(), &local_only_cid)
        .await
        .unwrap();
    assert_eq!(
        local_only_files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["foreign.txt"]
    );

    let visible = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap();
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
    let changed_paths = BTreeSet::from(["local.txt".to_string()]);
    daemon
        .import_visible_root_with_tombstone_base_and_paths(
            edited_visible_root,
            Some(visible.root_cid.clone()),
            Some(&changed_paths),
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
    assert!(!root.local_only);
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

    let merged = crate::primary_merged_view(daemon.tree(), daemon.config())
        .await
        .unwrap();
    assert_eq!(
        merged
            .view
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["foreign.txt", "local.txt"]
    );
}

#[tokio::test]
async fn local_only_projection_import_does_not_tombstone_previous_local_files() {
    let cfg_dir = tempdir().unwrap();
    let account = init_config_with_account(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

    let first = tempdir().unwrap();
    std::fs::create_dir_all(first.path().join("seed")).unwrap();
    std::fs::write(first.path().join("seed/android.txt"), b"from android").unwrap();
    daemon.import_source_dir(first.path()).await.unwrap();

    let visible_root_without_seed = daemon.tree().put_directory(Vec::new()).await.unwrap();
    daemon
        .import_visible_root_local_only(visible_root_without_seed)
        .await
        .unwrap();

    let root = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap();
    assert!(root.local_only);
    let root_cid = Cid::parse(&root.root_cid).unwrap();
    let (_, tombstones) = crate::merge::walk_app_key_tree(daemon.tree(), &root_cid)
        .await
        .unwrap();
    assert!(
        tombstones.is_empty(),
        "local-only projection roots must not invent user deletes: {tombstones:?}"
    );
}

#[tokio::test]
async fn publish_after_local_only_projection_uses_publishable_history_root() {
    let cfg_dir = tempdir().unwrap();
    let account = init_config_with_account(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

    let first = tempdir().unwrap();
    std::fs::create_dir_all(first.path().join("seed")).unwrap();
    std::fs::write(first.path().join("seed/android.txt"), b"from android").unwrap();
    daemon.import_source_dir(first.path()).await.unwrap();

    let initial_root = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap()
        .clone();
    let initial_cid = Cid::parse(&initial_root.root_cid).unwrap();
    let old_projection_root = daemon.tree().put_directory(Vec::new()).await.unwrap();
    let old_projection_at = daemon.next_import_timestamp();
    let mut old_projection_meta = daemon.root_meta_for_import(old_projection_at).unwrap();
    old_projection_meta.local_only = true;
    let old_projection_cid = crate::indexer::layer_history_and_meta_on_root(
        daemon.tree(),
        old_projection_root,
        Some(&initial_cid),
        old_projection_at,
        Some(&old_projection_meta),
    )
    .await
    .unwrap();
    let (_, old_tombstones) = crate::merge::walk_app_key_tree(daemon.tree(), &old_projection_cid)
        .await
        .unwrap();
    assert_eq!(
        old_tombstones
            .iter()
            .map(|tombstone| tombstone.path.as_str())
            .collect::<Vec<_>>(),
        vec!["seed/android.txt"]
    );
    daemon
        .report_and_record_root(
            old_projection_cid,
            None,
            Some(&old_projection_meta),
            old_projection_at,
        )
        .await
        .unwrap();

    let base_root = daemon.tree().put_directory(Vec::new()).await.unwrap();
    let (file, size) = daemon.tree().put(b"local edit").await.unwrap();
    let edited_root = daemon
        .tree()
        .set_entry(
            &base_root,
            &[],
            "local.txt",
            &file,
            size,
            hashtree_core::LinkType::Blob,
        )
        .await
        .unwrap();
    let changed_paths = BTreeSet::from(["local.txt".to_string()]);
    daemon
        .import_visible_root_with_tombstone_base_and_paths(
            edited_root,
            Some(base_root),
            Some(&changed_paths),
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
    assert!(!root.local_only);
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
    assert!(
        daemon
            .tree()
            .resolve(&root_cid, "seed")
            .await
            .unwrap()
            .is_none(),
        "legacy local-only tombstone masks must not leave empty parent directories"
    );
    assert!(
        tombstones.is_empty(),
        "publishable roots must not carry tombstones from local-only projections: {tombstones:?}"
    );
}

#[tokio::test]
async fn local_only_projection_chain_keeps_publishable_parent_reachable() {
    let cfg_dir = tempdir().unwrap();
    let account = init_config_with_account(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

    let first = tempdir().unwrap();
    std::fs::create_dir_all(first.path().join("seed")).unwrap();
    std::fs::write(first.path().join("seed/android.txt"), b"from android").unwrap();
    daemon.import_source_dir(first.path()).await.unwrap();

    let publishable_root = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap()
        .clone();

    for _ in 0..40 {
        let projection_root = daemon.tree().put_directory(Vec::new()).await.unwrap();
        daemon
            .import_visible_root_local_only(projection_root)
            .await
            .unwrap();
    }

    let current = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap();
    assert!(current.local_only);
    let current_cid = Cid::parse(&current.root_cid).unwrap();
    let current_meta = crate::indexer::read_root_meta(daemon.tree(), &current_cid)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(current_meta.parents.len(), 1);
    assert_eq!(current_meta.parents[0].root_cid, publishable_root.root_cid);

    let merged = crate::primary_merged_view(daemon.tree(), daemon.config())
        .await
        .unwrap();
    assert_eq!(
        merged
            .view
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["seed/android.txt"]
    );
}

#[tokio::test]
async fn legacy_local_only_projection_chain_keeps_publishable_parent_reachable() {
    let cfg_dir = tempdir().unwrap();
    let account = init_config_with_account(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

    let first = tempdir().unwrap();
    std::fs::create_dir_all(first.path().join("seed")).unwrap();
    std::fs::write(first.path().join("seed/android.txt"), b"from android").unwrap();
    daemon.import_source_dir(first.path()).await.unwrap();

    for _ in 0..40 {
        let projection_root = daemon.tree().put_directory(Vec::new()).await.unwrap();
        let projection_at = daemon.next_import_timestamp();
        let mut projection_meta = daemon.root_meta_for_import(projection_at).unwrap();
        projection_meta.local_only = true;
        let projection_cid = crate::indexer::layer_history_and_meta_on_root(
            daemon.tree(),
            projection_root,
            None,
            projection_at,
            Some(&projection_meta),
        )
        .await
        .unwrap();
        daemon
            .report_and_record_root(projection_cid, None, Some(&projection_meta), projection_at)
            .await
            .unwrap();
    }

    let current = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap();
    assert!(current.local_only);

    let merged = crate::primary_merged_view(daemon.tree(), daemon.config())
        .await
        .unwrap();
    assert_eq!(
        merged
            .view
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["seed/android.txt"]
    );
}
