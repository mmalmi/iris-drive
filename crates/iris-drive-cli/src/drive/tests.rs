use super::*;
use iris_drive_core::root_meta::DriveRootMeta;

fn init_config(config_dir: &Path) -> Account {
    let account = Account::create(config_dir, Some("local".into())).unwrap();
    let mut config = AppConfig {
        account: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(account.state.owner_pubkey.clone()));
    config.save(config_path_in(config_dir)).unwrap();
    account
}

fn init_config_with_remote_device(config_dir: &Path) -> (Account, String, DriveRootMeta) {
    let account = init_config(config_dir);
    let remote =
        iris_drive_core::identity::Identity::generate(config_dir.join("remote.key")).pubkey_hex();
    let mut config = AppConfig::load_or_default(config_path_in(config_dir)).unwrap();
    let state = config.account.as_mut().unwrap();
    state
        .app_keys
        .as_mut()
        .unwrap()
        .devices
        .push(iris_drive_core::app_keys::DeviceEntry::member(
            remote.clone(),
            100,
            Some("remote".into()),
        ));
    state.app_keys.as_mut().unwrap().normalize();
    config.save(config_path_in(config_dir)).unwrap();

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
    (account, remote, remote_meta)
}

#[test]
fn provider_delete_local_file_is_idempotent() {
    let config_dir = tempfile::tempdir().unwrap();
    init_config(config_dir.path());
    let source_dir = tempfile::tempdir().unwrap();
    std::fs::write(source_dir.path().join("juuh.txt"), b"delete me").unwrap();
    cmd_import(config_dir.path(), source_dir.path()).unwrap();

    cmd_provider(
        config_dir.path(),
        ProviderCmd::Delete {
            path: "juuh.txt".into(),
        },
    )
    .unwrap();
    cmd_provider(
        config_dir.path(),
        ProviderCmd::Delete {
            path: "juuh.txt".into(),
        },
    )
    .unwrap();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let daemon = Daemon::open(config_dir.path()).unwrap();
        let merged = iris_drive_core::primary_merged_view(daemon.tree(), daemon.config())
            .await
            .unwrap();
        assert!(
            merged
                .view
                .files
                .iter()
                .all(|entry| entry.path != "juuh.txt"),
            "deleted file should not reappear in the merged view"
        );
    });
}

#[test]
fn provider_delete_directory_removes_tree() {
    let config_dir = tempfile::tempdir().unwrap();
    init_config(config_dir.path());
    let source_dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(source_dir.path().join("folder")).unwrap();
    std::fs::write(
        source_dir.path().join("folder").join("child.txt"),
        b"delete me",
    )
    .unwrap();
    cmd_import(config_dir.path(), source_dir.path()).unwrap();

    cmd_provider(
        config_dir.path(),
        ProviderCmd::Delete {
            path: "folder".into(),
        },
    )
    .unwrap();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let daemon = Daemon::open(config_dir.path()).unwrap();
        let merged = iris_drive_core::primary_merged_view(daemon.tree(), daemon.config())
            .await
            .unwrap();
        assert!(
            merged
                .view
                .files
                .iter()
                .all(|entry| !entry.path.starts_with("folder/")),
            "deleted directory children should not remain in the merged view"
        );
    });
}

#[test]
fn provider_delete_tombstones_foreign_visible_files() {
    let config_dir = tempfile::tempdir().unwrap();
    let (_account, remote, remote_meta) = init_config_with_remote_device(config_dir.path());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let remote_root = runtime.block_on(async {
        let daemon = Daemon::open(config_dir.path()).unwrap();
        let remote_dir = tempfile::tempdir().unwrap();
        std::fs::write(remote_dir.path().join("foreign.txt"), b"from remote").unwrap();
        iris_drive_core::indexer::index_dir_with_history_and_meta(
            daemon.tree(),
            remote_dir.path(),
            None,
            remote_meta.created_at,
            Some(&remote_meta),
        )
        .await
        .unwrap()
    });

    let mut config = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    let mut drive = config.drive(PRIMARY_DRIVE_ID).unwrap().clone();
    drive.device_roots.insert(
        remote.clone(),
        DeviceRootRef::from_meta(
            remote_root.to_string(),
            remote_meta.created_at,
            &remote_meta,
        ),
    );
    config.upsert_drive(drive);
    config.save(config_path_in(config_dir.path())).unwrap();

    cmd_provider(
        config_dir.path(),
        ProviderCmd::Delete {
            path: "foreign.txt".into(),
        },
    )
    .unwrap();

    runtime.block_on(async {
        let daemon = Daemon::open(config_dir.path()).unwrap();
        let merged = iris_drive_core::primary_merged_view(daemon.tree(), daemon.config())
            .await
            .unwrap();
        assert!(
            merged
                .view
                .files
                .iter()
                .all(|entry| entry.path != "foreign.txt")
        );
        assert_eq!(
            merged.view.suppressed_by_tombstone,
            vec!["foreign.txt".to_string()]
        );
    });
}

#[test]
fn provider_import_retries_windows_transient_missing_store_reads() {
    assert!(provider_import_error_message_is_retryable(
        "index: tree: Store error: IO error: The system cannot find the file specified. (os error 2)"
    ));
    assert!(provider_import_error_message_is_retryable(
        "index: tree: Store error: IO error: No such file or directory (os error 2)"
    ));
    assert!(provider_import_error_message_is_retryable(
        "index: tree: Missing chunk: abc123"
    ));
    assert!(!provider_import_error_message_is_retryable(
        "config: invalid json"
    ));
}
