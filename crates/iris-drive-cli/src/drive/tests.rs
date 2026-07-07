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
            base_root_cid: None,
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
fn provider_compose_path_does_not_require_live_daemon() {
    let config_dir = tempfile::tempdir().unwrap();
    init_config(config_dir.path());

    cmd_provider(
        config_dir.path(),
        ProviderCmd::ComposePath {
            parent_path: "Reports".into(),
            display_name: "../Quarter:1/report.txt".into(),
        },
    )
    .unwrap();
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
            base_root_cid: None,
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
            base_root_cid: None,
            path: "juuh.txt".into(),
        },
    )
    .unwrap();
    cmd_provider(
        config_dir.path(),
        ProviderCmd::Delete {
            base_root_cid: None,
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
            base_root_cid: None,
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
            base_root_cid: None,
            path: "photo.png".into(),
            source: real_source.clone(),
        },
    )
    .unwrap();
    cmd_provider(
        config_dir.path(),
        ProviderCmd::Write {
            base_root_cid: None,
            path: "photo copy (2).png".into(),
            source: empty_source.clone(),
        },
    )
    .unwrap();

    let error = cmd_provider(
        config_dir.path(),
        ProviderCmd::Write {
            base_root_cid: None,
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
fn provider_writes_with_same_stale_base_accumulate_all_files() {
    let config_dir = tempfile::tempdir().unwrap();
    init_config(config_dir.path());
    mark_daemon_live(config_dir.path());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let base_root = runtime.block_on(async {
        let daemon = Daemon::open(config_dir.path()).unwrap();
        daemon.tree().put_directory(Vec::new()).await.unwrap()
    });
    let source_dir = tempfile::tempdir().unwrap();

    for index in 1..=4 {
        let source = source_dir.path().join(format!("file-{index}.txt"));
        std::fs::write(&source, format!("file {index}")).unwrap();
        cmd_provider(
            config_dir.path(),
            ProviderCmd::Write {
                base_root_cid: Some(base_root.to_string()),
                path: format!("file-{index}.txt"),
                source,
            },
        )
        .unwrap();
    }

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
        assert_eq!(
            paths,
            vec!["file-1.txt", "file-2.txt", "file-3.txt", "file-4.txt"]
        );
    });
}

#[test]
fn provider_duplicate_replace_batch_does_not_publish_intermediate_deletes() {
    let config_dir = tempfile::tempdir().unwrap();
    init_config(config_dir.path());
    mark_daemon_live(config_dir.path());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let source_dir = tempfile::tempdir().unwrap();
    for index in 1..=7 {
        std::fs::write(
            source_dir.path().join(format!("file-{index}.txt")),
            format!("original {index}"),
        )
        .unwrap();
    }
    cmd_import(config_dir.path(), source_dir.path()).unwrap();
    let base_root = runtime.block_on(async {
        let daemon = Daemon::open(config_dir.path()).unwrap();
        iris_drive_core::primary_merged_root(daemon.tree(), daemon.config())
            .await
            .unwrap()
            .root_cid
    });
    let listener = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    std::fs::write(
        iris_drive_core::paths::provider_root_wake_path_in(config_dir.path()),
        serde_json::to_vec(&json!({ "port": port })).unwrap(),
    )
    .unwrap();
    let wake_reader = std::thread::spawn(move || {
        use std::io::Read as _;

        for _ in 0..12 {
            let (mut stream, _) = listener.accept().unwrap();
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).unwrap();
        }
    });
    let pasted_dir = tempfile::tempdir().unwrap();
    for name in ["new-a.txt", "new-b.txt"] {
        let source = pasted_dir.path().join(name);
        std::fs::write(&source, format!("new {name}")).unwrap();
        cmd_provider(
            config_dir.path(),
            ProviderCmd::Write {
                base_root_cid: Some(base_root.to_string()),
                path: name.to_string(),
                source,
            },
        )
        .unwrap();
    }
    for index in 1..=5 {
        cmd_provider(
            config_dir.path(),
            ProviderCmd::Delete {
                base_root_cid: Some(base_root.to_string()),
                path: format!("file-{index}.txt"),
            },
        )
        .unwrap();
    }
    for index in 1..=5 {
        let source = pasted_dir.path().join(format!("replacement-{index}.txt"));
        std::fs::write(&source, format!("replacement {index}")).unwrap();
        cmd_provider(
            config_dir.path(),
            ProviderCmd::Write {
                base_root_cid: Some(base_root.to_string()),
                path: format!("file-{index}.txt"),
                source,
            },
        )
        .unwrap();
    }
    wake_reader.join().unwrap();

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
        assert_eq!(
            paths,
            vec![
                "file-1.txt",
                "file-2.txt",
                "file-3.txt",
                "file-4.txt",
                "file-5.txt",
                "file-6.txt",
                "file-7.txt",
            ],
            "provider replacement batch should not publish partial roots before daemon coalescing"
        );
    });

    runtime.block_on(async {
        let staged = crate::provider_staging::read_provider_staging(config_dir.path())
            .unwrap()
            .expect("replacement batch should leave a staged provider root");
        let mut daemon = Daemon::open(config_dir.path()).unwrap();
        daemon
            .import_visible_root_with_tombstone_base_and_paths(
                staged.root().unwrap(),
                staged.tombstone_base_root().unwrap(),
                Some(&staged.tombstone_paths),
            )
            .await
            .unwrap();
        crate::provider_staging::clear_provider_staging(config_dir.path()).unwrap();
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
        assert_eq!(
            paths,
            vec![
                "file-1.txt",
                "file-2.txt",
                "file-3.txt",
                "file-4.txt",
                "file-5.txt",
                "file-6.txt",
                "file-7.txt",
                "new-a.txt",
                "new-b.txt",
            ],
            "coalesced replacement batch should publish the final provider root"
        );
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
            base_root_cid: None,
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
fn provider_write_does_not_tombstone_unrelated_missing_peer_file() {
    let config_dir = tempfile::tempdir().unwrap();
    let (_account, remote, remote_meta) = init_config_with_remote_device(config_dir.path());
    mark_daemon_live(config_dir.path());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let (remote_root, empty_root) = runtime.block_on(async {
        let daemon = Daemon::open(config_dir.path()).unwrap();
        let remote_dir = tempfile::tempdir().unwrap();
        std::fs::write(remote_dir.path().join("foreign.txt"), b"from remote").unwrap();
        let remote_root = iris_drive_core::indexer::index_dir_with_history_and_meta(
            daemon.tree(),
            remote_dir.path(),
            None,
            remote_meta.created_at,
            Some(&remote_meta),
        )
        .await
        .unwrap();
        let empty_root = daemon.tree().put_directory(Vec::new()).await.unwrap();
        (remote_root, empty_root)
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

    runtime.block_on(async {
        let mut daemon = Daemon::open(config_dir.path()).unwrap();
        daemon
            .materialize_primary_merged_root()
            .await
            .unwrap()
            .expect("local cache root should include the remote file");
    });

    let source_dir = tempfile::tempdir().unwrap();
    let source = source_dir.path().join("local.txt");
    std::fs::write(&source, b"from local").unwrap();

    cmd_provider(
        config_dir.path(),
        ProviderCmd::Write {
            base_root_cid: Some(empty_root.to_string()),
            path: "local.txt".into(),
            source,
        },
    )
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
        assert!(
            paths.contains(&"foreign.txt"),
            "unrelated peer file should not be tombstoned by provider write: {merged:#?}"
        );
        assert!(paths.contains(&"local.txt"));
        assert!(
            !merged
                .view
                .suppressed_by_tombstone
                .iter()
                .any(|path| path == "foreign.txt"),
            "foreign file was suppressed by an unrelated provider write: {merged:#?}"
        );
    });
}

#[test]
fn provider_import_tombstone_scope_preserves_unrelated_base_file() {
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

    runtime.block_on(async {
        let mut daemon = Daemon::open(config_dir.path()).unwrap();
        let base = iris_drive_core::primary_merged_root(daemon.tree(), daemon.config())
            .await
            .unwrap()
            .root_cid;
        let (local_hash, local_size) = daemon.tree().put(b"from local").await.unwrap();
        let edited = daemon
            .tree()
            .set_entry(
                &daemon.tree().put_directory(Vec::new()).await.unwrap(),
                &[],
                "local.txt",
                &local_hash,
                local_size,
                hashtree_core::LinkType::Blob,
            )
            .await
            .unwrap();
        let tombstone_paths = BTreeSet::from(["local.txt".to_string()]);
        provider_retry::import_provider_root_with_retry(
            &mut daemon,
            edited,
            Some(base),
            Some(&tombstone_paths),
        )
        .await
        .unwrap();

        let merged = iris_drive_core::primary_merged_view(daemon.tree(), daemon.config())
            .await
            .unwrap();
        let paths = merged
            .view
            .files
            .iter()
            .map(|entry| entry.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["foreign.txt", "local.txt"]);
        assert!(
            !merged
                .view
                .suppressed_by_tombstone
                .iter()
                .any(|path| path == "foreign.txt"),
            "unrelated base file should not be tombstoned: {merged:#?}"
        );
    });
}

#[test]
fn provider_import_retries_windows_transient_missing_store_reads() {
    assert!(provider_retry::provider_import_error_message_is_retryable(
        "index: tree: Store error: IO error: The system cannot find the file specified. (os error 2)"
    ));
    assert!(provider_retry::provider_import_error_message_is_retryable(
        "index: tree: Store error: IO error: No such file or directory (os error 2)"
    ));
    assert!(provider_retry::provider_import_error_message_is_retryable(
        "index: tree: Missing chunk: abc123"
    ));
    assert!(provider_retry::provider_import_error_message_is_retryable(
        "local store is missing provider root block abc123"
    ));
    assert!(!provider_retry::provider_import_error_message_is_retryable(
        "config: invalid json"
    ));
}

#[test]
fn provider_import_retries_long_enough_for_peer_root_warmup() {
    let retry_budget_ms: u64 = provider_retry::PROVIDER_IMPORT_RETRY_DELAYS_MS.iter().sum();

    assert!(
        retry_budget_ms >= 60_000,
        "provider root warmup retry budget was only {retry_budget_ms}ms"
    );
    assert!(
        provider_retry::PROVIDER_IMPORT_RETRY_DELAYS_MS
            .iter()
            .all(|delay| *delay <= 16_000),
        "individual provider retry sleeps should stay bounded"
    );
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
fn provider_modified_at_index_projects_child_timestamps_to_directories() {
    let view = iris_drive_core::projection::PrimaryMergedView {
        view: iris_drive_core::merge::MergedView {
            files: vec![
                iris_drive_core::merge::MergedEntry {
                    path: "Reports/nested.txt".to_string(),
                    source_path: None,
                    hash: [1; 32],
                    size: 12,
                    whole_file_hash: None,
                    modified_at: Some(1_700_000_000),
                    source_app_key_pubkey: "app-key".to_string(),
                    published_at: 1_700_000_001,
                },
                iris_drive_core::merge::MergedEntry {
                    path: "Reports/epoch.txt".to_string(),
                    source_path: None,
                    hash: [2; 32],
                    size: 12,
                    whole_file_hash: None,
                    modified_at: Some(0),
                    source_app_key_pubkey: "app-key".to_string(),
                    published_at: 1_700_000_001,
                },
            ],
            ..Default::default()
        },
        authorized_app_keys: 1,
        app_key_roots_present: 1,
    };

    let index = provider_modified_at_index(&view);

    assert_eq!(index.get("Reports/nested.txt"), Some(&1_700_000_000));
    assert_eq!(index.get("Reports"), Some(&1_700_000_000));
    assert_eq!(index.get("Reports/epoch.txt"), None);
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
