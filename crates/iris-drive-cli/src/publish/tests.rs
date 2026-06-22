use super::*;

#[test]
fn direct_root_mesh_publish_sequence_is_monotonic() {
    let mut exchange = DirectRootExchange::default();

    let first = exchange.next_mesh_publish_seq();
    let second = exchange.next_mesh_publish_seq();
    let third = exchange.next_mesh_publish_seq();

    assert!(first > 0);
    assert_eq!(second, first + 1);
    assert_eq!(third, second + 1);
}

#[test]
fn direct_root_mesh_reuses_cached_event_for_same_logical_root() {
    let mut exchange = DirectRootExchange::default();
    let first = DirectRootEvent {
        key: "drive-root:device:main:7:root".to_string(),
        event_id: "first-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"first\"}".to_string(),
    };
    let rebuilt = DirectRootEvent {
        key: first.key.clone(),
        event_id: "rebuilt-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"rebuilt\"}".to_string(),
    };

    exchange.cache_event(first.clone());
    let event = exchange.event_for_publish(rebuilt);

    assert_eq!(event.event_id, first.event_id);
    assert_eq!(event.json, first.json);
}

#[test]
fn direct_root_republish_includes_cached_remote_events() {
    let mut exchange = DirectRootExchange::default();
    let local = DirectRootEvent {
        key: "drive-root:local:main:1:local-root".to_string(),
        event_id: "local-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"local\"}".to_string(),
    };
    let remote = DirectRootEvent {
        key: "drive-root:remote:main:7:remote-root".to_string(),
        event_id: "remote-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"remote\"}".to_string(),
    };

    exchange.cache_event(remote.clone());
    let events = exchange.events_for_publish(vec![local.clone()]);

    assert_eq!(events.len(), 2);
    assert!(events.iter().any(|event| event.event_id == local.event_id));
    assert!(events.iter().any(|event| event.event_id == remote.event_id));
}

#[test]
fn direct_root_peer_churn_does_not_clear_republish_throttle() {
    let mut exchange = DirectRootExchange::default();
    let key = "drive-root:device:main:7:root";
    let now = std::time::Instant::now();

    assert!(exchange.refresh_known_root_peers(
        ["authorized-a".to_string(), "authorized-b".to_string()],
        ["mesh-a".to_string()],
    ));
    assert!(exchange.should_publish_key(key, now));

    assert!(!exchange.refresh_known_root_peers(
        ["authorized-a".to_string(), "authorized-b".to_string()],
        ["mesh-a".to_string()],
    ));
    assert!(!exchange.should_publish_key(key, now + std::time::Duration::from_secs(1)));
}

#[test]
fn direct_root_republishes_after_short_native_cadence() {
    let mut exchange = DirectRootExchange::default();
    let key = "drive-root:device:main:8:root";
    let now = std::time::Instant::now();

    assert!(exchange.should_publish_key(key, now));
    assert!(!exchange.should_publish_key(
        key,
        now + std::time::Duration::from_secs(DIRECT_ROOT_REPUBLISH_INTERVAL_SECS - 1)
    ));
    assert!(exchange.should_publish_key(
        key,
        now + std::time::Duration::from_secs(DIRECT_ROOT_REPUBLISH_INTERVAL_SECS)
    ));
}

#[test]
fn direct_root_profile_stream_cache_reuses_unchanged_config() {
    let config_dir = tempfile::tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("native".to_string())).unwrap();
    let config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    let mut exchange = DirectRootExchange::default();
    let mut loads = 0;
    let initial_fingerprint = ConfigFileFingerprint {
        len: 10,
        modified: None,
    };

    let first = exchange
        .cached_profile_stream_root_scope_id_from_config(initial_fingerprint.clone(), || {
            loads += 1;
            Ok(config.clone())
        })
        .unwrap();
    let second = exchange
        .cached_profile_stream_root_scope_id_from_config(initial_fingerprint, || {
            loads += 1;
            Ok(AppConfig::default())
        })
        .unwrap();
    let changed = exchange
        .cached_profile_stream_root_scope_id_from_config(
            ConfigFileFingerprint {
                len: 11,
                modified: None,
            },
            || {
                loads += 1;
                Ok(AppConfig::default())
            },
        )
        .unwrap();

    assert_eq!(first, Some(account.state.root_scope_id()));
    assert_eq!(second, first);
    assert_eq!(changed, None);
    assert_eq!(loads, 2);
}

const _: () = {
    assert!(DIRECT_ROOT_PERIODIC_ANNOUNCE_SECS >= 30);
    assert!(DIRECT_ROOT_PERIODIC_ANNOUNCE_SECS >= DIRECT_ROOT_REPUBLISH_INTERVAL_SECS * 6);
};

#[test]
fn direct_root_publish_includes_profile_roster_ops() {
    let config_dir = tempfile::tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("native".to_string())).unwrap();
    let config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let events = runtime
        .block_on(build_current_sync_events(
            config_dir.path(),
            &config,
            &account.state,
        ))
        .unwrap();

    assert!(events.iter().any(|event| {
        event.kind == iris_drive_core::KIND_IRIS_PROFILE_ROSTER_OP
            && event.key.starts_with("profile-op:")
    }));
}

#[test]
fn direct_root_publish_includes_share_access_snapshot_and_roots() {
    let config_dir = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("native".to_string())).unwrap();
    let mut initial_config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    initial_config.upsert_drive(Drive::primary(account.state.root_scope_id()));
    initial_config
        .save(config_path_in(config_dir.path()))
        .unwrap();
    std::fs::write(work.path().join("alpha.txt"), b"share root").unwrap();
    let mut daemon = Daemon::open(config_dir.path()).unwrap();
    let report = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(daemon.import_source_dir(work.path()))
        .unwrap();
    let root = daemon
        .config()
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.state.app_key_pubkey)
        .unwrap()
        .clone();
    assert_eq!(root.root_cid, report.root_cid);
    let mut folder = iris_drive_core::create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("native".to_string()),
        Vec::new(),
        10,
    )
    .unwrap();
    folder
        .app_key_roots
        .insert(account.state.app_key_pubkey.clone(), root);
    let config = AppConfig {
        profile: Some(account.state.clone()),
        shared_folders: vec![folder.clone()],
        ..AppConfig::default()
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let events = runtime
        .block_on(build_current_sync_events(
            config_dir.path(),
            &config,
            &account.state,
        ))
        .unwrap();

    let share_access = events
        .iter()
        .find(|event| {
            event.kind == iris_drive_core::KIND_SHARE_ACCESS_SNAPSHOT
                && event
                    .key
                    .starts_with(&format!("share-access:{}:", folder.share_id))
        })
        .expect("share access snapshot should be announced");
    let event = nostr_sdk::Event::from_json(&share_access.json).unwrap();
    let snapshot = iris_drive_core::parse_share_access_snapshot_event(&event).unwrap();
    assert_eq!(snapshot.content, folder.access);
    let share_root = events
        .iter()
        .find(|event| {
            event
                .key
                .starts_with(&format!("share-root:{}:", folder.share_id))
        })
        .expect("share root event should be announced");
    let event = nostr_sdk::Event::from_json(&share_root.json).unwrap();
    let (_, root_scope, drive_id) =
        iris_drive_core::nostr_events::parse_drive_root_event_header(&event).unwrap();
    assert_eq!(root_scope, folder.share_id.to_string());
    assert_eq!(drive_id, iris_drive_core::PRIMARY_DRIVE_ID);
}

#[test]
fn unchanged_mount_visible_root_is_not_publishable() {
    let root = Cid::encrypted([0x11; 32], [0x22; 32]);
    let other = Cid::encrypted([0x33; 32], [0x44; 32]);

    assert!(!mount_visible_root_has_changed(&root, Some(&root)));
    assert!(mount_visible_root_has_changed(&root, Some(&other)));
    assert!(mount_visible_root_has_changed(&root, None));
}
