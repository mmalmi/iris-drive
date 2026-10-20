use super::*;

#[tokio::test]
async fn provider_directory_delete_does_not_revive_stale_file_directory_conflict() {
    let config_dir = tempdir().unwrap();
    let mut account = init_config_with_account(config_dir.path());
    let file_app_key = Identity::generate(config_dir.path().join("remote-file.key")).pubkey_hex();
    let directory_app_key =
        Identity::generate(config_dir.path().join("remote-directory.key")).pubkey_hex();
    approve_remote_app_key(
        config_dir.path(),
        &mut account,
        &file_app_key,
        "remote-file",
    );
    approve_remote_app_key(
        config_dir.path(),
        &mut account,
        &directory_app_key,
        "remote-directory",
    );

    let daemon = Daemon::open(config_dir.path()).unwrap();
    let file_source = tempdir().unwrap();
    std::fs::write(file_source.path().join("draft"), b"stale file").unwrap();
    let file_meta = remote_root_meta(&file_app_key, 100);
    let file_root = crate::indexer::index_dir_with_history_and_meta(
        daemon.tree(),
        file_source.path(),
        None,
        file_meta.created_at,
        Some(&file_meta),
    )
    .await
    .unwrap();

    let directory_source = tempdir().unwrap();
    std::fs::create_dir(directory_source.path().join("draft")).unwrap();
    std::fs::write(
        directory_source.path().join("draft/child.txt"),
        b"stale directory child",
    )
    .unwrap();
    let directory_meta = remote_root_meta(&directory_app_key, 101);
    let directory_root = crate::indexer::index_dir_with_history_and_meta(
        daemon.tree(),
        directory_source.path(),
        None,
        directory_meta.created_at,
        Some(&directory_meta),
    )
    .await
    .unwrap();
    drop(daemon);

    let mut config = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    let mut drive = config.drive(PRIMARY_DRIVE_ID).unwrap().clone();
    drive.app_key_roots.insert(
        file_app_key.clone(),
        AppKeyRootRef::from_meta(file_root.to_string(), file_meta.created_at, &file_meta),
    );
    drive.app_key_roots.insert(
        directory_app_key.clone(),
        AppKeyRootRef::from_meta(
            directory_root.to_string(),
            directory_meta.created_at,
            &directory_meta,
        ),
    );
    config.upsert_drive(drive);
    config.save(config_path_in(config_dir.path())).unwrap();

    let mut daemon = Daemon::open(config_dir.path()).unwrap();
    let projected_root = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap()
        .root_cid;
    let draft = daemon
        .tree()
        .resolve(&projected_root, "draft")
        .await
        .unwrap()
        .expect("the concurrent directory is initially canonical");
    assert!(daemon.tree().is_dir(&draft).await.unwrap());

    let edited_root = daemon
        .tree()
        .remove_entry(&projected_root, &[], "draft")
        .await
        .unwrap();
    daemon
        .import_visible_root_with_tombstone_base(edited_root, Some(projected_root))
        .await
        .unwrap();
    let local_root = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap();
    let local_meta =
        crate::indexer::read_root_meta(daemon.tree(), &Cid::parse(&local_root.root_cid).unwrap())
            .await
            .unwrap()
            .expect("provider import records causal root metadata");
    assert!(local_meta.observed.contains_key(&file_app_key));
    assert!(local_meta.observed.contains_key(&directory_app_key));
    let (_, local_tombstones) =
        crate::merge::walk_app_key_tree(daemon.tree(), &Cid::parse(&local_root.root_cid).unwrap())
            .await
            .unwrap();
    assert!(
        local_tombstones
            .iter()
            .any(|tombstone| tombstone.path == "draft")
    );
    assert!(
        local_tombstones
            .iter()
            .any(|tombstone| tombstone.path == "draft/child.txt")
    );

    let merged_root = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap()
        .root_cid;
    assert!(
        daemon
            .tree()
            .resolve(&merged_root, "draft")
            .await
            .unwrap()
            .is_none(),
        "a causally newer ordinary directory delete must not be mistaken for a kind replacement"
    );
}
