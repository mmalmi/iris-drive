use super::*;

#[tokio::test]
async fn materialize_primary_merged_root_converges_accepted_remote_files() {
    let cfg_dir = tempdir().unwrap();
    let account = init_config_with_account(cfg_dir.path());
    let remote = Identity::generate(cfg_dir.path().join("remote.key")).pubkey_hex();
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

    let local_dir = tempdir().unwrap();
    std::fs::write(local_dir.path().join("mac.txt"), b"from mac").unwrap();
    daemon.import_source_dir(local_dir.path()).await.unwrap();

    let remote_dir = tempdir().unwrap();
    std::fs::write(remote_dir.path().join("pixel.txt"), b"from pixel").unwrap();
    let remote_meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.into(),
        device_id: remote.clone(),
        device_seq: 1,
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
    let state = config.account.as_mut().unwrap();
    state
        .app_keys
        .as_mut()
        .unwrap()
        .devices
        .push(DeviceEntry::member(
            remote.clone(),
            100,
            Some("Pixel".into()),
        ));
    state.app_keys.as_mut().unwrap().normalize();
    let mut drive = config.drive(PRIMARY_DRIVE_ID).unwrap().clone();
    drive.device_roots.insert(
        remote.clone(),
        DeviceRootRef::from_meta(remote_root.to_string(), 100, &remote_meta),
    );
    config.upsert_drive(drive);
    config.save(config_path_in(cfg_dir.path())).unwrap();

    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    let report = daemon
        .materialize_primary_merged_root()
        .await
        .unwrap()
        .expect("merged root should become this device's next root");
    assert_eq!(report.file_count, 2);

    let root = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .device_roots
        .get(&account.state.device_pubkey)
        .unwrap();
    assert_eq!(root.device_seq, 2);
    assert!(root.local_only);
    let root_cid = Cid::parse(&root.root_cid).unwrap();
    let meta = crate::indexer::read_root_meta(daemon.tree(), &root_cid)
        .await
        .unwrap()
        .expect("materialized root has causal metadata");
    assert!(meta.local_only);
    assert_eq!(
        meta.observed
            .get(&remote)
            .map(|observed| observed.root_cid.clone()),
        Some(remote_root.to_string())
    );
    let (files, _) = crate::merge::walk_device_tree(daemon.tree(), &root_cid)
        .await
        .unwrap();
    assert_eq!(
        files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>(),
        vec!["mac.txt", "pixel.txt"]
    );

    let mut converged = daemon.config().clone();
    converged
        .account
        .as_mut()
        .unwrap()
        .app_keys
        .as_mut()
        .unwrap()
        .devices
        .retain(|device| device.pubkey != remote);
    converged
        .account
        .as_mut()
        .unwrap()
        .app_keys
        .as_mut()
        .unwrap()
        .normalize();
    converged
        .drives
        .iter_mut()
        .find(|drive| drive.drive_id == PRIMARY_DRIVE_ID)
        .unwrap()
        .device_roots
        .remove(&remote);
    converged.save(config_path_in(cfg_dir.path())).unwrap();

    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let view = crate::primary_merged_view(daemon.tree(), daemon.config())
        .await
        .unwrap();
    assert_eq!(
        view.view
            .files
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>(),
        vec!["mac.txt", "pixel.txt"]
    );
}
