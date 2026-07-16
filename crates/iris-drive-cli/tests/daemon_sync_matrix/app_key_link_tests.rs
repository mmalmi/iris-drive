#[allow(clippy::wildcard_imports)]
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn running_daemon_subscribes_when_join_request_is_created_after_startup() {
    let _guard = live_daemon_test_guard().await;
    let relay = LocalNostrRelay::spawn().await;
    let owner_cfg = tempdir().unwrap();
    let linked_cfg = tempdir().unwrap();

    let _owner = run_json(owner_cfg.path(), &["init", "--label", "admin"]);
    add_config_relay(owner_cfg.path(), &relay.url);
    let owner_log = owner_cfg.path().join("owner.log");
    let owner_daemon = DaemonChild::spawn(
        owner_cfg.path(),
        &relay.url,
        owner_log,
        unused_loopback_port(),
    );

    let mut linked = iris_drive_core::Profile::start_join_request(
        linked_cfg.path(),
        Some("already-running-mac".to_string()),
    )
    .unwrap();
    let mut linked_config = iris_drive_core::AppConfig {
        profile: Some(linked.state.clone()),
        ..iris_drive_core::AppConfig::default()
    };
    linked_config
        .save(iris_drive_core::paths::config_path_in(linked_cfg.path()))
        .unwrap();

    let linked_log = linked_cfg.path().join("linked.log");
    let linked_daemon = DaemonChild::spawn(
        linked_cfg.path(),
        &relay.url,
        linked_log,
        unused_loopback_port(),
    );
    let startup_deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < startup_deadline && !linked_daemon.log().contains("subscribed") {
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    assert!(
        linked_daemon.log().contains("subscribed"),
        "linked daemon did not start:\n{}",
        linked_daemon.log()
    );

    let approval = iris_drive_core::app_key_link_transport::create_app_key_approval_bootstrap(
        linked.app_key.keys(),
        linked.state.app_key_label.as_deref(),
    )
    .unwrap();
    linked.state.queue_unbound_app_key_join_request(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
        approval.url.clone(),
        approval.request_keys.secret_key().to_secret_hex(),
    );
    linked_config.profile = Some(linked.state);
    linked_config
        .save(iris_drive_core::paths::config_path_in(linked_cfg.path()))
        .unwrap();

    let approval_result = run_json(
        owner_cfg.path(),
        &["app-keys", "approve", &approval.url, "--label", "Mac"],
    );
    assert_eq!(approval_result["approval_publish_error"], Value::Null);

    let authorization_deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < authorization_deadline {
        let status = run_json(linked_cfg.path(), &["status"]);
        let owner_receipt_acknowledged = iris_drive_core::AppConfig::load_or_default(
            iris_drive_core::paths::config_path_in(owner_cfg.path()),
        )
        .unwrap()
        .profile
        .is_some_and(|profile| profile.pending_device_approval_receipts.is_empty());
        if status["profile"]["authorization_state"] == "authorized"
            && status["profile"]["roster_size"] == 2
            && owner_receipt_acknowledged
        {
            return;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }

    panic!(
        "already-running daemon did not apply the approval and complete roster\nlinked status: {}\nlinked log:\n{}\nowner log:\n{}",
        serde_json::to_string_pretty(&run_json(linked_cfg.path(), &["status"])).unwrap(),
        linked_daemon.log(),
        owner_daemon.log(),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_bootstrap_webrtc_over_relay_and_deliver_link_request() {
    let _guard = live_daemon_test_guard().await;
    let relay = LocalNostrRelay::spawn().await;
    let blossom = LocalBlossomServer::spawn_with_upload_delay(Duration::ZERO).await;
    let owner_cfg = tempdir().unwrap();
    let linked_cfg = tempdir().unwrap();
    configure_local_blossom(owner_cfg.path(), &blossom.url);
    configure_local_blossom(linked_cfg.path(), &blossom.url);

    let owner = run_json(owner_cfg.path(), &["init", "--label", "admin"]);
    let owner_npub = owner["current_app_key_npub"].as_str().unwrap().to_string();
    let invite_url = owner["app_key_link_invite"]["url"].as_str().unwrap();
    let linked = run_json(
        linked_cfg.path(),
        &["link", invite_url, "--label", "iphone"],
    );
    let linked_npub = linked["current_app_key_npub"].as_str().unwrap().to_string();
    let owner_rendezvous_port = unused_udp_loopback_port();
    let linked_rendezvous_port = unused_udp_loopback_port();
    let owner_log = owner_cfg.path().join("owner.log");
    let linked_log = linked_cfg.path().join("linked.log");
    let owner_daemon = DaemonChild::spawn_webrtc_only(
        owner_cfg.path(),
        &relay.url,
        owner_log,
        unused_loopback_port(),
        owner_rendezvous_port,
        8,
    );
    let linked_daemon = DaemonChild::spawn_webrtc_only(
        linked_cfg.path(),
        &relay.url,
        linked_log,
        unused_loopback_port(),
        linked_rendezvous_port,
        8,
    );

    wait_until_webrtc_fips_connected(
        owner_cfg.path(),
        linked_cfg.path(),
        &linked_npub,
        &owner_npub,
        &owner_daemon,
        &linked_daemon,
    )
    .await;
    let started_at = Instant::now();
    let fast_window = Duration::from_secs(30);
    while started_at.elapsed() < fast_window {
        let status = run_json(owner_cfg.path(), &["status"]);
        if status["profile"]["inbound_app_key_link_requests"]
            .as_array()
            .is_some_and(|requests| !requests.is_empty())
        {
            return;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }

    panic!(
        "app-key link request did not reach admin within {:?}\nowner status: {}\nlinked status: {}\nowner log:\n{}\nlinked log:\n{}",
        started_at.elapsed(),
        serde_json::to_string_pretty(&run_json(owner_cfg.path(), &["status"])).unwrap(),
        serde_json::to_string_pretty(&run_json(linked_cfg.path(), &["status"])).unwrap(),
        owner_daemon.log(),
        linked_daemon.log(),
    );
}

async fn wait_until_webrtc_fips_connected(
    owner_cfg: &Path,
    linked_cfg: &Path,
    linked_npub: &str,
    owner_npub: &str,
    owner_daemon: &DaemonChild,
    linked_daemon: &DaemonChild,
) {
    let started_at = Instant::now();
    // One failed negotiation may consume the 30-second FIPS attempt deadline;
    // leave room for the bounded retry while still requiring a real WebRTC link.
    let window = Duration::from_mins(1);
    while started_at.elapsed() < window {
        let owner = run_json(owner_cfg, &["status"]);
        let linked = run_json(linked_cfg, &["status"]);
        if webrtc_fips_connected(&owner, linked_npub) && webrtc_fips_connected(&linked, owner_npub)
        {
            return;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    panic!(
        "relay-carried FIPS WebRTC did not connect within {:?}\nowner status: {}\nlinked status: {}\nowner log:\n{}\nlinked log:\n{}",
        started_at.elapsed(),
        serde_json::to_string_pretty(&run_json(owner_cfg, &["status"])).unwrap(),
        serde_json::to_string_pretty(&run_json(linked_cfg, &["status"])).unwrap(),
        owner_daemon.log(),
        linked_daemon.log(),
    );
}

fn webrtc_fips_connected(status: &Value, expected_peer: &str) -> bool {
    let fips = &status["network"]["fips"];
    fips["running"].as_bool().unwrap_or(false)
        && fips["fresh"].as_bool().unwrap_or(false)
        && fips["peer_statuses"].as_array().is_some_and(|peers| {
            peers.iter().any(|peer| {
                peer["npub"].as_str() == Some(expected_peer)
                    && peer["connected"].as_bool() == Some(true)
                    && peer["transport_type"].as_str() == Some("webrtc")
            })
        })
}
