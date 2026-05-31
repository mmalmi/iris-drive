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
fn unchanged_mount_visible_root_is_not_publishable() {
    let root = Cid::encrypted([0x11; 32], [0x22; 32]);
    let other = Cid::encrypted([0x33; 32], [0x44; 32]);

    assert!(!mount_visible_root_has_changed(&root, Some(&root)));
    assert!(mount_visible_root_has_changed(&root, Some(&other)));
    assert!(mount_visible_root_has_changed(&root, None));
}
