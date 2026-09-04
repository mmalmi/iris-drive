#[allow(clippy::wildcard_imports)]
use super::*;
use nostr_sdk::JsonUtil as _;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn running_owner_replays_durable_approval_ack_after_live_fanout_is_lost() {
    let _guard = live_daemon_test_guard().await;
    let relay = LocalNostrRelay::spawn().await;
    let owner_cfg = tempdir().unwrap();
    let linked_cfg = tempdir().unwrap();

    let owner = run_json(owner_cfg.path(), &["init", "--label", "admin"]);
    add_config_relay(owner_cfg.path(), &relay.url);
    let linked = run_json(
        linked_cfg.path(),
        &[
            "link",
            owner["app_key_link_invite"]["url"].as_str().unwrap(),
            "--label",
            "phone",
        ],
    );
    add_config_relay(linked_cfg.path(), &relay.url);
    let request_url = linked["app_key_link_request"]["url"]
        .as_str()
        .unwrap()
        .to_string();

    let owner_daemon = DaemonChild::spawn_relay_only(
        owner_cfg.path(),
        &relay.url,
        owner_cfg.path().join("owner.log"),
        unused_loopback_port(),
    );
    let startup_deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < startup_deadline && !owner_daemon.log().contains("subscribed") {
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    assert!(owner_daemon.log().contains("subscribed"));

    let approval = run_json(
        owner_cfg.path(),
        &["app-keys", "approve", &request_url, "--label", "Phone"],
    );
    assert_eq!(approval["roster_size"], 2);
    let subscription_deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < subscription_deadline
        && !owner_daemon.log().contains("app_key_link_roster_sent")
    {
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    assert!(
        owner_daemon.log().contains("app_key_link_roster_sent"),
        "owner did not activate its post-approval exchange:\n{}",
        owner_daemon.log()
    );

    // Model a lost notification on an otherwise durable relay. A new bounded
    // fetch can replay the ACK, but the daemon's existing live stream cannot.
    relay.suppress_live_kinds(&[iris_drive_core::KIND_NOSTR_IDENTITY_ROSTER_OP]);
    let linked_sync = run_json(
        linked_cfg.path(),
        &["sync", "--relay", &relay.url, "--timeout", "2"],
    );
    assert!(
        relay.events().await.iter().any(|event| {
            nostr_sdk::Event::from_json(event.to_string()).is_ok_and(|event| {
                iris_drive_core::relay_sync::is_device_approval_applied_ack_event(&event)
            })
        }),
        "linked sync did not durably publish an approval ACK: {linked_sync}"
    );

    let ack_deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < ack_deadline {
        let status = run_json(owner_cfg.path(), &["status"]);
        if status["profile"]["pending_device_approval_receipt_count"] == 0 {
            assert!(
                owner_daemon
                    .log()
                    .contains("\"event\":\"nostr_identity_device_approval_applied_ack_replay\""),
                "durable ACK was applied without daemon audit evidence:\n{}",
                owner_daemon.log()
            );
            return;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    panic!(
        "running owner did not replay the durable approval ACK after live fanout loss\nowner status: {}\nowner log:\n{}",
        serde_json::to_string_pretty(&run_json(owner_cfg.path(), &["status"])).unwrap(),
        owner_daemon.log(),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_only_running_daemon_receives_preexisting_root_after_unbound_approval() {
    let _guard = live_daemon_test_guard().await;
    let relay = LocalNostrRelay::spawn().await;
    let blossom = LocalBlossomServer::spawn_with_upload_delay(Duration::ZERO).await;
    let owner_cfg = tempdir().unwrap();
    let linked_cfg = tempdir().unwrap();

    let _owner = run_json(owner_cfg.path(), &["init", "--label", "admin"]);
    add_config_relay(owner_cfg.path(), &relay.url);
    configure_local_blossom(owner_cfg.path(), &blossom.url);
    let owner_log = owner_cfg.path().join("owner.log");
    let owner_daemon = DaemonChild::spawn_relay_only(
        owner_cfg.path(),
        &relay.url,
        owner_log,
        unused_loopback_port(),
    );
    let owner_startup_deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < owner_startup_deadline && !owner_daemon.log().contains("subscribed") {
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    assert!(
        owner_daemon.log().contains("subscribed"),
        "owner daemon did not start:\n{}",
        owner_daemon.log()
    );
    let source = owner_cfg.path().join("pre-link-source.txt");
    std::fs::write(&source, b"visible immediately after linking").unwrap();
    run_json(
        owner_cfg.path(),
        &[
            "provider",
            "write",
            "preexisting.txt",
            source.to_str().unwrap(),
        ],
    );
    let root_deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < root_deadline
        && !relay.events().await.iter().any(|event| {
            event["kind"].as_u64()
                == Some(u64::from(iris_drive_core::nostr_events::KIND_DRIVE_ROOT))
        })
    {
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    assert!(
        relay.events().await.iter().any(|event| {
            event["kind"].as_u64()
                == Some(u64::from(iris_drive_core::nostr_events::KIND_DRIVE_ROOT))
        }),
        "owner did not publish the pre-link drive root\n{}",
        owner_daemon.log()
    );

    let mut linked = iris_drive_core::Profile::start_join_request(
        linked_cfg.path(),
        Some("already-running-mac".to_string()),
    )
    .unwrap();
    let mut linked_config = iris_drive_core::AppConfig {
        profile: Some(linked.state.clone()),
        blossom_servers: vec![blossom.url.clone()],
        ..iris_drive_core::AppConfig::default()
    };
    linked_config
        .save(iris_drive_core::paths::config_path_in(linked_cfg.path()))
        .unwrap();

    let linked_log = linked_cfg.path().join("linked.log");
    let linked_daemon = DaemonChild::spawn_relay_only(
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

    let authorization_deadline = Instant::now() + Duration::from_secs(20);
    while Instant::now() < authorization_deadline {
        let status = run_json(linked_cfg.path(), &["status"]);
        let preexisting_visible = run_json_result(linked_cfg.path(), &["list"])
            .ok()
            .and_then(|list| list["files"].as_array().cloned())
            .is_some_and(|files| {
                files
                    .iter()
                    .any(|file| file["path"].as_str() == Some("preexisting.txt"))
            });
        if status["profile"]["authorization_state"] == "authorized"
            && status["profile"]["roster_size"] == 2
            && preexisting_visible
            && status["network"]["fips"]["roster_connected_peer_count"].as_u64() == Some(0)
        {
            return;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }

    panic!(
        "relay-only already-running daemon did not apply the approval and pre-link root\nlinked status: {}\nlinked list: {}\nlinked log:\n{}\nowner log:\n{}",
        serde_json::to_string_pretty(&run_json(linked_cfg.path(), &["status"])).unwrap(),
        serde_json::to_string_pretty(&run_json(linked_cfg.path(), &["list"])).unwrap(),
        linked_daemon.log(),
        owner_daemon.log(),
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_bootstrap_over_websocket_seed_and_deliver_link_request() {
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
    let websocket_seed_port = unused_loopback_port();
    let owner_log = owner_cfg.path().join("owner.log");
    let linked_log = linked_cfg.path().join("linked.log");
    let owner_daemon = DaemonChild::spawn_websocket_listener(
        owner_cfg.path(),
        &relay.url,
        owner_log,
        unused_loopback_port(),
        websocket_seed_port,
        8,
    );
    let owner_startup_deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < owner_startup_deadline && !owner_daemon.log().contains("subscribed") {
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    assert!(
        owner_daemon.log().contains("subscribed"),
        "WebSocket seed daemon did not start:\n{}",
        owner_daemon.log()
    );
    let linked_daemon = DaemonChild::spawn_websocket_client(
        linked_cfg.path(),
        &relay.url,
        linked_log,
        unused_loopback_port(),
        websocket_seed_port,
        8,
    );

    wait_until_websocket_fips_connected(
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn routed_websocket_devices_deliver_approval_ack_and_report_online_through_mesh_sessions() {
    let _guard = live_daemon_test_guard().await;
    let relay = LocalNostrRelay::spawn().await;
    let blossom = LocalBlossomServer::spawn_with_upload_delay(Duration::ZERO).await;
    let seed_cfg = tempdir().unwrap();
    let owner_cfg = tempdir().unwrap();
    let linked_cfg = tempdir().unwrap();
    for config_dir in [seed_cfg.path(), owner_cfg.path(), linked_cfg.path()] {
        configure_local_blossom(config_dir, &blossom.url);
    }

    let _seed = run_json(seed_cfg.path(), &["init", "--label", "transit"]);
    let owner = run_json(owner_cfg.path(), &["init", "--label", "admin"]);
    let owner_npub = owner["current_app_key_npub"].as_str().unwrap().to_string();
    let invite_url = owner["app_key_link_invite"]["url"].as_str().unwrap();
    let linked = run_json(
        linked_cfg.path(),
        &["link", invite_url, "--label", "iphone"],
    );
    let linked_npub = linked["current_app_key_npub"].as_str().unwrap().to_string();
    let request_url = linked["app_key_link_request"]["url"]
        .as_str()
        .unwrap()
        .to_string();

    let websocket_seed_port = unused_loopback_port();
    let seed_daemon = DaemonChild::spawn_websocket_listener(
        seed_cfg.path(),
        &relay.url,
        seed_cfg.path().join("seed.log"),
        unused_loopback_port(),
        websocket_seed_port,
        8,
    );
    let owner_daemon = DaemonChild::spawn_websocket_client(
        owner_cfg.path(),
        &relay.url,
        owner_cfg.path().join("owner.log"),
        unused_loopback_port(),
        websocket_seed_port,
        8,
    );
    let linked_daemon = DaemonChild::spawn_websocket_client(
        linked_cfg.path(),
        &relay.url,
        linked_cfg.path().join("linked.log"),
        unused_loopback_port(),
        websocket_seed_port,
        8,
    );

    let request_deadline = Instant::now() + Duration::from_secs(30);
    while Instant::now() < request_deadline {
        let owner_status = run_json(owner_cfg.path(), &["status"]);
        if owner_status["profile"]["inbound_app_key_link_requests"]
            .as_array()
            .is_some_and(|requests| !requests.is_empty())
        {
            break;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    let owner_status = run_json(owner_cfg.path(), &["status"]);
    assert!(
        owner_status["profile"]["inbound_app_key_link_requests"]
            .as_array()
            .is_some_and(|requests| !requests.is_empty()),
        "routed FIPS link request did not reach owner\nowner status: {}\nseed log:\n{}\nowner log:\n{}\nlinked log:\n{}",
        serde_json::to_string_pretty(&owner_status).unwrap(),
        seed_daemon.log(),
        owner_daemon.log(),
        linked_daemon.log(),
    );

    // Accept the approval events at the relay but do not fan them out. The
    // linked device must become authorized, and both roster entries must
    // become online, through the routed FIPS session itself.
    relay.drop_kinds(&[iris_drive_core::KIND_NOSTR_IDENTITY_ROSTER_OP]);
    let approval = run_json(
        owner_cfg.path(),
        &["app-keys", "approve", &request_url, "--label", "iPhone"],
    );
    assert_eq!(approval["roster_size"], 2);

    let online_deadline = Instant::now() + Duration::from_secs(8);
    while Instant::now() < online_deadline {
        let owner_status = run_json(owner_cfg.path(), &["status"]);
        let linked_status = run_json(linked_cfg.path(), &["status"]);
        if owner_status["profile"]["roster_size"] == 2
            && owner_status["profile"]["pending_device_approval_receipt_count"] == 0
            && linked_status["profile"]["authorization_state"] == "authorized"
            && linked_status["profile"]["roster_size"] == 2
            && mesh_fips_connected(&owner_status, &linked_npub)
            && mesh_fips_connected(&linked_status, &owner_npub)
        {
            return;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }

    panic!(
        "routed FIPS approval did not authorize the joiner, drain its exact receipt, and bring both devices online within 8s\nowner status: {}\nlinked status: {}\nseed log:\n{}\nowner log:\n{}\nlinked log:\n{}",
        serde_json::to_string_pretty(&run_json(owner_cfg.path(), &["status"])).unwrap(),
        serde_json::to_string_pretty(&run_json(linked_cfg.path(), &["status"])).unwrap(),
        seed_daemon.log(),
        owner_daemon.log(),
        linked_daemon.log(),
    );
}

async fn wait_until_websocket_fips_connected(
    owner_cfg: &Path,
    linked_cfg: &Path,
    linked_npub: &str,
    owner_npub: &str,
    owner_daemon: &DaemonChild,
    linked_daemon: &DaemonChild,
) {
    let started_at = Instant::now();
    let window = Duration::from_secs(30);
    while started_at.elapsed() < window {
        let owner = run_json(owner_cfg, &["status"]);
        let linked = run_json(linked_cfg, &["status"]);
        if websocket_fips_connected(&owner, linked_npub)
            && websocket_fips_connected(&linked, owner_npub)
        {
            return;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    panic!(
        "FIPS WebSocket first adjacency did not connect within {:?}\nowner status: {}\nlinked status: {}\nowner log:\n{}\nlinked log:\n{}",
        started_at.elapsed(),
        serde_json::to_string_pretty(&run_json(owner_cfg, &["status"])).unwrap(),
        serde_json::to_string_pretty(&run_json(linked_cfg, &["status"])).unwrap(),
        owner_daemon.log(),
        linked_daemon.log(),
    );
}

fn websocket_fips_connected(status: &Value, expected_peer: &str) -> bool {
    let fips = &status["network"]["fips"];
    fips["running"].as_bool().unwrap_or(false)
        && fips["fresh"].as_bool().unwrap_or(false)
        && fips["peer_statuses"].as_array().is_some_and(|peers| {
            peers.iter().any(|peer| {
                peer["npub"].as_str() == Some(expected_peer)
                    && peer["connected"].as_bool() == Some(true)
                    && peer["transport_type"].as_str() == Some("websocket")
            })
        })
}

fn mesh_fips_connected(status: &Value, expected_peer: &str) -> bool {
    let fips = &status["network"]["fips"];
    let mesh_online = fips["mesh_devices"].as_array().is_some_and(|peers| {
        peers
            .iter()
            .any(|peer| peer.as_str() == Some(expected_peer))
    });
    let incorrectly_direct = fips["direct_devices"].as_array().is_some_and(|peers| {
        peers
            .iter()
            .any(|peer| peer.as_str() == Some(expected_peer))
    });
    let roster_online = status["peers"].as_array().is_some_and(|peers| {
        peers.iter().any(|peer| {
            peer["app_key_npub"].as_str() == Some(expected_peer)
                && peer["fips_online"].as_bool() == Some(true)
                && peer["fips_online_via"].as_str() == Some("mesh")
                && peer["connection_label"].as_str() == Some("Online (Mesh)")
        })
    });
    fips["running"].as_bool().unwrap_or(false)
        && fips["fresh"].as_bool().unwrap_or(false)
        && mesh_online
        && !incorrectly_direct
        && roster_online
}
