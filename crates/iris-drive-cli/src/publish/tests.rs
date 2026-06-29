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
        key: "drive-root:device:main:7:root-hash:root-key:device".to_string(),
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
        key: "drive-root:local:main:1:local-hash:local-key:local,remote".to_string(),
        event_id: "local-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"local\"}".to_string(),
    };
    let remote = DirectRootEvent {
        key: "drive-root:remote:main:7:remote-hash:remote-key:local,remote".to_string(),
        event_id: "remote-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"remote\"}".to_string(),
    };

    exchange.cache_event(remote.clone());
    let events = exchange.events_for_publish(vec![local.clone()]);

    assert_eq!(events.len(), 2);
    let local_publish = events
        .iter()
        .find(|publish| publish.event.event_id == local.event_id)
        .unwrap();
    let remote_publish = events
        .iter()
        .find(|publish| publish.event.event_id == remote.event_id)
        .unwrap();
    assert_eq!(local_publish.source, DirectRootPublishSource::LocalCurrent);
    assert_eq!(remote_publish.source, DirectRootPublishSource::CachedRelay);
}

#[test]
fn direct_root_heartbeat_publishes_local_root_events_only() {
    let mut exchange = DirectRootExchange::default();
    exchange.cache_event(DirectRootEvent {
        key: "drive-root:remote:main:7:remote-hash:remote-key:local,remote".to_string(),
        event_id: "remote-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"remote\"}".to_string(),
    });
    let drive = DirectRootEvent {
        key: "drive-root:local:main:8:drive-hash:drive-key:local,remote".to_string(),
        event_id: "drive-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"drive\"}".to_string(),
    };
    let share = DirectRootEvent {
        key: "share-root:share:local:4:share-hash:share-key:local,remote".to_string(),
        event_id: "share-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"share\"}".to_string(),
    };
    let files = DirectRootEvent {
        key: "files-root:local:main:drive-hash:drive-key".to_string(),
        event_id: "files-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_HASHTREE_ROOT,
        json: "{\"id\":\"files\"}".to_string(),
    };
    let profile = DirectRootEvent {
        key: "profile-op:profile:op".to_string(),
        event_id: "profile-event".to_string(),
        kind: iris_drive_core::KIND_NOSTR_IDENTITY_ROSTER_OP,
        json: "{\"id\":\"profile\"}".to_string(),
    };

    let events = exchange.local_root_events_for_publish(
        vec![drive.clone(), share.clone(), files, profile],
        DirectRootPublishSource::LocalHeartbeat,
    );

    assert_eq!(events.len(), 2);
    assert_eq!(events[0].event.event_id, drive.event_id);
    assert_eq!(events[0].source, DirectRootPublishSource::LocalHeartbeat);
    assert_eq!(events[1].event.event_id, share.event_id);
    assert_eq!(events[1].source, DirectRootPublishSource::LocalHeartbeat);
}

#[test]
fn direct_root_republish_keeps_latest_sequence_per_root_family() {
    let mut exchange = DirectRootExchange::default();
    let older = DirectRootEvent {
        key: "drive-root:remote:main:7:old-hash:old-key:local,remote".to_string(),
        event_id: "old-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"old\"}".to_string(),
    };
    let newer = DirectRootEvent {
        key: "drive-root:remote:main:8:new-hash:new-key:local,remote".to_string(),
        event_id: "new-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"new\"}".to_string(),
    };

    exchange.cache_event(older.clone());
    exchange.cache_event(newer.clone());
    exchange.cache_event(older);
    let events = exchange.events_for_publish(Vec::new());

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event.event_id, newer.event_id);
    assert_eq!(events[0].source, DirectRootPublishSource::CachedRelay);
}

#[test]
fn direct_root_republish_filters_cached_roots_superseded_by_local_root() {
    let mut exchange = DirectRootExchange::default();
    let older = DirectRootEvent {
        key: "drive-root:local:main:7:old-hash:old-key:local,remote".to_string(),
        event_id: "old-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"old\"}".to_string(),
    };
    let newer = DirectRootEvent {
        key: "drive-root:local:main:8:new-hash:new-key:local,remote".to_string(),
        event_id: "new-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"new\"}".to_string(),
    };

    exchange.cache_event(older);
    let events = exchange.events_for_publish(vec![newer.clone()]);

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event.event_id, newer.event_id);
    assert_eq!(events[0].source, DirectRootPublishSource::LocalCurrent);
}

#[test]
fn direct_root_peer_churn_does_not_clear_republish_throttle() {
    let mut exchange = DirectRootExchange::default();
    let key = "drive-root:device:main:7:root-hash:root-key:device,remote";
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
    assert!(!exchange.should_publish_key(key, now + std::time::Duration::from_millis(500)));
}

#[test]
fn direct_root_mesh_route_churn_does_not_clear_republish_throttle() {
    let mut exchange = DirectRootExchange::default();
    let key = "drive-root:device:main:7:root-hash:root-key:device,remote";
    let now = std::time::Instant::now();

    assert!(exchange.refresh_known_root_peers(
        ["authorized-a".to_string(), "authorized-b".to_string()],
        ["mesh-a".to_string()],
    ));
    assert!(exchange.should_publish_key(key, now));

    assert!(exchange.refresh_known_root_peers(
        ["authorized-a".to_string(), "authorized-b".to_string()],
        ["mesh-b".to_string()],
    ));
    assert!(!exchange.should_publish_key(key, now + std::time::Duration::from_millis(500)));
}

#[test]
fn direct_root_mesh_route_churn_does_not_clear_cached_relay_throttle() {
    let mut exchange = DirectRootExchange::default();
    let key = "drive-root:device:main:7:root-hash:root-key:device,remote";
    let now = std::time::Instant::now();

    assert!(exchange.refresh_known_root_peers(
        ["authorized-a".to_string(), "authorized-b".to_string()],
        ["mesh-a".to_string()],
    ));
    assert!(exchange.should_publish_candidate_key(key, DirectRootPublishSource::CachedRelay, now));

    assert!(exchange.refresh_known_root_peers(
        ["authorized-a".to_string(), "authorized-b".to_string()],
        ["mesh-b".to_string()],
    ));
    assert!(!exchange.should_publish_candidate_key(
        key,
        DirectRootPublishSource::CachedRelay,
        now + std::time::Duration::from_millis(500)
    ));
}

#[test]
fn direct_root_authorized_peer_loss_does_not_clear_republish_throttle() {
    let mut exchange = DirectRootExchange::default();
    let key = "drive-root:device:main:7:root-hash:root-key:device,remote";
    let now = std::time::Instant::now();

    assert!(exchange.refresh_known_root_peers(
        ["authorized-a".to_string(), "authorized-b".to_string()],
        ["mesh-a".to_string()],
    ));
    assert!(exchange.should_publish_key(key, now));

    assert!(
        exchange.refresh_known_root_peers(["authorized-a".to_string()], ["mesh-a".to_string()],)
    );
    assert!(!exchange.should_publish_key(key, now + std::time::Duration::from_millis(500)));
}

#[test]
fn direct_root_new_authorized_peer_clears_republish_throttle() {
    let mut exchange = DirectRootExchange::default();
    let key = "drive-root:device:main:7:root-hash:root-key:device,remote";
    let now = std::time::Instant::now();

    assert!(exchange.refresh_known_root_peers(
        ["authorized-a".to_string(), "authorized-b".to_string()],
        ["mesh-a".to_string()],
    ));
    assert!(exchange.should_publish_key(key, now));

    assert!(exchange.refresh_known_root_peers(
        [
            "authorized-a".to_string(),
            "authorized-b".to_string(),
            "authorized-c".to_string(),
        ],
        ["mesh-a".to_string()],
    ));
    assert!(exchange.should_publish_key(key, now + std::time::Duration::from_secs(1)));
}

#[test]
fn direct_root_republishes_after_short_native_cadence() {
    let mut exchange = DirectRootExchange::default();
    let key = "drive-root:device:main:8:root-hash:root-key:device,remote";
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
fn direct_root_cached_relay_roots_publish_newer_sequence_immediately() {
    let mut exchange = DirectRootExchange::default();
    let key = "drive-root:device:main:8:root-hash:root-key:device,remote";
    let newer_key = "drive-root:device:main:9:new-root-hash:new-root-key:device,remote";
    let now = std::time::Instant::now();

    assert!(exchange.should_publish_candidate_key(key, DirectRootPublishSource::CachedRelay, now));
    assert!(!exchange.should_publish_candidate_key(
        key,
        DirectRootPublishSource::CachedRelay,
        now + std::time::Duration::from_secs(DIRECT_ROOT_REPUBLISH_INTERVAL_SECS - 1)
    ));
    assert!(exchange.should_publish_candidate_key(
        newer_key,
        DirectRootPublishSource::CachedRelay,
        now + std::time::Duration::from_millis(1)
    ));
}

#[test]
fn direct_root_cache_event_preserves_same_key_republish_throttle() {
    let mut exchange = DirectRootExchange::default();
    let key = "drive-root:device:main:8:root-hash:root-key:device,remote";
    let now = std::time::Instant::now();
    let event = DirectRootEvent {
        key: key.to_string(),
        event_id: "event".to_string(),
        kind: 30_078,
        json: "{\"id\":\"event\"}".to_string(),
    };

    assert!(exchange.should_publish_key(key, now));
    exchange.cache_event(event.clone());

    assert!(!exchange.should_publish_key(key, now + std::time::Duration::from_millis(500)));
    exchange.cache_event(event);
    assert!(!exchange.should_publish_key(
        key,
        now + std::time::Duration::from_secs(DIRECT_ROOT_REPUBLISH_INTERVAL_SECS - 1)
    ));
}

#[test]
fn direct_root_peer_change_allows_cached_relay_once() {
    let mut exchange = DirectRootExchange::default();
    let key = "drive-root:device:main:8:root-hash:root-key:device,remote";
    let now = std::time::Instant::now();

    assert!(exchange.refresh_known_root_peers(
        ["authorized-a".to_string(), "authorized-b".to_string()],
        ["mesh-a".to_string()],
    ));
    assert!(exchange.should_publish_candidate_key(key, DirectRootPublishSource::CachedRelay, now));
    assert!(!exchange.should_publish_candidate_key(
        key,
        DirectRootPublishSource::CachedRelay,
        now + std::time::Duration::from_secs(DIRECT_ROOT_REPUBLISH_INTERVAL_SECS - 1)
    ));

    assert!(exchange.refresh_known_root_peers(
        [
            "authorized-a".to_string(),
            "authorized-b".to_string(),
            "authorized-c".to_string(),
        ],
        ["mesh-a".to_string()],
    ));
    assert!(exchange.should_publish_candidate_key(
        key,
        DirectRootPublishSource::CachedRelay,
        now + std::time::Duration::from_secs(1)
    ));
}

#[test]
fn direct_root_metadata_republishes_on_longer_cadence() {
    let mut exchange = DirectRootExchange::default();
    let key = "profile-op:profile:op";
    let now = std::time::Instant::now();

    assert!(exchange.should_publish_key(key, now));
    assert!(!exchange.should_publish_key(
        key,
        now + std::time::Duration::from_secs(DIRECT_ROOT_REPUBLISH_INTERVAL_SECS)
    ));
    assert!(exchange.should_publish_key(
        key,
        now + std::time::Duration::from_secs(DIRECT_ROOT_METADATA_REPUBLISH_INTERVAL_SECS)
    ));
}

#[test]
fn direct_root_cache_slot_parses_real_cid_shape() {
    assert_eq!(
        direct_root_cache_slot("drive-root:device:main:8:root-hash:root-key:device,remote"),
        Some(DirectRootCacheSlot {
            family: "drive-root:device:main".to_string(),
            seq: 8,
            recipient_count: 2,
        })
    );
    assert_eq!(
        direct_root_cache_slot("share-root:share:device:9:root-hash:root-key:device,remote"),
        Some(DirectRootCacheSlot {
            family: "share-root:share:device".to_string(),
            seq: 9,
            recipient_count: 2,
        })
    );
}

#[test]
fn direct_root_exchange_rejects_older_root_when_newer_is_cached() {
    let mut exchange = DirectRootExchange::default();
    exchange.cache_event(DirectRootEvent {
        key: "drive-root:remote:main:9:new-root:new-key:local,remote".to_string(),
        event_id: "new-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"new\"}".to_string(),
    });

    assert!(
        !exchange
            .should_cache_event_as_latest("drive-root:remote:main:8:old-root:old-key:local,remote")
    );
}

#[test]
fn direct_root_republish_collapses_recipient_list_variants() {
    let mut exchange = DirectRootExchange::default();
    let narrow = DirectRootEvent {
        key: "drive-root:remote:main:8:same-hash:same-key:local".to_string(),
        event_id: "narrow-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"narrow\"}".to_string(),
    };
    let wide = DirectRootEvent {
        key: "drive-root:remote:main:8:same-hash:same-key:local,remote".to_string(),
        event_id: "wide-event".to_string(),
        kind: iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        json: "{\"id\":\"wide\"}".to_string(),
    };

    exchange.cache_event(narrow);
    exchange.cache_event(wide.clone());
    let events = exchange.events_for_publish(Vec::new());

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event.event_id, wide.event_id);
}

#[test]
fn direct_root_republish_skips_cached_files_root_events() {
    let mut exchange = DirectRootExchange::default();
    let remote = DirectRootEvent {
        key: "files-root:remote:main:root-hash:root-key".to_string(),
        event_id: "remote-files-root".to_string(),
        kind: iris_drive_core::nostr_events::KIND_HASHTREE_ROOT,
        json: "{\"id\":\"remote\"}".to_string(),
    };

    exchange.cache_event(remote.clone());
    let events = exchange.events_for_publish(Vec::new());

    assert!(events.is_empty());
    assert!(exchange.seen_keys.contains(&remote.key));
}

#[test]
fn direct_root_seen_drive_root_retries_until_blocks_sync() {
    let config_dir = tempfile::tempdir().unwrap();
    let mut exchange = DirectRootExchange::default();
    let key = "drive-root:remote:main:8:root-hash:root-key:local,remote".to_string();

    exchange.seen_keys.insert(key.clone());

    assert!(!exchange.should_skip_seen_direct_root_frame(config_dir.path(), &key));

    write_daemon_status(
        config_dir.path(),
        json!({
            "event": "test",
            "block_sync_by_root": {
                "root-hash:root-key": {
                    "transport": "fips",
                    "total_hashes": 3,
                    "fetched": 3,
                }
            }
        }),
    );

    assert!(exchange.should_skip_seen_direct_root_frame(config_dir.path(), &key));
}

#[test]
fn direct_root_retry_policy_keeps_prerequisite_skips_uncached() {
    use iris_drive_core::relay_sync::DriveRootApply;

    for outcome in [
        DriveRootApply::NotOurScope,
        DriveRootApply::UnknownDrive,
        DriveRootApply::UnauthorizedAppKey,
        DriveRootApply::KeyUnavailable,
    ] {
        assert!(drive_root_apply_outcome_is_retryable(&outcome));
    }
    for outcome in [DriveRootApply::StaleTimestamp, DriveRootApply::Applied] {
        assert!(!drive_root_apply_outcome_is_retryable(&outcome));
    }
    assert!(!EventApplyOutcome::RetryablePrerequisiteMissing.should_cache_direct_root_frame());
    assert!(EventApplyOutcome::Changed.should_cache_direct_root_frame());
    assert!(EventApplyOutcome::Unchanged.should_cache_direct_root_frame());
    assert!(EventApplyOutcome::Changed.should_announce_current_state());
    assert!(!EventApplyOutcome::Unchanged.should_announce_current_state());
    assert!(!EventApplyOutcome::RetryablePrerequisiteMissing.should_announce_current_state());
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

#[test]
fn direct_root_publish_cache_reuses_unchanged_local_events() {
    let mut exchange = DirectRootExchange::default();
    let initial_fingerprint = ConfigFileFingerprint {
        len: 10,
        modified: None,
    };
    let changed_fingerprint = ConfigFileFingerprint {
        len: 11,
        modified: None,
    };
    let first = DirectRootEvent {
        key: "drive-root:first".to_string(),
        event_id: "event-a".to_string(),
        kind: 30_078,
        json: "{\"id\":\"event-a\"}".to_string(),
    };
    let second = DirectRootEvent {
        key: "drive-root:second".to_string(),
        event_id: "event-b".to_string(),
        kind: 30_078,
        json: "{\"id\":\"event-b\"}".to_string(),
    };
    let mut builds = 0;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let initial = runtime
        .block_on(exchange.cached_current_sync_events_from_config(
            initial_fingerprint.clone(),
            || async {
                builds += 1;
                Ok(vec![first.clone()])
            },
        ))
        .unwrap();
    let cached = runtime
        .block_on(
            exchange.cached_current_sync_events_from_config(initial_fingerprint, || async {
                builds += 1;
                Ok(vec![second.clone()])
            }),
        )
        .unwrap();
    let changed = runtime
        .block_on(
            exchange.cached_current_sync_events_from_config(changed_fingerprint, || async {
                builds += 1;
                Ok(vec![second.clone()])
            }),
        )
        .unwrap();

    assert_eq!(initial[0].key, first.key);
    assert_eq!(cached[0].key, first.key);
    assert_eq!(changed[0].key, second.key);
    assert_eq!(builds, 2);
}

#[test]
fn direct_root_publish_cache_can_be_invalidated_for_provider_updates() {
    let mut exchange = DirectRootExchange::default();
    let fingerprint = ConfigFileFingerprint {
        len: 10,
        modified: None,
    };
    let first = DirectRootEvent {
        key: "drive-root:first".to_string(),
        event_id: "event-a".to_string(),
        kind: 30_078,
        json: "{\"id\":\"event-a\"}".to_string(),
    };
    let second = DirectRootEvent {
        key: "drive-root:second".to_string(),
        event_id: "event-b".to_string(),
        kind: 30_078,
        json: "{\"id\":\"event-b\"}".to_string(),
    };
    let mut builds = 0;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let initial = runtime
        .block_on(
            exchange.cached_current_sync_events_from_config(fingerprint.clone(), || async {
                builds += 1;
                Ok(vec![first.clone()])
            }),
        )
        .unwrap();
    exchange.invalidate_current_sync_events_cache();
    let refreshed = runtime
        .block_on(
            exchange.cached_current_sync_events_from_config(fingerprint, || async {
                builds += 1;
                Ok(vec![second.clone()])
            }),
        )
        .unwrap();

    assert_eq!(initial[0].key, first.key);
    assert_eq!(refreshed[0].key, second.key);
    assert_eq!(builds, 2);
}

const _: () = {
    assert!(DIRECT_ROOT_PERIODIC_ANNOUNCE_SECS <= DIRECT_ROOT_REPUBLISH_INTERVAL_SECS);
    assert!(DIRECT_ROOT_PERIODIC_ANNOUNCE_SECS <= 10);
};

#[test]
fn direct_root_publish_bursts_root_frames_only() {
    let drive_root = "drive-root:device:main:8:root-hash:root-key:device,remote";
    let share_root = "share-root:share:device:9:root-hash:root-key:device,remote";
    assert_eq!(
        direct_root_publish_attempts(drive_root),
        4,
        "local drive roots need a short redundant burst"
    );
    assert_eq!(
        direct_root_publish_attempts(share_root),
        4,
        "local share roots need the same delivery burst"
    );
    assert_eq!(
        direct_root_publish_attempts_for_source(drive_root, DirectRootPublishSource::CachedRelay),
        2,
        "relayed drive roots should not be single-shot"
    );
    assert!(should_publish_direct_root_hint(
        drive_root,
        DirectRootPublishSource::LocalCurrent
    ));
    assert!(should_publish_direct_root_hint(
        share_root,
        DirectRootPublishSource::LocalCurrent
    ));
    assert!(!should_publish_direct_root_hint(
        drive_root,
        DirectRootPublishSource::CachedRelay
    ));
    assert!(!should_publish_direct_root_hint(
        "profile-op:profile:op",
        DirectRootPublishSource::LocalCurrent
    ));
    assert_eq!(direct_root_publish_attempts("files-root:device:main"), 2);
    assert_eq!(direct_root_publish_attempts("profile-op:profile:op"), 1);
}

#[test]
fn direct_root_heartbeat_uses_single_hinted_attempt_with_local_throttle() {
    let mut exchange = DirectRootExchange::default();
    let key = "drive-root:device:main:8:root-hash:root-key:device,remote";
    let now = std::time::Instant::now();

    assert_eq!(
        direct_root_publish_attempts_for_source(key, DirectRootPublishSource::LocalHeartbeat),
        1
    );
    assert!(should_publish_direct_root_hint(
        key,
        DirectRootPublishSource::LocalHeartbeat
    ));
    assert!(exchange.should_publish_candidate_key(key, DirectRootPublishSource::LocalCurrent, now));
    assert!(!exchange.should_publish_candidate_key(
        key,
        DirectRootPublishSource::LocalHeartbeat,
        now + std::time::Duration::from_millis(500)
    ));
    assert!(exchange.should_publish_candidate_key(
        key,
        DirectRootPublishSource::LocalHeartbeat,
        now + std::time::Duration::from_secs(DIRECT_ROOT_REPUBLISH_INTERVAL_SECS)
    ));
}

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
        event.kind == iris_drive_core::KIND_NOSTR_IDENTITY_ROSTER_OP
            && event.key.starts_with("profile-op:")
    }));
}

#[test]
fn direct_root_publish_prioritizes_roots_before_profile_metadata() {
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
    std::fs::write(work.path().join("local.txt"), b"local root").unwrap();
    let mut daemon = Daemon::open(config_dir.path()).unwrap();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime
        .block_on(daemon.import_source_dir(work.path()))
        .unwrap();
    let config = AppConfig {
        profile: Some(account.state.clone()),
        drives: daemon.config().drives.clone(),
        ..AppConfig::default()
    };

    let events = runtime
        .block_on(build_current_sync_events(
            config_dir.path(),
            &config,
            &account.state,
        ))
        .unwrap();

    let root_index = events
        .iter()
        .position(|event| event.key.starts_with("drive-root:"))
        .expect("drive root should be announced");
    let profile_index = events
        .iter()
        .position(|event| event.key.starts_with("profile-op:"))
        .expect("profile metadata should be announced");
    assert!(
        root_index < profile_index,
        "drive roots should hit the direct-root fast lane before profile metadata"
    );
}

#[test]
fn direct_root_publish_skips_private_root_without_key_recipients() {
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
    std::fs::write(work.path().join("local.txt"), b"local root").unwrap();
    let mut daemon = Daemon::open(config_dir.path()).unwrap();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(daemon.import_source_dir(work.path()))
        .unwrap();

    let mut awaiting_state = account.state.clone();
    awaiting_state.profile_roster_ops.clear();
    awaiting_state.profile_roster_projection = None;
    awaiting_state.app_keys = None;
    awaiting_state.authorization_state =
        iris_drive_core::AppKeyAuthorizationState::AwaitingApproval;
    let config = AppConfig {
        profile: Some(awaiting_state.clone()),
        drives: daemon.config().drives.clone(),
        ..AppConfig::default()
    };

    let events = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(build_current_sync_events(
            config_dir.path(),
            &config,
            &awaiting_state,
        ))
        .unwrap();

    assert!(
        !events
            .iter()
            .any(|event| event.kind == iris_drive_core::nostr_events::KIND_DRIVE_ROOT)
    );
    assert!(
        !events
            .iter()
            .any(|event| event.kind == iris_drive_core::nostr_events::KIND_HASHTREE_ROOT)
    );
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
