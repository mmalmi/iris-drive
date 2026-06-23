use super::*;

async fn fresh_test_provider() -> (tempfile::TempDir, HashTreeProviderFs<FsBlobStore>) {
    let dir = tempfile::tempdir().unwrap();
    let store = FsBlobStore::new(dir.path()).unwrap();
    let tree = Arc::new(HashTree::new(HashTreeConfig::new(Arc::new(store)).public()));
    let provider = HashTreeProviderFs::fresh(tree).await.unwrap();
    (dir, provider)
}

#[test]
fn live_block_pull_tries_blossom_after_fips_failure() {
    let mut config = AppConfig {
        blossom_servers: vec!["https://upload.example".to_string()],
        ..AppConfig::default()
    };

    assert!(should_try_blossom_download(&config, true, true));
    assert!(should_try_blossom_download(&config, true, false));
    assert!(should_try_blossom_download(&config, false, false));

    config.blossom_servers.clear();
    assert!(!should_try_blossom_download(&config, false, false));
}

#[test]
fn config_root_watch_filters_to_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    let event = notify::Event {
        kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any,
        )),
        paths: vec![config.clone()],
        attrs: notify::event::EventAttributes::new(),
    };
    let other = notify::Event {
        kind: notify::EventKind::Modify(notify::event::ModifyKind::Data(
            notify::event::DataChange::Any,
        )),
        paths: vec![dir.path().join("daemon-status.json")],
        attrs: notify::event::EventAttributes::new(),
    };

    assert!(event_touches_path(&event, &config));
    assert!(!event_touches_path(&other, &config));
}

#[test]
fn daemon_status_records_binary_version_for_gui_mismatch_detection() {
    let dir = tempfile::tempdir().unwrap();

    let status = write_daemon_status(dir.path(), json!({"event": "test"}));

    assert_eq!(status["binary_version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn root_update_debounce_has_fast_floor() {
    assert_eq!(
        root_update_debounce_duration(100),
        std::time::Duration::from_millis(150)
    );
    assert_eq!(
        root_update_debounce_duration(2_500),
        std::time::Duration::from_millis(2_500)
    );
}

#[test]
fn provider_root_poll_remains_safety_sweep_when_config_watch_is_active() {
    assert!(provider_root_poll_enabled(true));
    assert!(provider_root_poll_enabled(false));
    assert_eq!(
        provider_root_poll_period(0),
        std::time::Duration::from_secs(1)
    );
    assert_eq!(
        provider_root_poll_period(60),
        std::time::Duration::from_mins(1)
    );
}

#[test]
fn provider_root_publish_cache_matches_fingerprint_and_root_key() {
    let fingerprint = ConfigFileFingerprint {
        len: 10,
        modified: None,
    };
    let other_fingerprint = ConfigFileFingerprint {
        len: 11,
        modified: None,
    };
    let root_key = Some("primary:device:root-a".to_string());
    let other_root_key = Some("primary:device:root-b".to_string());
    let mut cache = ProviderRootPublishCache::default();

    assert!(!cache.is_current(&fingerprint, &root_key));
    cache.update(fingerprint.clone(), root_key.clone());

    assert!(cache.is_current(&fingerprint, &root_key));
    assert!(!cache.is_current(&other_fingerprint, &root_key));
    assert!(!cache.is_current(&fingerprint, &other_root_key));
}

#[test]
fn stale_root_apply_followup_detects_superseded_app_key_root() {
    let config_dir = tempfile::tempdir().unwrap();
    let mut drive = Drive {
        root_scope_id: iris_drive_core::IrisProfileId::new_v4().to_string(),
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        display_name: "My Drive".to_string(),
        role: DriveRole::Owner,
        app_key_roots: BTreeMap::new(),
        last_root_cid: None,
        key_hex: None,
    };
    drive.app_key_roots.insert(
        "device-a".to_string(),
        AppKeyRootRef::legacy("root-a", 10, 1),
    );
    drive.app_key_roots.insert(
        "device-b".to_string(),
        AppKeyRootRef::legacy("remote-root-a", 11, 1),
    );
    let mut config = AppConfig {
        profile: Some(ProfileState {
            profile_id: iris_drive_core::IrisProfileId::new_v4(),
            app_key_pubkey: "device-a".to_string(),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".to_string(),
            authorization_state: iris_drive_core::AppKeyAuthorizationState::Authorized,
            app_key_label: None,
            app_keys: None,
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
        }),
        drives: vec![drive],
        ..AppConfig::default()
    };
    config
        .save(config_path_in(config_dir.path()))
        .expect("save initial config");
    let stale_key = root_apply_followup_key(&config, Some("remote-root-a"), true)
        .expect("initial followup key");

    config
        .drives
        .get_mut(0)
        .unwrap()
        .app_key_roots
        .get_mut("device-a")
        .unwrap()
        .root_cid = "root-b".to_string();
    config
        .save(config_path_in(config_dir.path()))
        .expect("save unrelated local update");
    assert!(!root_apply_followup_is_stale(
        config_dir.path(),
        Some(&stale_key)
    ));

    config
        .drives
        .get_mut(0)
        .unwrap()
        .app_key_roots
        .get_mut("device-b")
        .unwrap()
        .root_cid = "remote-root-b".to_string();
    config
        .save(config_path_in(config_dir.path()))
        .expect("save updated config");
    let current_key = root_apply_followup_key(&config, Some("remote-root-b"), true)
        .expect("updated followup key");

    assert!(root_apply_followup_is_stale(
        config_dir.path(),
        Some(&stale_key)
    ));
    assert!(!root_apply_followup_is_stale(
        config_dir.path(),
        Some(&current_key)
    ));
    assert!(!root_apply_followup_is_stale(config_dir.path(), None));
}

#[tokio::test]
async fn pending_mount_update_drain_keeps_latest_after_debounce() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let first = Cid::public([1; 32]);
    let second = Cid::public([2; 32]);
    let third = Cid::public([3; 32]);
    tx.send(second.clone()).unwrap();
    let delayed = tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        delayed.send(third.clone()).unwrap();
    });

    let drained = drain_latest_mount_root_update(
        &mut rx,
        std::time::Duration::from_millis(25),
        Some(first),
    )
    .await;

    assert_eq!(drained, Some(Cid::public([3; 32])));
}

#[tokio::test]
async fn config_mutation_lock_serializes_same_config_dir() {
    let dir = tempfile::tempdir().unwrap();
    let first = ConfigMutationLock::acquire(dir.path()).await.unwrap();
    let contender_dir = dir.path().to_path_buf();
    let contender =
        tokio::spawn(async move { ConfigMutationLock::acquire(&contender_dir).await });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(!contender.is_finished());
    drop(first);

    let second = tokio::time::timeout(std::time::Duration::from_secs(2), contender)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    drop(second);
}

#[test]
fn event_block_pull_retry_budget_stays_short_without_blossom() {
    let config = AppConfig {
        blossom_servers: Vec::new(),
        ..AppConfig::default()
    };
    let attempts = event_block_pull_retry_delays(&config).len() as u64;
    let retry_sleep: u64 = event_block_pull_retry_delays(&config).iter().sum();

    assert!(attempts * event_block_pull_timeout_secs(&config) + retry_sleep <= 12);
}

#[test]
fn event_block_pull_timeout_allows_blossom_retry_window() {
    let config = AppConfig {
        blossom_servers: vec!["http://127.0.0.1:12345".to_string()],
        ..AppConfig::default()
    };

    assert_eq!(event_block_pull_retry_delays(&config), &[0]);
    assert!(
        event_block_pull_timeout_secs(&config)
            > FIPS_DOWNLOAD_BEFORE_BLOSSOM_ATTEMPT_TIMEOUT_SECS
                + BLOSSOM_DOWNLOAD_RETRY_DELAYS.iter().sum::<u64>()
    );
}

#[test]
fn windows_cloud_file_read_skip_only_ignores_transient_placeholder_errors() {
    assert!(windows_cloud_file_read_should_skip(&std::io::Error::new(
        std::io::ErrorKind::PermissionDenied,
        "hydrating placeholder"
    )));
    assert!(windows_cloud_file_read_should_skip(
        &std::io::Error::from_raw_os_error(395)
    ));
    assert!(windows_cloud_file_read_should_skip(
        &std::io::Error::from_raw_os_error(362)
    ));
    assert!(windows_cloud_file_read_should_skip(
        &std::io::Error::from_raw_os_error(404)
    ));
    assert!(!windows_cloud_file_read_should_skip(&std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "real missing file"
    )));
}

#[test]
fn windows_cloud_rescan_detects_deleted_cached_placeholder_paths() {
    let sync_root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(sync_root.path().join("present")).unwrap();
    std::fs::write(sync_root.path().join("present").join("file.txt"), b"keep").unwrap();
    let cached = BTreeSet::from([
        "gone".to_string(),
        "gone/child.txt".to_string(),
        "gone.txt".to_string(),
        "present".to_string(),
        "present/file.txt".to_string(),
    ]);

    let missing =
        windows_cloud_missing_cached_provider_paths(sync_root.path(), &cached).unwrap();

    assert_eq!(
        missing,
        vec![
            "gone".to_string(),
            "gone.txt".to_string(),
            "gone/child.txt".to_string(),
        ]
    );
}

#[test]
fn windows_cloud_rescan_without_delete_recovery_ignores_cached_projection_misses() {
    let sync_root = tempfile::tempdir().unwrap();
    let cached = BTreeSet::from(["remote-lag.txt".to_string()]);

    let missing =
        windows_cloud_rescan_missing_cached_provider_paths(sync_root.path(), &cached, false)
            .unwrap();

    assert!(missing.is_empty());
}

#[test]
fn windows_cloud_cached_delete_recovery_detects_projection_misses() {
    let sync_root = tempfile::tempdir().unwrap();
    let cached = BTreeSet::from(["remote-gone.txt".to_string()]);

    let missing =
        windows_cloud_rescan_missing_cached_provider_paths(sync_root.path(), &cached, true)
            .unwrap();

    assert_eq!(missing, vec!["remote-gone.txt".to_string()]);
}

#[test]
fn windows_cloud_periodic_validation_is_narrow_local_state_check() {
    assert!(matches!(
        windows_cloud_periodic_validate_change(),
        WindowsCloudRootChange::ValidateLocalState
    ));
}

#[test]
fn windows_cloud_detects_missing_previous_local_state_after_rename_to_event() {
    let sync_root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(sync_root.path().join("renames")).unwrap();
    std::fs::write(sync_root.path().join("renames").join("dst.txt"), b"renamed").unwrap();
    let previous_state = vec![
        WindowsCloudLocalStateEntry {
            path: "renames/src.txt".to_string(),
            kind: "file".to_string(),
            size: 7,
            sha256: Some(to_hex(&hashtree_core::sha256(b"renamed"))),
            provider_version: None,
        },
        WindowsCloudLocalStateEntry {
            path: "renames/dst.txt".to_string(),
            kind: "file".to_string(),
            size: 7,
            sha256: Some(to_hex(&hashtree_core::sha256(b"renamed"))),
            provider_version: None,
        },
    ];
    let protected_paths = BTreeSet::from(["renames/dst.txt".to_string()]);

    let missing = windows_cloud_missing_previous_local_state_paths(
        sync_root.path(),
        &previous_state,
        &protected_paths,
    )
    .unwrap();

    assert_eq!(missing, vec!["renames/src.txt".to_string()]);
}

#[test]
fn windows_cloud_recreated_placeholder_does_not_count_as_missing_local_file() {
    let projected = WindowsCloudLocalStateEntry {
        path: "renames/src.txt".to_string(),
        kind: "file".to_string(),
        size: 7,
        sha256: Some(to_hex(&hashtree_core::sha256(b"renamed"))),
        provider_version: None,
    };
    let placeholder = WindowsCloudLocalStateEntry {
        path: "renames/src.txt".to_string(),
        kind: "file".to_string(),
        size: 7,
        sha256: None,
        provider_version: None,
    };
    let directory = WindowsCloudLocalStateEntry {
        path: "renames".to_string(),
        kind: "directory".to_string(),
        size: 0,
        sha256: None,
        provider_version: None,
    };

    assert!(
        !windows_cloud_previous_local_state_reparse_counts_as_missing(&projected, true)
    );
    assert!(
        !windows_cloud_previous_local_state_reparse_counts_as_missing(&projected, false)
    );
    assert!(!windows_cloud_previous_local_state_reparse_counts_as_missing(&placeholder, true));
    assert!(!windows_cloud_previous_local_state_reparse_counts_as_missing(&directory, true));
}

#[test]
fn windows_cloud_cleanup_delete_marker_suppresses_delete_once() {
    let config_dir = tempfile::tempdir().unwrap();
    let now_ms = windows_cloud_cleanup_marker_now_ms();
    std::fs::write(
        config_dir.path().join(WINDOWS_CLOUD_CLEANUP_DELETE_FILE),
        serde_json::to_string(&json!({
            "entries": [
                {
                    "path": "codex-lab/run/from-windows.txt",
                    "created_at_unix_ms": now_ms,
                },
                {
                    "path": "keep.txt",
                    "created_at_unix_ms": now_ms,
                },
            ],
        }))
        .unwrap(),
    )
    .unwrap();

    assert!(consume_windows_cloud_cleanup_delete_marker(
        config_dir.path(),
        "codex-lab/run/from-windows.txt",
    ));
    assert!(!consume_windows_cloud_cleanup_delete_marker(
        config_dir.path(),
        "codex-lab/run/from-windows.txt",
    ));

    let raw =
        std::fs::read_to_string(config_dir.path().join(WINDOWS_CLOUD_CLEANUP_DELETE_FILE))
            .unwrap();
    let value: Value = serde_json::from_str(&raw).unwrap();
    let paths: Vec<_> = value
        .get("entries")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(|entry| entry.get("path").and_then(Value::as_str))
        .collect();
    assert_eq!(paths, vec!["keep.txt"]);
}

#[test]
fn windows_cloud_cleanup_delete_marker_prunes_expired_entries() {
    let config_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        config_dir.path().join(WINDOWS_CLOUD_CLEANUP_DELETE_FILE),
        r#"{"entries":[{"path":"old.txt","created_at_unix_ms":1}]}"#,
    )
    .unwrap();

    assert!(!consume_windows_cloud_cleanup_delete_marker(
        config_dir.path(),
        "old.txt",
    ));
    assert!(
        !config_dir
            .path()
            .join(WINDOWS_CLOUD_CLEANUP_DELETE_FILE)
            .exists()
    );
}
