#[allow(clippy::wildcard_imports)]
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_app_key_link_request_reaches_admin_quickly() {
    let _guard = live_daemon_test_guard().await;
    let relay = LocalNostrRelay::spawn().await;
    let blossom = LocalBlossomServer::spawn_with_upload_delay(Duration::ZERO).await;
    let owner_cfg = tempdir().unwrap();
    let linked_cfg = tempdir().unwrap();
    configure_local_blossom(owner_cfg.path(), &blossom.url);
    configure_local_blossom(linked_cfg.path(), &blossom.url);

    let owner = run_json(owner_cfg.path(), &["init", "--label", "admin"]);
    let invite_url = owner["app_key_link_invite"]["url"].as_str().unwrap();
    let _linked = run_json(
        linked_cfg.path(),
        &["link", invite_url, "--label", "iphone"],
    );
    let owner_log = owner_cfg.path().join("owner.log");
    let linked_log = linked_cfg.path().join("linked.log");
    let owner_daemon = DaemonChild::spawn(
        owner_cfg.path(),
        &relay.url,
        owner_log,
        unused_loopback_port(),
    );
    let linked_daemon = DaemonChild::spawn(
        linked_cfg.path(),
        &relay.url,
        linked_log,
        unused_loopback_port(),
    );

    let started_at = Instant::now();
    let fast_window = Duration::from_secs(6);
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
