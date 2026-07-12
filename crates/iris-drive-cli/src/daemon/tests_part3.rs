#[test]
fn direct_root_state_request_timeout_keeps_followup_on_block_pull_path() {
    let config = AppConfig {
        blossom_servers: Vec::new(),
        ..AppConfig::default()
    };

    assert!(DIRECT_ROOT_STATE_REQUEST_SEND_TIMEOUT_SECS < event_block_pull_timeout_secs(&config));
    assert!(DIRECT_ROOT_STATE_REQUEST_SEND_TIMEOUT_SECS <= 1);
}

#[test]
fn direct_root_recovery_state_request_bypasses_short_throttle() {
    let root_scope_id = format!(
        "test-{}",
        iris_drive_core::NostrIdentityId::new_v4()
    );

    assert!(should_publish_direct_root_state_request(
        &root_scope_id,
        ["peer-a"],
        false,
    ));
    assert!(!should_publish_direct_root_state_request(
        &root_scope_id,
        ["peer-a"],
        false,
    ));
    assert!(should_publish_direct_root_state_request(
        &root_scope_id,
        ["peer-b"],
        false,
    ));
    assert!(should_publish_direct_root_state_request(
        &root_scope_id,
        ["peer-a"],
        true,
    ));
    assert!(!should_publish_direct_root_state_request(
        &root_scope_id,
        ["peer-a"],
        false,
    ));
}
