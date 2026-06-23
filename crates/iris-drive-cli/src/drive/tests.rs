use super::*;
use iris_drive_core::root_meta::DriveRootMeta;

fn init_config(config_dir: &Path) -> Profile {
    let account = Profile::create(config_dir, Some("local".into())).unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(account.state.root_scope_id()));
    config.save(config_path_in(config_dir)).unwrap();
    account
}

fn init_config_with_remote_device(config_dir: &Path) -> (Profile, String, DriveRootMeta) {
    let mut account = init_config(config_dir);
    let remote =
        iris_drive_core::identity::Identity::generate(config_dir.join("remote.key")).pubkey_hex();
    account
        .approve_app_key(&remote, Some("remote".into()))
        .unwrap();
    let mut config = AppConfig::load_or_default(config_path_in(config_dir)).unwrap();
    config.profile = Some(account.state.clone());
    config.save(config_path_in(config_dir)).unwrap();

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
    (account, remote, remote_meta)
}

fn mark_daemon_live(config_dir: &Path) {
    std::fs::write(
        iris_drive_core::daemon_liveness::daemon_lock_path(config_dir),
        format!("{}\n", std::process::id()),
    )
    .unwrap();
}

#[test]
fn provider_mutation_requires_live_daemon() {
    let config_dir = tempfile::tempdir().unwrap();
    init_config(config_dir.path());

    let error = cmd_provider(
        config_dir.path(),
        ProviderCmd::Mkdir {
            path: "Reports".into(),
        },
    )
    .expect_err("provider mutation should fail when daemon is unavailable");

    assert!(
        error.to_string().contains("daemon is unavailable"),
        "unexpected error: {error:#}"
    );
}

#[test]
fn provider_mutation_accepts_fresh_daemon_status_when_pid_probe_fails() {
    let config_dir = tempfile::tempdir().unwrap();
    init_config(config_dir.path());
    std::fs::write(
        iris_drive_core::daemon_liveness::daemon_lock_path(config_dir.path()),
        "99999999\n",
    )
    .unwrap();
    std::fs::write(
        config_dir.path().join("daemon-status.json"),
        format!(
            r#"{{"pid":99999999,"running":true,"fresh":true,"updated_at":{}}}"#,
            unix_now_seconds()
        ),
    )
    .unwrap();

    cmd_provider(
        config_dir.path(),
        ProviderCmd::Mkdir {
            path: "Reports".into(),
        },
    )
    .unwrap();
}

#[test]
fn provider_delete_local_file_is_idempotent() {
    let config_dir = tempfile::tempdir().unwrap();
    init_config(config_dir.path());
    mark_daemon_live(config_dir.path());
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

fn unix_now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[test]
fn provider_delete_directory_removes_tree() {
    let config_dir = tempfile::tempdir().unwrap();
    init_config(config_dir.path());
    mark_daemon_live(config_dir.path());
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
fn provider_write_rejects_probable_os_placeholder_collision() {
    let config_dir = tempfile::tempdir().unwrap();
    init_config(config_dir.path());
    mark_daemon_live(config_dir.path());
    let source_dir = tempfile::tempdir().unwrap();
    let real_source = source_dir.path().join("real.png");
    std::fs::write(&real_source, b"real image bytes").unwrap();
    let empty_source = source_dir.path().join("empty.png");
    std::fs::write(&empty_source, b"").unwrap();

    cmd_provider(
        config_dir.path(),
        ProviderCmd::Write {
            path: "photo.png".into(),
            source: real_source.clone(),
        },
    )
    .unwrap();
    cmd_provider(
        config_dir.path(),
        ProviderCmd::Write {
            path: "photo copy (2).png".into(),
            source: empty_source.clone(),
        },
    )
    .unwrap();

    let error = cmd_provider(
        config_dir.path(),
        ProviderCmd::Write {
            path: "photo copy (2) (2).png".into(),
            source: empty_source,
        },
    )
    .expect_err("runaway zero-byte FileProvider placeholder should be rejected");
    assert!(
        error.to_string().contains("placeholder copy"),
        "unexpected error: {error:#}"
    );

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let daemon = Daemon::open(config_dir.path()).unwrap();
        let merged = iris_drive_core::primary_merged_view(daemon.tree(), daemon.config())
            .await
            .unwrap();
        let paths = merged
            .view
            .files
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["photo copy (2).png", "photo.png"]);
    });
}

#[test]
fn provider_delete_tombstones_foreign_visible_files() {
    let config_dir = tempfile::tempdir().unwrap();
    let (_account, remote, remote_meta) = init_config_with_remote_device(config_dir.path());
    mark_daemon_live(config_dir.path());
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
    drive.app_key_roots.insert(
        remote.clone(),
        AppKeyRootRef::from_meta(
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

#[test]
fn provider_list_entry_serializes_core_path_metadata() {
    let entry = ProviderListEntry {
        path: "Reports/nested.txt".to_string(),
        parent_path: "Reports".to_string(),
        display_name: "nested.txt".to_string(),
        kind: "file",
        size: 12,
        version: "root".to_string(),
        modified_at: None,
    };

    let value = serde_json::to_value(entry).unwrap();

    assert_eq!(value["parent_path"], "Reports");
    assert_eq!(value["display_name"], "nested.txt");
}

#[test]
fn provider_resolve_path_reports_collision_display_name() {
    let entries = vec![ProviderListEntry {
        path: "Reports/Shared_file.txt".to_string(),
        parent_path: "Reports".to_string(),
        display_name: "Shared_file.txt".to_string(),
        kind: "file",
        size: 5,
        version: "root".to_string(),
        modified_at: None,
    }];

    let path = unique_provider_path(&entries, "Reports", "Shared_file.txt", None);
    let (parent_path, display_name) = split_provider_path(&path).unwrap();

    assert_eq!(parent_path, "Reports");
    assert_eq!(display_name, "Shared_file (2).txt");
}

#[test]
fn provider_path_normalization_rejects_native_separator_aliases() {
    assert!(normalize_provider_path("Reports\\note.txt").is_err());
    assert!(normalize_provider_path("Reports:note.txt").is_err());
}
