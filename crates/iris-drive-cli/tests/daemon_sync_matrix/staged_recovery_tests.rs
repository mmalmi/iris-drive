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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn newly_approved_writer_uploads_existing_blocks_without_a_local_edit() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start_with_options(SyncClusterOptions {
        seed_files: vec![SeedFile::new(
            Client::Ubuntu,
            "offline-before-approval.txt",
            b"existing before approval",
        )],
        ..SyncClusterOptions::default()
    })
    .await;
    cluster.wait_until_authorized().await;
    assert_all_writer_blocks_uploaded(cluster.config_path(Client::Ubuntu)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn authorized_restart_retries_failed_storage_upload_without_an_edit() {
    let _guard = live_daemon_test_guard().await;
    let relay = LocalNostrRelay::spawn().await;
    let blossom = LocalBlossomServer::spawn().await;
    let config_dir = tempdir().unwrap();
    let dir = config_dir.path();
    run_json(dir, &["init", "--label", "writer"]);
    let config_path = paths::config_path_in(dir);
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    config.relays = vec![relay.url.clone()];
    config.blossom_servers = vec![blossom.url.clone()];
    config.local_nhash_resolver_enabled = false;
    config.save(&config_path).unwrap();
    let files = tempdir().unwrap();
    std::fs::write(files.path().join("before-restart.txt"), b"pending original").unwrap();
    run_json(dir, &["import", files.path().to_str().unwrap()]);
    blossom.set_uploads_offline(true);
    let log = dir.join("daemon.log");
    let child = DaemonChild::spawn_relay_only(dir, &relay.url, log, unused_loopback_port());
    tokio::time::timeout(Duration::from_secs(30), async {
        while !child
            .log()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|e| {
                e["event"] == "initial_publish_error"
                    || (e["event"] == "initial_publish" && e["blossom_upload_error"].is_string())
            })
        {
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .expect("startup must observe unavailable storage");
    blossom.set_uploads_offline(false);
    assert_all_writer_blocks_uploaded(dir).await;
}

async fn assert_all_writer_blocks_uploaded(dir: &Path) {
    let cfg = AppConfig::load_or_default(paths::config_path_in(dir)).unwrap();
    let root = iris_drive_core::publishable_root::publishable_app_key_root(
        dir,
        cfg.drive(iris_drive_core::PRIMARY_DRIVE_ID).unwrap(),
        cfg.profile.as_ref().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    let daemon = iris_drive_core::Daemon::open(dir).unwrap();
    let cid = Cid::parse(&root.root_cid).unwrap();
    let hashes = iris_drive_core::block_sync::collect_live_sync_hashes(daemon.tree(), &cid, 4)
        .await
        .unwrap();
    let reader =
        iris_drive_core::blossom_sync_client(nostr_sdk::Keys::generate(), &cfg.blossom_servers);
    tokio::time::timeout(Duration::from_secs(12), async {
        for hash in hashes {
            loop {
                if let Ok(bytes) = reader.download(&to_hex(&hash)).await {
                    assert_eq!(sha256(&bytes), hash);
                    break;
                }
                tokio::time::sleep(POLL_INTERVAL).await;
            }
        }
    }).await.expect("a fresh reader could not fetch the approved writer’s existing root from its configured storage");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_peer_download_recovers_when_storage_returns_without_another_announcement() {
    use hashtree_core::{HashTree, HashTreeConfig, MemoryStore};
    use iris_drive_core::{AppKey, AppKeyRootRef, Drive, Profile};
    use std::sync::Arc;

    let _guard = live_daemon_test_guard().await;
    let relay = LocalNostrRelay::spawn().await;
    let blossom = LocalBlossomServer::spawn().await;
    let source = Arc::new(MemoryStore::new());
    let tree = HashTree::new(HashTreeConfig::new(source.clone()));
    let bytes = b"download after the short retries expire";
    let (file, size) = tree.put_file(bytes).await.unwrap();
    let root = tree
        .put_directory(vec![
            DirEntry::from_cid("delayed.txt", &file)
                .with_size(size)
                .with_link_type(LinkType::File),
        ])
        .await
        .unwrap();
    let config_dir = tempdir().unwrap();
    let dir = config_dir.path();
    let mut profile = Profile::create(dir, Some("reader".into())).unwrap();
    let peer = AppKey::generate(dir.join("peer.key")).pubkey_hex();
    profile.approve_app_key(&peer, Some("peer".into())).unwrap();
    let mut drive = Drive::primary(profile.state.root_scope_id());
    drive
        .app_key_roots
        .insert(peer, AppKeyRootRef::legacy(root.to_string(), 1, 1));
    let mut config = AppConfig {
        profile: Some(profile.state),
        relays: vec![relay.url.clone()],
        blossom_servers: vec![blossom.url.clone()],
        local_nhash_resolver_enabled: false,
        ..AppConfig::default()
    };
    config.upsert_drive(drive);
    config.save(paths::config_path_in(dir)).unwrap();
    let child = DaemonChild::spawn_relay_only(
        dir,
        &relay.url,
        dir.join("daemon.log"),
        unused_loopback_port(),
    );
    let cid = root.to_string();
    tokio::time::timeout(Duration::from_secs(30), async {
        while !child
            .log()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|e| e["event"] == "block_download_error" && e["root_cid"] == cid)
        {
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .expect("the isolated reader must exhaust its first block requests");
    let storage = iris_drive_core::blossom_sync_client(
        nostr_sdk::Keys::generate(),
        std::slice::from_ref(&blossom.url),
    );
    for hash in iris_drive_core::block_sync::collect_live_sync_hashes(&tree, &root, 4)
        .await
        .unwrap()
    {
        let block = source.get(&hash).await.unwrap().unwrap();
        assert_eq!(storage.upload(&block).await.unwrap(), to_hex(&hash));
    }
    tokio::time::timeout(Duration::from_secs(15), async {
        while !child
            .log()
            .lines()
            .filter_map(|line| serde_json::from_str::<Value>(line).ok())
            .any(|e| e["event"] == "blossom_downloaded" && e["root_cid"] == cid)
        {
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    })
    .await
    .expect("pending remote blocks did not recover after storage was restored");
    let output = dir.join("recovered.txt");
    run_json(
        dir,
        &["provider", "read", "delayed.txt", output.to_str().unwrap()],
    );
    assert_eq!(std::fs::read(output).unwrap(), bytes);
}
