#[test]
fn direct_root_state_request_timeout_keeps_followup_on_block_pull_path() {
    let config = AppConfig {
        blossom_servers: Vec::new(),
        ..AppConfig::default()
    };

    assert!(DIRECT_ROOT_STATE_REQUEST_SEND_TIMEOUT_SECS < event_block_pull_timeout_secs(&config));
    const { assert!(DIRECT_ROOT_STATE_REQUEST_SEND_TIMEOUT_SECS <= 1) };
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

#[tokio::test]
async fn provider_root_wake_listener_reads_fragmented_payload() {
    use tokio::io::AsyncWriteExt as _;

    let config_dir = tempfile::tempdir().unwrap();
    let (mut rx, task, _status) = start_provider_root_wake_listener(config_dir.path())
        .await
        .unwrap();
    let wake_path = iris_drive_core::paths::provider_root_wake_path_in(config_dir.path());
    let endpoint: Value = serde_json::from_slice(&std::fs::read(wake_path).unwrap()).unwrap();
    let port = endpoint["port"].as_u64().unwrap() as u16;
    let payload = json!({
        "root_cid": "root-fragmented",
        "file_count": 7,
        "top_level_entries": 2,
        "staged": true,
    });
    let bytes = serde_json::to_vec(&payload).unwrap();
    let split = bytes.len() / 2;
    let mut stream = tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    stream.write_all(&bytes[..split]).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    stream.write_all(&bytes[split..]).await.unwrap();
    stream.shutdown().await.unwrap();

    let received = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
        .await
        .unwrap()
        .unwrap();
    task.abort();

    assert_eq!(received, Some(payload));
}
