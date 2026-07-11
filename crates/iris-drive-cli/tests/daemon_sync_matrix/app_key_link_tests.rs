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
    let owner_npub = owner["current_app_key_npub"].as_str().unwrap().to_string();
    let invite_url = owner["app_key_link_invite"]["url"].as_str().unwrap();
    let linked = run_json(
        linked_cfg.path(),
        &["link", invite_url, "--label", "iphone"],
    );
    let linked_npub = linked["current_app_key_npub"].as_str().unwrap().to_string();
    let owner_fips_port = unused_udp_loopback_port();
    let linked_fips_port = unused_udp_loopback_port();
    let owner_log = owner_cfg.path().join("owner.log");
    let linked_log = linked_cfg.path().join("linked.log");
    let owner_daemon = DaemonChild::spawn_with_fips_peers(
        owner_cfg.path(),
        &relay.url,
        owner_log,
        unused_loopback_port(),
        owner_fips_port,
        &format!("{linked_npub}=127.0.0.1:{linked_fips_port}"),
    );
    let linked_daemon = DaemonChild::spawn_with_fips_peers(
        linked_cfg.path(),
        &relay.url,
        linked_log,
        unused_loopback_port(),
        linked_fips_port,
        &format!("{owner_npub}=127.0.0.1:{owner_fips_port}"),
    );

    wait_until_open_fips_connected(owner_cfg.path(), linked_cfg.path()).await;
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

async fn wait_until_open_fips_connected(owner_cfg: &Path, linked_cfg: &Path) {
    let started_at = Instant::now();
    let window = Duration::from_secs(10);
    while started_at.elapsed() < window {
        let owner = run_json(owner_cfg, &["status"]);
        let linked = run_json(linked_cfg, &["status"]);
        if open_fips_connected(&owner) && open_fips_connected(&linked) {
            return;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    panic!(
        "open FIPS discovery did not connect within {:?}\nowner status: {}\nlinked status: {}",
        started_at.elapsed(),
        serde_json::to_string_pretty(&run_json(owner_cfg, &["status"])).unwrap(),
        serde_json::to_string_pretty(&run_json(linked_cfg, &["status"])).unwrap(),
    );
}

fn open_fips_connected(status: &Value) -> bool {
    let fips = &status["network"]["fips"];
    fips["running"].as_bool().unwrap_or(false)
        && fips["fresh"].as_bool().unwrap_or(false)
        && fips["connected_peer_count"].as_u64().unwrap_or(0) >= 1
}
