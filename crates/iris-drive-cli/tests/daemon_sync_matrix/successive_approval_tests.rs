use super::app_key_link_tests::wait_until_websocket_fips_connected;
#[allow(clippy::wildcard_imports)]
use super::*;

async fn persist_ui_approval(owner: &Path, request: &Value, label: &str) {
    let _lock = iris_drive_core::config_lock::ConfigMutationLock::acquire(owner)
        .await
        .unwrap();
    let path = iris_drive_core::paths::config_path_in(owner);
    let mut config = iris_drive_core::AppConfig::load_or_default(&path).unwrap();
    let mut profile =
        iris_drive_core::Profile::load(config.profile.take().unwrap(), owner).unwrap();
    let bootstrap = iris_drive_core::app_key_link_transport::parse_app_key_approval_bootstrap(
        request["app_key_link_request"]["url"].as_str().unwrap(),
    )
    .unwrap()
    .unwrap();
    profile
        .approve_device_bootstrap(&bootstrap, Some(label.into()))
        .unwrap();
    config.profile = Some(profile.state);
    config.save(path).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn new_approval_does_not_wait_for_an_earlier_stalled_roster_publication() {
    let _guard = live_daemon_test_guard().await;
    let relay = LocalNostrRelay::spawn().await;
    let owner_cfg = tempdir().unwrap();
    let first_cfg = tempdir().unwrap();
    let second_cfg = tempdir().unwrap();
    let owner = run_json(owner_cfg.path(), &["init", "--label", "admin"]);
    let invite = owner["app_key_link_invite"]["url"].as_str().unwrap();
    let first = run_json(first_cfg.path(), &["link", invite, "--label", "first"]);
    let second = run_json(second_cfg.path(), &["link", invite, "--label", "second"]);
    for config in [&owner_cfg, &first_cfg, &second_cfg] {
        add_config_relay(config.path(), &relay.url);
    }
    let seed = unused_loopback_port();
    let owner_daemon = DaemonChild::spawn_websocket_listener(
        owner_cfg.path(),
        &relay.url,
        owner_cfg.path().join("owner.log"),
        unused_loopback_port(),
        seed,
        8,
    );
    let first_daemon = DaemonChild::spawn_websocket_client(
        first_cfg.path(),
        &relay.url,
        first_cfg.path().join("first.log"),
        unused_loopback_port(),
        seed,
        8,
    );
    let second_daemon = DaemonChild::spawn_websocket_client(
        second_cfg.path(),
        &relay.url,
        second_cfg.path().join("second.log"),
        unused_loopback_port(),
        seed,
        8,
    );
    for (config, request, daemon) in [
        (&first_cfg, &first, &first_daemon),
        (&second_cfg, &second, &second_daemon),
    ] {
        wait_until_websocket_fips_connected(
            owner_cfg.path(),
            config.path(),
            request["current_app_key_npub"].as_str().unwrap(),
            owner["current_app_key_npub"].as_str().unwrap(),
            &owner_daemon,
            daemon,
        )
        .await;
    }
    relay.stall_kinds(&[iris_drive_core::KIND_NOSTR_IDENTITY_ROSTER_OP]);
    persist_ui_approval(owner_cfg.path(), &first, "First").await;
    let first_deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let first_status = run_json(first_cfg.path(), &["status"]);
        let owner_status = run_json(owner_cfg.path(), &["status"]);
        if first_status["profile"]["authorization_state"] == "authorized"
            && owner_status["profile"]["pending_device_approval_receipt_count"] == 0
            && relay.stalled_publications() > 0
        {
            break;
        }
        assert!(
            Instant::now() < first_deadline,
            "first approval did not complete\n{}",
            owner_daemon.log()
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    // The first direct handoff has completed while its relay publication is
    // still blocked. A new UI approval must have its own prompt direct turn.
    persist_ui_approval(owner_cfg.path(), &second, "Second").await;
    let second_started = Instant::now();
    let second_deadline = second_started + Duration::from_secs(3);
    loop {
        let second_status = run_json(second_cfg.path(), &["status"]);
        let owner_status = run_json(owner_cfg.path(), &["status"]);
        if second_status["profile"]["authorization_state"] == "authorized"
            && owner_status["profile"]["pending_device_approval_receipt_count"] == 0
        {
            break;
        }
        assert!(
            Instant::now() < second_deadline,
            "earlier stalled relay held a newer direct approval\nowner:\n{}\nsecond:\n{}",
            owner_daemon.log(),
            second_daemon.log(),
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    matrix_progress(format!(
        "second UI approval while earlier relay remains blocked: {:?}",
        second_started.elapsed()
    ));
}
