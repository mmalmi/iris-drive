#[tokio::test]
async fn unbound_manual_join_backfill_adopts_approval_roster_candidate() {
    use nostr_sdk::JsonUtil;

    let owner_dir = tempfile::tempdir().unwrap();
    let mut owner = Profile::create(owner_dir.path(), Some("iOS owner".to_string())).unwrap();
    let linked_dir = tempfile::tempdir().unwrap();
    let mut linked =
        Profile::start_join_request(linked_dir.path(), Some("Mac waiting".to_string())).unwrap();
    let linked_pubkey = linked.state.app_key_pubkey.clone();
    linked
        .state
        .queue_unbound_app_key_join_request(123, String::new());
    let placeholder_profile_id = linked.state.profile_id;
    let mut linked_config = AppConfig {
        profile: Some(linked.state.clone()),
        ..AppConfig::default()
    };
    linked_config.upsert_drive(Drive::primary(placeholder_profile_id.to_string()));
    linked_config
        .save(config_path_in(linked_dir.path()))
        .unwrap();

    owner
        .approve_app_key(&linked_pubkey, Some("Mac waiting".to_string()))
        .unwrap();
    let relay_events = owner
        .state
        .profile_roster_ops
        .iter()
        .map(|op| nostr_sdk::Event::from_json(&op.event_json).unwrap())
        .collect::<Vec<_>>();
    let candidates =
        iris_drive_core::relay_sync::nostr_identity_app_key_approval_candidates_from_events(
            &linked_pubkey,
            &relay_events,
        )
        .unwrap();

    let tasks = DaemonTaskSet::default();
    let outcome = apply_pending_app_key_link_roster_candidates(
        linked_dir.path(),
        candidates,
        None,
        None,
        &tasks,
    )
    .await
    .unwrap();

    assert_eq!(outcome, Some(EventApplyOutcome::Changed));
    let saved = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    let state = saved.profile.as_ref().unwrap();
    assert_eq!(state.profile_id, owner.state.profile_id);
    assert_eq!(
        state.authorization_state,
        iris_drive_core::AppKeyAuthorizationState::Authorized
    );
    assert!(state.outbound_app_key_link_request.is_none());
    assert_eq!(
        saved.drive(iris_drive_core::PRIMARY_DRIVE_ID).unwrap().root_scope_id,
        owner.state.profile_id.to_string()
    );
    tasks.abort_all().await;
}

#[tokio::test]
async fn waiting_join_backfill_adopts_approval_candidate_without_cached_request() {
    use nostr_sdk::JsonUtil;

    let owner_dir = tempfile::tempdir().unwrap();
    let mut owner = Profile::create(owner_dir.path(), Some("iOS owner".to_string())).unwrap();
    let linked_dir = tempfile::tempdir().unwrap();
    let linked =
        Profile::start_join_request(linked_dir.path(), Some("Mac waiting".to_string())).unwrap();
    let linked_pubkey = linked.state.app_key_pubkey.clone();
    assert!(linked.state.outbound_app_key_link_request.is_none());
    let placeholder_profile_id = linked.state.profile_id;
    let mut linked_config = AppConfig {
        profile: Some(linked.state.clone()),
        ..AppConfig::default()
    };
    linked_config.upsert_drive(Drive::primary(placeholder_profile_id.to_string()));
    linked_config
        .save(config_path_in(linked_dir.path()))
        .unwrap();

    owner
        .approve_app_key(&linked_pubkey, Some("Mac waiting".to_string()))
        .unwrap();
    let relay_events = owner
        .state
        .profile_roster_ops
        .iter()
        .map(|op| nostr_sdk::Event::from_json(&op.event_json).unwrap())
        .collect::<Vec<_>>();
    let candidates =
        iris_drive_core::relay_sync::nostr_identity_app_key_approval_candidates_from_events(
            &linked_pubkey,
            &relay_events,
        )
        .unwrap();

    let tasks = DaemonTaskSet::default();
    let outcome = apply_pending_app_key_link_roster_candidates(
        linked_dir.path(),
        candidates,
        None,
        None,
        &tasks,
    )
    .await
    .unwrap();

    assert_eq!(outcome, Some(EventApplyOutcome::Changed));
    let saved = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    let state = saved.profile.as_ref().unwrap();
    assert_eq!(state.profile_id, owner.state.profile_id);
    assert_eq!(
        state.authorization_state,
        iris_drive_core::AppKeyAuthorizationState::Authorized
    );
    assert!(state.outbound_app_key_link_request.is_none());
    tasks.abort_all().await;
}

#[tokio::test]
async fn windows_cloud_rescan_upserts_nested_local_file() {
    let (_blocks, provider) = fresh_test_provider().await;
    let sync_root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(sync_root.path().join("codex-lab").join("run")).unwrap();
    std::fs::write(
        sync_root
            .path()
            .join("codex-lab")
            .join("run")
            .join("live.txt"),
        b"live",
    )
    .unwrap();

    for path in windows_cloud_local_projected_paths(sync_root.path()).unwrap() {
        apply_windows_cloud_upsert(&provider, sync_root.path(), &path, &BTreeSet::new())
            .await
            .unwrap();
    }

    let item = provider.item(&"codex-lab/run/live.txt".to_string()).await;
    assert!(item.is_ok());
}

#[test]
fn windows_cloud_local_state_loads_pascal_case_cache() {
    let config_dir = tempfile::tempdir().unwrap();
    std::fs::write(
        config_dir.path().join(WINDOWS_CLOUD_LOCAL_STATE_FILE),
        r#"{"entries":[{"Path":"remote.txt","Kind":"file","Size":4,"Sha256":"abcd","ProviderVersion":"remote-v1"},{"Path":".Trash-1000/nope","Kind":"file","Size":1,"Sha256":"eeee"}]}"#,
    )
    .unwrap();

    let state = load_windows_cloud_local_state(config_dir.path());

    assert_eq!(
        state,
        vec![WindowsCloudLocalStateEntry {
            path: "remote.txt".to_string(),
            kind: "file".to_string(),
            size: 4,
            sha256: Some("abcd".to_string()),
            provider_version: Some("remote-v1".to_string()),
        }]
    );
}

#[test]
fn windows_cloud_local_state_keeps_old_provider_version_for_unreplaced_placeholder() {
    let previous = WindowsCloudLocalStateEntry {
        path: "remote.txt".to_string(),
        kind: "file".to_string(),
        size: 4,
        sha256: None,
        provider_version: Some("remote-v1".to_string()),
    };
    let current = WindowsCloudExpectedEntry {
        path: "remote.txt".to_string(),
        kind: "file",
        size: 4,
        version: "remote-v2".to_string(),
    };

    let provider_version =
        windows_cloud_snapshot_provider_version(&current, Some(&previous), true, None);

    assert_eq!(provider_version.as_deref(), Some("remote-v1"));
}

#[test]
fn windows_cloud_stale_cleanup_removes_unchanged_synced_file() {
    let sync_root = tempfile::tempdir().unwrap();
    let path = sync_root.path().join("remote.txt");
    std::fs::write(&path, b"same").unwrap();
    let state = vec![WindowsCloudLocalStateEntry {
        path: "remote.txt".to_string(),
        kind: "file".to_string(),
        size: 4,
        sha256: Some(to_hex(&hashtree_core::sha256(b"same"))),
        provider_version: None,
    }];

    let removed = windows_cloud_remove_stale_synced_local_items(
        sync_root.path(),
        &BTreeSet::new(),
        &state,
        &BTreeSet::new(),
    );

    assert_eq!(removed, vec!["remote.txt".to_string()]);
    assert!(!path.exists());
}

#[test]
fn windows_cloud_stale_cleanup_preserves_local_edit() {
    let sync_root = tempfile::tempdir().unwrap();
    let path = sync_root.path().join("remote.txt");
    std::fs::write(&path, b"edited").unwrap();
    let state = vec![WindowsCloudLocalStateEntry {
        path: "remote.txt".to_string(),
        kind: "file".to_string(),
        size: 4,
        sha256: Some(to_hex(&hashtree_core::sha256(b"same"))),
        provider_version: None,
    }];

    let removed = windows_cloud_remove_stale_synced_local_items(
        sync_root.path(),
        &BTreeSet::new(),
        &state,
        &BTreeSet::new(),
    );

    assert!(removed.is_empty());
    assert!(path.exists());
}

#[test]
fn windows_cloud_stale_cleanup_preserves_protected_local_mutation() {
    let sync_root = tempfile::tempdir().unwrap();
    let dir = sync_root.path().join("smoke");
    let file = dir.join("from-windows.txt");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(&file, b"same").unwrap();
    let state = vec![
        WindowsCloudLocalStateEntry {
            path: "smoke".to_string(),
            kind: "directory".to_string(),
            size: 0,
            sha256: None,
            provider_version: None,
        },
        WindowsCloudLocalStateEntry {
            path: "smoke/from-windows.txt".to_string(),
            kind: "file".to_string(),
            size: 4,
            sha256: Some(to_hex(&hashtree_core::sha256(b"same"))),
            provider_version: None,
        },
    ];
    let protected = BTreeSet::from(["smoke".to_string()]);

    let removed = windows_cloud_remove_stale_synced_local_items(
        sync_root.path(),
        &BTreeSet::new(),
        &state,
        &protected,
    );

    assert!(removed.is_empty());
    assert!(dir.exists());
    assert!(file.exists());
}

#[test]
fn windows_cloud_local_state_records_imported_local_mutation_after_apply() {
    let sync_root = tempfile::tempdir().unwrap();
    std::fs::create_dir(sync_root.path().join("smoke")).unwrap();
    std::fs::write(
        sync_root.path().join("smoke").join("from-windows.txt"),
        b"same",
    )
    .unwrap();
    let entries = vec![
        WindowsCloudExpectedEntry {
            path: "smoke".to_string(),
            kind: "directory",
            size: 0,
            version: "dir-v1".to_string(),
        },
        WindowsCloudExpectedEntry {
            path: "smoke/from-windows.txt".to_string(),
            kind: "file",
            size: 4,
            version: "file-v1".to_string(),
        },
    ];

    let state =
        snapshot_windows_cloud_local_state(sync_root.path(), &entries, &[], &BTreeSet::new());

    assert!(state.contains(&WindowsCloudLocalStateEntry {
        path: "smoke/from-windows.txt".to_string(),
        kind: "file".to_string(),
        size: 4,
        sha256: Some(to_hex(&hashtree_core::sha256(b"same"))),
        provider_version: Some("file-v1".to_string()),
    }));

    let config_dir = tempfile::tempdir().unwrap();
    write_windows_cloud_local_state(
        config_dir.path(),
        sync_root.path(),
        &entries,
        &[],
        &BTreeSet::new(),
    );
    let raw = std::fs::read_to_string(config_dir.path().join(WINDOWS_CLOUD_LOCAL_STATE_FILE))
        .unwrap();
    let value: Value = serde_json::from_str(&raw).unwrap();
    let provider_version = value
        .get("entries")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .find(|entry| {
            entry.get("path").and_then(Value::as_str) == Some("smoke/from-windows.txt")
        })
        .and_then(|entry| entry.get("providerVersion"))
        .and_then(Value::as_str);
    assert_eq!(provider_version, Some("file-v1"));
}

#[test]
fn windows_cloud_local_state_omits_protected_local_mutation() {
    let sync_root = tempfile::tempdir().unwrap();
    std::fs::create_dir(sync_root.path().join("smoke")).unwrap();
    std::fs::write(
        sync_root.path().join("smoke").join("from-windows.txt"),
        b"same",
    )
    .unwrap();
    let entries = vec![
        WindowsCloudExpectedEntry {
            path: "smoke".to_string(),
            kind: "directory",
            size: 0,
            version: "dir-v1".to_string(),
        },
        WindowsCloudExpectedEntry {
            path: "smoke/from-windows.txt".to_string(),
            kind: "file",
            size: 4,
            version: "file-v1".to_string(),
        },
    ];
    let protected = BTreeSet::from(["smoke/from-windows.txt".to_string()]);

    let state = snapshot_windows_cloud_local_state(sync_root.path(), &entries, &[], &protected);

    assert!(state.is_empty());
}

#[test]
fn windows_cloud_local_state_does_not_retain_protected_previous_state() {
    let sync_root = tempfile::tempdir().unwrap();
    std::fs::create_dir(sync_root.path().join("smoke")).unwrap();
    std::fs::write(
        sync_root.path().join("smoke").join("from-windows.txt"),
        b"same",
    )
    .unwrap();
    let previous = vec![
        WindowsCloudLocalStateEntry {
            path: "smoke".to_string(),
            kind: "directory".to_string(),
            size: 0,
            sha256: None,
            provider_version: None,
        },
        WindowsCloudLocalStateEntry {
            path: "smoke/from-windows.txt".to_string(),
            kind: "file".to_string(),
            size: 4,
            sha256: Some(to_hex(&hashtree_core::sha256(b"same"))),
            provider_version: None,
        },
    ];
    let protected = BTreeSet::from(["smoke/from-windows.txt".to_string()]);

    let state =
        snapshot_windows_cloud_local_state(sync_root.path(), &[], &previous, &protected);

    assert!(state.is_empty());
}

#[test]
fn windows_cloud_state_retains_unremoved_stale_synced_file_for_retry() {
    let sync_root = tempfile::tempdir().unwrap();
    std::fs::write(sync_root.path().join("remote.txt"), b"same").unwrap();
    let previous = vec![WindowsCloudLocalStateEntry {
        path: "remote.txt".to_string(),
        kind: "file".to_string(),
        size: 4,
        sha256: Some(to_hex(&hashtree_core::sha256(b"same"))),
        provider_version: None,
    }];

    let retained = windows_cloud_retained_stale_local_state(
        sync_root.path(),
        &BTreeSet::new(),
        &previous,
        &BTreeSet::new(),
    );

    assert_eq!(retained, previous);
}

#[test]
fn windows_cloud_state_drops_stale_file_after_local_edit() {
    let sync_root = tempfile::tempdir().unwrap();
    std::fs::write(sync_root.path().join("remote.txt"), b"edited").unwrap();
    let previous = vec![WindowsCloudLocalStateEntry {
        path: "remote.txt".to_string(),
        kind: "file".to_string(),
        size: 4,
        sha256: Some(to_hex(&hashtree_core::sha256(b"same"))),
        provider_version: None,
    }];

    let retained = windows_cloud_retained_stale_local_state(
        sync_root.path(),
        &BTreeSet::new(),
        &previous,
        &BTreeSet::new(),
    );

    assert!(retained.is_empty());
}

#[tokio::test]
async fn windows_cloud_projection_removes_changed_synced_file() {
    let config_dir = tempfile::tempdir().unwrap();
    let sync_root = tempfile::tempdir().unwrap();
    let (_blocks, provider) = fresh_test_provider().await;
    write_provider_file(&provider, "remote.txt", b"new bytes")
        .await
        .unwrap();
    std::fs::write(sync_root.path().join("remote.txt"), b"old bytes").unwrap();
    let previous = vec![WindowsCloudLocalStateEntry {
        path: "remote.txt".to_string(),
        kind: "file".to_string(),
        size: 9,
        sha256: Some(to_hex(&hashtree_core::sha256(b"old bytes"))),
        provider_version: None,
    }];
    let entries = vec![WindowsCloudExpectedEntry {
        path: "remote.txt".to_string(),
        kind: "file",
        size: 9,
        version: "remote-v2".to_string(),
    }];

    let removed = windows_cloud_remove_changed_synced_local_files(
        config_dir.path(),
        sync_root.path(),
        &provider,
        &entries,
        &previous,
        &BTreeSet::new(),
    )
    .await
    .unwrap();

    assert_eq!(removed, vec!["remote.txt".to_string()]);
    assert!(!sync_root.path().join("remote.txt").exists());
    assert!(consume_windows_cloud_cleanup_delete_marker(
        config_dir.path(),
        "remote.txt",
    ));
}

#[tokio::test]
async fn windows_cloud_projection_preserves_local_edit_over_remote_change() {
    let config_dir = tempfile::tempdir().unwrap();
    let sync_root = tempfile::tempdir().unwrap();
    let (_blocks, provider) = fresh_test_provider().await;
    write_provider_file(&provider, "remote.txt", b"new bytes")
        .await
        .unwrap();
    std::fs::write(sync_root.path().join("remote.txt"), b"local edit").unwrap();
    let previous = vec![WindowsCloudLocalStateEntry {
        path: "remote.txt".to_string(),
        kind: "file".to_string(),
        size: 9,
        sha256: Some(to_hex(&hashtree_core::sha256(b"old bytes"))),
        provider_version: None,
    }];
    let entries = vec![WindowsCloudExpectedEntry {
        path: "remote.txt".to_string(),
        kind: "file",
        size: 9,
        version: "remote-v2".to_string(),
    }];

    let removed = windows_cloud_remove_changed_synced_local_files(
        config_dir.path(),
        sync_root.path(),
        &provider,
        &entries,
        &previous,
        &BTreeSet::new(),
    )
    .await
    .unwrap();

    assert!(removed.is_empty());
    assert_eq!(
        std::fs::read(sync_root.path().join("remote.txt")).unwrap(),
        b"local edit"
    );
}

#[tokio::test]
async fn windows_cloud_upsert_prunes_ignored_local_tree_from_provider() {
    let (_blocks, provider) = fresh_test_provider().await;
    write_provider_file(&provider, ".Trash-1000/files/removed.txt", b"trash")
        .await
        .unwrap();
    write_provider_file(&provider, "keep.txt", b"keep")
        .await
        .unwrap();

    let sync_root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(sync_root.path().join(".Trash-1000").join("files")).unwrap();
    std::fs::write(
        sync_root
            .path()
            .join(".Trash-1000")
            .join("files")
            .join("removed.txt"),
        b"trash",
    )
    .unwrap();

    assert!(
        apply_windows_cloud_upsert(
            &provider,
            sync_root.path(),
            ".Trash-1000",
            &BTreeSet::new(),
        )
        .await
        .unwrap()
    );
    let trash = ".Trash-1000".to_string();
    let keep = "keep.txt".to_string();
    assert!(provider.item(&trash).await.is_err());
    assert!(provider.item(&keep).await.is_ok());
}

#[tokio::test]
async fn windows_cloud_provider_prune_removes_ignored_merged_paths() {
    let (_blocks, provider) = fresh_test_provider().await;
    write_provider_file(&provider, "noise/.DS_Store", b"finder")
        .await
        .unwrap();
    write_provider_file(&provider, "$RECYCLE.BIN/S-1-5-21/removed.txt", b"recycle")
        .await
        .unwrap();
    write_provider_file(&provider, "keep.txt", b"keep")
        .await
        .unwrap();

    let pruned = prune_ignored_provider_paths(&provider).await.unwrap();

    assert_eq!(
        pruned,
        vec!["$RECYCLE.BIN".to_string(), "noise/.DS_Store".to_string()]
    );
    let recycle = "$RECYCLE.BIN".to_string();
    let noise = "noise".to_string();
    let keep = "keep.txt".to_string();
    assert!(provider.item(&recycle).await.is_err());
    assert!(provider.item(&noise).await.is_ok());
    assert!(provider.item(&keep).await.is_ok());
}

#[tokio::test]
async fn windows_cloud_event_delete_skips_when_local_directory_exists() {
    let (_blocks, provider) = fresh_test_provider().await;
    write_provider_file(&provider, "codex-lab/run/live.txt", b"live")
        .await
        .unwrap();
    let before = provider.current_root().await;

    let sync_root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(sync_root.path().join("codex-lab").join("run")).unwrap();

    assert!(
        !apply_windows_cloud_delete_if_local_missing(
            &provider,
            sync_root.path(),
            "codex-lab/run",
        )
        .await
        .unwrap()
    );
    assert_eq!(provider.current_root().await, before);
    assert!(
        provider
            .item(&"codex-lab/run/live.txt".to_string())
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn windows_cloud_event_delete_applies_when_local_path_is_missing() {
    let (_blocks, provider) = fresh_test_provider().await;
    write_provider_file(&provider, "codex-lab/run/gone.txt", b"gone")
        .await
        .unwrap();

    let sync_root = tempfile::tempdir().unwrap();

    assert!(
        apply_windows_cloud_delete_if_local_missing(
            &provider,
            sync_root.path(),
            "codex-lab/run",
        )
        .await
        .unwrap()
    );
    assert!(
        provider
            .item(&"codex-lab/run/gone.txt".to_string())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn root_apply_followup_skips_refresh_when_blocks_are_missing() {
    let config_dir = tempfile::tempdir().unwrap();
    let mut config = AppConfig::default();
    config.blossom_servers.clear();
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);

    let task = spawn_root_apply_followup(
        config_dir.path().to_path_buf(),
        config,
        Some("not-a-cid".to_string()),
        None,
        true,
        "test_refresh",
        Some(tx),
    )
    .expect("followup should be spawned");

    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(200), rx.recv())
            .await
            .is_err()
    );
    task.abort();
    let _ = task.await;
}

#[test]
fn stale_drive_root_followup_refreshes_projection_after_blocks_sync() {
    assert_eq!(
        drive_root_followup_plan(false, true, false),
        DriveRootFollowupPlan {
            pull_blocks: true,
            refresh_projection: true,
        }
    );
    assert_eq!(
        drive_root_followup_plan(false, true, true),
        DriveRootFollowupPlan {
            pull_blocks: true,
            refresh_projection: true,
        }
    );
    assert_eq!(
        drive_root_followup_plan(true, false, false),
        DriveRootFollowupPlan {
            pull_blocks: true,
            refresh_projection: true,
        }
    );
}

#[test]
fn startup_root_sync_collects_unsynced_remote_roots() {
    let config_dir = tempfile::tempdir().unwrap();
    let synced = AppKeyRootRef::legacy("already-synced", 10, 1);
    let needs_sync = AppKeyRootRef::legacy("needs-sync", 20, 1);
    let duplicate = AppKeyRootRef::legacy("needs-sync", 21, 1);
    let mut projected = AppKeyRootRef::legacy("local-only", 30, 1);
    projected.local_only = true;
    let mut drive = Drive {
        root_scope_id: iris_drive_core::NostrIdentityId::new_v4().to_string(),
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        display_name: "My Drive".to_string(),
        role: DriveRole::Owner,
        app_key_roots: BTreeMap::new(),
        last_root_cid: None,
        key_hex: None,
    };
    drive.app_key_roots.insert("device-a".to_string(), synced);
    drive
        .app_key_roots
        .insert("device-b".to_string(), needs_sync);
    drive.app_key_roots.insert("device-c".to_string(), duplicate);
    drive
        .app_key_roots
        .insert("device-d".to_string(), projected);
    let mut config = AppConfig::default();
    config.drives.push(drive);
    record_block_sync(
        config_dir.path(),
        "already-synced",
        "fips",
        &DownloadReport::default(),
    );

    let roots = startup_root_cids_needing_sync(config_dir.path(), &config);

    assert_eq!(roots, vec!["needs-sync".to_string()]);
}

#[test]
fn startup_root_sync_retries_failed_remote_roots() {
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
        AppKeyRootRef::legacy("failed-root", 10, 1),
    );
    let mut config = AppConfig::default();
    config.drives.push(drive);
    record_block_sync_error(config_dir.path(), "failed-root", "timed out after 15s");

    let roots = startup_root_cids_needing_sync(config_dir.path(), &config);

    assert_eq!(roots, vec!["failed-root".to_string()]);
}

#[tokio::test]
async fn windows_cloud_upsert_skips_matching_existing_file() {
    let (_blocks, provider) = fresh_test_provider().await;
    write_provider_file(&provider, "remote.txt", b"same")
        .await
        .unwrap();
    let before = provider.current_root().await;

    let sync_root = tempfile::tempdir().unwrap();
    std::fs::write(sync_root.path().join("remote.txt"), b"same").unwrap();

    assert!(
        !apply_windows_cloud_upsert(
            &provider,
            sync_root.path(),
            "remote.txt",
            &BTreeSet::new(),
        )
        .await
        .unwrap()
    );
    assert_eq!(provider.current_root().await, before);
}

#[tokio::test]
async fn windows_cloud_upsert_skips_existing_directory() {
    let (_blocks, provider) = fresh_test_provider().await;
    create_provider_dir(&provider, "existing").await.unwrap();
    let before = provider.current_root().await;

    let sync_root = tempfile::tempdir().unwrap();
    std::fs::create_dir(sync_root.path().join("existing")).unwrap();

    assert!(
        !apply_windows_cloud_upsert(&provider, sync_root.path(), "existing", &BTreeSet::new(),)
            .await
            .unwrap()
    );
    assert_eq!(provider.current_root().await, before);
}

#[tokio::test]
async fn windows_cloud_upsert_skips_stale_cached_placeholder() {
    let (_blocks, provider) = fresh_test_provider().await;
    let before = provider.current_root().await;

    let sync_root = tempfile::tempdir().unwrap();
    std::fs::write(sync_root.path().join("remote-deleted.txt"), b"stale").unwrap();
    let placeholder_paths = BTreeSet::from(["remote-deleted.txt".to_string()]);

    assert!(
        !apply_windows_cloud_upsert(
            &provider,
            sync_root.path(),
            "remote-deleted.txt",
            &placeholder_paths,
        )
        .await
        .unwrap()
    );
    assert_eq!(provider.current_root().await, before);
    assert!(
        provider
            .item(&"remote-deleted.txt".to_string())
            .await
            .is_err()
    );
}
