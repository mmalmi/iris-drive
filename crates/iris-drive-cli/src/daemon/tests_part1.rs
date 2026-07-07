use super::*;

struct DropNotify(Option<tokio::sync::oneshot::Sender<()>>);

impl Drop for DropNotify {
    fn drop(&mut self) {
        if let Some(tx) = self.0.take() {
            let _ = tx.send(());
        }
    }
}

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

#[tokio::test]
async fn grouped_keyed_daemon_tasks_replace_older_roots_but_coalesce_duplicates() {
    let tasks = DaemonTaskSet::default();
    let group = "root-apply:device-a".to_string();

    let (old_ready_tx, old_ready_rx) = tokio::sync::oneshot::channel();
    let (old_dropped_tx, old_dropped_rx) = tokio::sync::oneshot::channel();
    let old = tokio::spawn(async move {
        let _drop = DropNotify(Some(old_dropped_tx));
        let _ = old_ready_tx.send(());
        std::future::pending::<()>().await;
    });
    assert!(tasks.push_keyed_replacing_group(
        "root-apply:device-a:root-1".to_string(),
        group.clone(),
        old,
    ));
    old_ready_rx.await.expect("old task started");

    let (new_ready_tx, new_ready_rx) = tokio::sync::oneshot::channel();
    let (new_dropped_tx, new_dropped_rx) = tokio::sync::oneshot::channel();
    let new = tokio::spawn(async move {
        let _drop = DropNotify(Some(new_dropped_tx));
        let _ = new_ready_tx.send(());
        std::future::pending::<()>().await;
    });
    assert!(tasks.push_keyed_replacing_group(
        "root-apply:device-a:root-2".to_string(),
        group.clone(),
        new,
    ));
    tokio::time::timeout(std::time::Duration::from_secs(1), old_dropped_rx)
        .await
        .expect("old root task aborted by newer root")
        .expect("old drop notification");
    new_ready_rx.await.expect("new task started");

    assert!(!tasks.push_keyed_replacing_group(
        "root-apply:device-a:root-2".to_string(),
        group,
        tokio::spawn(async {
            panic!("duplicate exact root task should be coalesced");
        }),
    ));

    tasks.abort_all().await;
    tokio::time::timeout(std::time::Duration::from_secs(1), new_dropped_rx)
        .await
        .expect("new task aborted by cleanup")
        .expect("new drop notification");
}

#[test]
fn config_root_watch_accepts_exact_file_and_parent_directory_events() {
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
    let parent = notify::Event {
        kind: notify::EventKind::Modify(notify::event::ModifyKind::Metadata(
            notify::event::MetadataKind::Any,
        )),
        paths: vec![dir.path().to_path_buf()],
        attrs: notify::event::EventAttributes::new(),
    };

    assert!(event_touches_path(&event, &config));
    assert!(event_touches_path(&parent, &config));
    assert!(!event_touches_path(&other, &config));
    assert!(event_touches_config_root(&event, dir.path()));
    assert!(event_touches_config_root(&other, dir.path()));
    assert!(event_touches_config_root(&parent, dir.path()));
}

#[test]
fn daemon_status_records_binary_version_for_gui_mismatch_detection() {
    let dir = tempfile::tempdir().unwrap();

    let status = write_daemon_status(dir.path(), json!({"event": "test"}));

    assert_eq!(status["binary_version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn daemon_status_heartbeat_preserves_last_file_count() {
    let dir = tempfile::tempdir().unwrap();

    let seeded = write_daemon_status(
        dir.path(),
        json!({
            "event": "provider_root_published",
            "summary": {
                "file_count": 3,
                "visible_file_bytes": 123,
                "provider_refresh_key": "current:root-a",
            },
        }),
    );
    let heartbeat = write_daemon_status(dir.path(), json!({"event": "heartbeat"}));

    assert_eq!(seeded["summary"]["file_count"], 3);
    assert_eq!(heartbeat["summary"]["file_count"], 3);
    assert_eq!(heartbeat["summary"]["visible_file_bytes"], 123);
}

#[test]
fn daemon_status_prefers_fresh_hashtree_count_over_copied_summary() {
    let dir = tempfile::tempdir().unwrap();

    write_daemon_status(
        dir.path(),
        json!({
            "event": "old",
            "summary": {
                "file_count": 0,
                "visible_file_bytes": 0,
                "provider_refresh_key": "current:old-root",
            },
        }),
    );
    let refreshed = write_daemon_status(
        dir.path(),
        json!({
            "event": "provider_root_published",
            "hashtree": {
                "current_root_cid": "root-b",
                "file_count": 11,
                "visible_file_bytes": 456,
            },
        }),
    );

    assert_eq!(refreshed["summary"]["file_count"], 11);
    assert_eq!(refreshed["summary"]["visible_file_bytes"], 456);
    assert_eq!(refreshed["summary"]["provider_refresh_key"], "current:root-b");
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
fn provider_root_poll_is_safety_sweep_only_without_config_watch() {
    assert!(!provider_root_poll_enabled(true));
    assert!(provider_root_poll_enabled(false));
    assert_eq!(
        provider_root_poll_period(0),
        std::time::Duration::from_secs(30)
    );
    assert_eq!(
        provider_root_poll_period(60),
        std::time::Duration::from_mins(1)
    );
}

#[test]
fn provider_root_publish_cache_matches_fingerprint_and_root_key() {
    let fingerprint = ConfigFileFingerprint::for_test(10);
    let other_fingerprint = ConfigFileFingerprint::for_test(11);
    let publish_key = ProviderRootPublishKey {
        current_root_key: Some("primary:device:root-a".to_string()),
        profile_roster_key: Some("profile:op-a".to_string()),
    };
    let other_root_key = ProviderRootPublishKey {
        current_root_key: Some("primary:device:root-b".to_string()),
        profile_roster_key: publish_key.profile_roster_key.clone(),
    };
    let other_roster_key = ProviderRootPublishKey {
        current_root_key: publish_key.current_root_key.clone(),
        profile_roster_key: Some("profile:op-a,op-b".to_string()),
    };
    let mut cache = ProviderRootPublishCache::default();

    assert!(!cache.is_current(&fingerprint, &publish_key));
    cache.update(fingerprint.clone(), publish_key.clone());

    assert!(cache.is_current(&fingerprint, &publish_key));
    assert!(!cache.is_current(&other_fingerprint, &publish_key));
    assert!(!cache.is_current(&fingerprint, &other_root_key));
    assert!(!cache.is_current(&fingerprint, &other_roster_key));
    assert!(cache.publish_key_matches(&publish_key));
    assert!(!cache.publish_key_matches(&other_roster_key));
}

#[test]
fn provider_root_wake_payload_carries_file_count_status() {
    let status = provider_root_wake_status_payload(&json!({
        "root_cid": "root-a",
        "file_count": 12,
        "top_level_entries": 4,
    }))
    .expect("status payload");

    assert_eq!(status["event"], "provider_root_local_update");
    assert_eq!(status["root_cid"], "root-a");
    assert_eq!(status["hashtree"]["current_root_cid"], "root-a");
    assert_eq!(status["hashtree"]["file_count"], 12);
    assert_eq!(status["hashtree"]["top_level_entries"], 4);
}

#[test]
fn staged_provider_root_wake_payload_waits_for_import() {
    let status = provider_root_wake_status_payload(&json!({
        "root_cid": "root-a",
        "file_count": 4,
        "top_level_entries": 4,
        "staged": true,
    }));

    assert!(
        status.is_none(),
        "staged provider roots should not update GUI counts before daemon import"
    );
}

#[tokio::test]
async fn provider_root_wake_drain_keeps_latest_after_debounce() {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let first = json!({"root_cid": "root-a", "file_count": 1});
    let second = json!({"root_cid": "root-b", "file_count": 2});
    let third = json!({"root_cid": "root-c", "file_count": 3});
    tx.send(Some(second)).unwrap();
    let delayed = tx.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        delayed.send(Some(third)).unwrap();
    });

    let drained = drain_latest_provider_root_wake_payload_after_debounce(
        &mut rx,
        std::time::Duration::from_millis(150),
        Some(first),
    )
    .await;

    assert_eq!(drained, Some(json!({"root_cid": "root-c", "file_count": 3})));
}

#[test]
fn stale_root_apply_followup_detects_superseded_app_key_root() {
    let config_dir = tempfile::tempdir().unwrap();
    let mut drive = Drive {
        root_scope_id: iris_drive_core::NostrIdentityId::new_v4().to_string(),
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
            profile_id: iris_drive_core::NostrIdentityId::new_v4(),
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

#[test]
fn root_apply_followup_queue_key_groups_superseded_app_key_roots() {
    let mut drive = Drive {
        root_scope_id: iris_drive_core::NostrIdentityId::new_v4().to_string(),
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
            profile_id: iris_drive_core::NostrIdentityId::new_v4(),
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

    let queue_a = root_apply_followup_queue_key(&config, Some("remote-root-a"), true)
        .expect("initial queue key");
    let stale_a =
        root_apply_followup_key(&config, Some("remote-root-a"), true).expect("initial stale key");

    let remote_root = config
        .drives
        .get_mut(0)
        .unwrap()
        .app_key_roots
        .get_mut("device-b")
        .unwrap();
    remote_root.root_cid = "remote-root-b".to_string();
    remote_root.app_key_seq = 12;
    let queue_b = root_apply_followup_queue_key(&config, Some("remote-root-b"), true)
        .expect("updated queue key");
    let stale_b =
        root_apply_followup_key(&config, Some("remote-root-b"), true).expect("updated stale key");

    assert_eq!(queue_a, queue_b);
    assert_ne!(stale_a, stale_b);
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
fn config_mutation_lock_treats_permission_denied_as_contention() {
    let dir = tempfile::tempdir().unwrap();
    let lock_path = dir.path().join("config-mutation.lock");
    std::fs::write(&lock_path, "12345\n").unwrap();
    let permission_denied =
        std::io::Error::new(std::io::ErrorKind::PermissionDenied, "open denied");
    let missing_path = dir.path().join("missing.lock");

    assert!(ConfigMutationLock::lock_create_error_is_contention(
        &lock_path,
        &permission_denied,
    ));
    assert!(ConfigMutationLock::lock_create_error_is_contention(
        &missing_path,
        &permission_denied,
    ));
}

#[tokio::test]
async fn background_config_mutation_lock_retries_after_contention() {
    let dir = tempfile::tempdir().unwrap();
    let first = ConfigMutationLock::acquire(dir.path()).await.unwrap();
    let release = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(70)).await;
        drop(first);
    });
    let retry_delays = [
        std::time::Duration::from_millis(10),
        std::time::Duration::from_millis(10),
        std::time::Duration::from_millis(10),
        std::time::Duration::from_millis(10),
    ];

    let second = ConfigMutationLock::acquire_for_background_with_options(
        dir.path(),
        || false,
        std::time::Duration::from_millis(20),
        &retry_delays,
    )
    .await
    .unwrap();

    assert!(second.is_some());
    drop(second);
    release.await.unwrap();
}

#[tokio::test]
async fn background_config_mutation_lock_stops_when_followup_becomes_stale() {
    let dir = tempfile::tempdir().unwrap();
    let _first = ConfigMutationLock::acquire(dir.path()).await.unwrap();
    let stale = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stale_for_task = stale.clone();
    let mark_stale = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(35)).await;
        stale_for_task.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let retry_delays = [std::time::Duration::from_millis(10)];

    let second = ConfigMutationLock::acquire_for_background_with_options(
        dir.path(),
        || stale.load(std::sync::atomic::Ordering::SeqCst),
        std::time::Duration::from_millis(20),
        &retry_delays,
    )
    .await
    .unwrap();

    assert!(second.is_none());
    mark_stale.await.unwrap();
}

#[tokio::test]
async fn daemon_task_set_coalesces_active_keyed_tasks() {
    let tasks = DaemonTaskSet::default();
    let (started_tx, mut started_rx) = tokio::sync::mpsc::channel(2);
    let (release_tx, release_rx) = tokio::sync::oneshot::channel();

    assert!(tasks.push_keyed(
        "root:one".to_string(),
        tokio::spawn(async move {
            started_tx.send(()).await.unwrap();
            let _ = release_rx.await;
        }),
    ));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), started_rx.recv())
            .await
            .unwrap()
            .is_some()
    );
    assert!(!tasks.push_keyed(
        "root:one".to_string(),
        tokio::spawn(async {
            panic!("duplicate keyed task should be aborted");
        }),
    ));

    release_tx.send(()).unwrap();
    for _ in 0..20 {
        if tasks.push_keyed("root:one".to_string(), tokio::spawn(async {})) {
            tasks.abort_all().await;
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    tasks.abort_all().await;
    panic!("keyed task was not released after completion");
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
    let attempts = event_block_pull_retry_delays(&config).len() as u64;
    let retry_sleep: u64 = event_block_pull_retry_delays(&config).iter().sum();

    assert_eq!(event_block_pull_retry_delays(&config), &[0, 1]);
    assert!(
        event_block_pull_timeout_secs(&config)
            > FIPS_DOWNLOAD_BEFORE_BLOSSOM_ATTEMPT_TIMEOUT_SECS
                + BLOSSOM_DOWNLOAD_RETRY_DELAYS.iter().sum::<u64>()
    );
    assert!(attempts * event_block_pull_timeout_secs(&config) + retry_sleep <= 35);
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
fn windows_cloud_periodic_validation_rescans_recent_local_changes() {
    assert_eq!(
        windows_cloud_periodic_validate_change(),
        WindowsCloudRootChange::Rescan {
            full: false,
            recover_cached_deletes: false,
        }
    );
}

#[test]
fn windows_cloud_root_setting_supports_default_disable_and_override() {
    assert_eq!(
        windows_cloud_root_setting_from_env_value(None).unwrap(),
        WindowsCloudRootSetting::Default
    );
    assert_eq!(
        windows_cloud_root_setting_from_env_value(Some("  ")).unwrap(),
        WindowsCloudRootSetting::Default
    );
    assert_eq!(
        windows_cloud_root_setting_from_env_value(Some("off")).unwrap(),
        WindowsCloudRootSetting::Disabled
    );
    assert_eq!(
        windows_cloud_root_setting_from_env_value(Some("DISABLED")).unwrap(),
        WindowsCloudRootSetting::Disabled
    );
    assert_eq!(
        windows_cloud_root_setting_from_env_value(Some("C:\\\\IrisDriveE2E")).unwrap(),
        WindowsCloudRootSetting::Path(PathBuf::from("C:\\\\IrisDriveE2E"))
    );
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
