#[allow(clippy::wildcard_imports)]
use super::*;
use hashtree_core::{DirEntry, Store};
use hashtree_fs::FsBlobStore;
use iris_drive_core::{AppConfig, daemon::Daemon, paths};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_staged_provider_import_recovers_when_missing_block_arrives_without_wake() {
    let _guard = live_daemon_test_guard().await;
    let relay = LocalNostrRelay::spawn().await;
    let config_dir = tempdir().unwrap();
    run_json(config_dir.path(), &["init", "--label", "owner"]);
    let config_path = paths::config_path_in(config_dir.path());
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    config.relays = vec![relay.url.clone()];
    config.blossom_servers.clear();
    config.local_nhash_resolver_enabled = false;
    config.save(&config_path).unwrap();
    let empty = tempdir().unwrap();
    run_json(
        config_dir.path(),
        &["import", empty.path().to_str().unwrap()],
    );

    let bytes = b"staged local write survives a missing block";
    let daemon = Daemon::open(config_dir.path()).unwrap();
    let base_root = daemon.tree().put_directory(Vec::new()).await.unwrap();
    let (file, size) = daemon.tree().put_file(bytes).await.unwrap();
    let root = daemon
        .tree()
        .put_directory(vec![
            DirEntry::from_cid("local.txt", &file)
                .with_size(size)
                .with_link_type(LinkType::File),
        ])
        .await
        .unwrap();
    let store = FsBlobStore::new(daemon.blocks_dir()).unwrap();
    let block = store.get(&root.hash).await.unwrap().unwrap();
    assert!(store.delete(&root.hash).await.unwrap());
    drop(daemon);

    let staged_path = paths::provider_root_staging_path_in(config_dir.path());
    std::fs::write(
        &staged_path,
        serde_json::to_vec(&serde_json::json!({
            "root_cid": root.to_string(),
            "tombstone_base_root_cid": base_root.to_string(),
            "tombstone_paths": [],
            "updated_at": 1,
        }))
        .unwrap(),
    )
    .unwrap();
    let child = DaemonChild::spawn_relay_only(
        config_dir.path(),
        &relay.url,
        config_dir.path().join("daemon.log"),
        unused_loopback_port(),
    );

    // A transient import miss must yield to the live daemon instead of holding
    // its startup/config lock through the interactive CLI retry schedule.
    wait_for_startup_import_miss(&child, &staged_path).await;

    // Keep the block absent through two timer failures to prove the retained
    // work re-arms its recheck, independent of startup config-watch events.
    wait_for_retained_staging_rechecks(&child, &staged_path).await;

    // Restore only the block: no new config change, provider signal, or wake
    // request may be necessary to recover work retained across daemon startup.
    store.put(root.hash, block).await.unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while staged_path.exists() {
        assert!(
            Instant::now() < deadline,
            "staged import was not retried after its missing block arrived:\n{}",
            diagnostic_tail(&child),
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let output = config_dir.path().join("read-local.txt");
    run_json(
        config_dir.path(),
        &["provider", "read", "local.txt", output.to_str().unwrap()],
    );
    assert_eq!(std::fs::read(output).unwrap(), bytes);
}

async fn wait_for_startup_import_miss(child: &DaemonChild, staged_path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            staged_path.exists(),
            "missing staged directory was acknowledged as an empty import:\n{}",
            diagnostic_tail(child),
        );
        if child.log().lines().any(|line| {
            serde_json::from_str::<Value>(line).is_ok_and(|event| {
                event["event"] == "provider_root_staged_import_error"
                    && event["trigger"] == "startup"
            })
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "startup staged import did not promptly yield after a missing block:\n{}",
            diagnostic_tail(child),
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(staged_path.exists(), "failed import discarded staged work");
}

async fn wait_for_retained_staging_rechecks(child: &DaemonChild, staged_path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            staged_path.exists(),
            "retry discarded the unavailable staged write"
        );
        let retries = child
            .log()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|event| {
                event["event"] == "provider_root_staged_import_error"
                    && event["trigger"] == "provider_root_event_recheck"
                    && event["error"]
                        .as_str()
                        .is_some_and(|error| error.contains("Missing chunk"))
            })
            .count();
        if retries >= 2 {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "retained staged import did not re-arm its timer after a second miss:\n{}",
            diagnostic_tail(child),
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn diagnostic_tail(child: &DaemonChild) -> String {
    child
        .log()
        .lines()
        .rev()
        .take(12)
        .map(|line| {
            serde_json::from_str::<Value>(line).map_or_else(
                |_| line.chars().take(250).collect(),
                |event| format!("{} {} {}", event["event"], event["trigger"], event["error"]),
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
