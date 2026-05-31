//! End-to-end tests for the `idrive` CLI.
//!
//! These exercise the actual compiled binary against a temp config dir,
//! so they catch arg-parsing, exit-code, and IO surprises. No mocks.

use assert_cmd::Command;
use hashtree_core::{Cid, HashTree, HashTreeConfig, Store, diff::collect_hashes, nhash_decode};
use hashtree_fs::FsBlobStore;
use hashtree_lmdb::LmdbBlobStore;
use iris_drive_core::{
    AppConfig, ConflictRecord, ConflictSide, ConflictState, PRIMARY_DRIVE_ID, paths::config_path_in,
};
use predicates::str::contains;
use std::process::Output;
use std::sync::Arc;
use tempfile::tempdir;

mod support;

use support::{LocalBlossomServer, LocalNostrRelay};

fn idrive(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("idrive").unwrap();
    cmd.env("IRIS_DRIVE_CONFIG_DIR", dir);
    cmd
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstatus: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_json(dir: &std::path::Path, args: &[&str]) -> serde_json::Value {
    let output = idrive(dir).args(args).output().unwrap();
    assert_success(&output);
    serde_json::from_slice(&output.stdout).unwrap_or_else(|err| {
        panic!(
            "invalid json: {err}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[test]
fn help_includes_desktop_operator_commands() {
    let dir = tempdir().unwrap();

    idrive(dir.path())
        .arg("--help")
        .assert()
        .success()
        .stdout(contains("version"))
        .stdout(contains("install-cli"))
        .stdout(contains("uninstall-cli"))
        .stdout(contains("devices"))
        .stdout(contains("stats"))
        .stdout(contains("daemon"));
}

#[test]
fn version_prints_plain_text_and_json() {
    let dir = tempdir().unwrap();

    idrive(dir.path())
        .arg("version")
        .assert()
        .success()
        .stdout(format!("{}\n", env!("CARGO_PKG_VERSION")));

    let value = run_json(dir.path(), &["version", "--json"]);
    assert_eq!(value["version"], env!("CARGO_PKG_VERSION"));
}

#[test]
fn install_cli_and_uninstall_cli_roundtrip_for_custom_path() {
    let dir = tempdir().unwrap();
    let target = dir.path().join(if cfg!(windows) {
        "idrive.exe"
    } else {
        "idrive"
    });
    let target_arg = target.to_string_lossy().into_owned();

    idrive(dir.path())
        .args(["install-cli", "--path", &target_arg])
        .assert()
        .success();
    assert!(target.exists(), "installed target should exist");

    idrive(dir.path())
        .args(["install-cli", "--path", &target_arg])
        .assert()
        .failure()
        .stderr(contains("already exists"));

    idrive(dir.path())
        .args(["install-cli", "--path", &target_arg, "--force"])
        .assert()
        .success();

    idrive(dir.path())
        .args(["uninstall-cli", "--path", &target_arg])
        .assert()
        .success();
    assert!(!target.exists(), "uninstall should remove target");
}

#[test]
fn stats_prints_gui_summary_counts() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();

    let value = run_json(dir.path(), &["stats"]);
    assert_eq!(value["initialized"], true);
    assert_eq!(value["files"], 0);
    assert_eq!(value["authorized_devices"], 1);
    assert_eq!(value["backup_targets"], 0);
    assert_eq!(value["unresolved_conflicts"], 0);
}

fn configure_local_blossom(config_dir: &std::path::Path, url: &str) {
    idrive(config_dir)
        .args(["blossom-servers", "remove", "https://upload.iris.to"])
        .assert()
        .success();
    idrive(config_dir)
        .args(["blossom-servers", "add", url])
        .assert()
        .success();
}

fn import_one_file(
    config_dir: &std::path::Path,
    file_name: &str,
    bytes: &[u8],
) -> tempfile::TempDir {
    let work = tempdir().unwrap();
    std::fs::write(work.path().join(file_name), bytes).unwrap();
    idrive(config_dir)
        .arg("import")
        .arg(work.path())
        .assert()
        .success();
    work
}

async fn assert_replica_contains_private_root<S>(
    store: Arc<S>,
    root_cid: &str,
    expected_hashes: u64,
) where
    S: Store + Send + Sync + 'static,
{
    let root = Cid::parse(root_cid).unwrap();
    assert!(
        root.key.is_some(),
        "backup roots must stay encrypted/private"
    );
    let tree = HashTree::new(HashTreeConfig::new(store));
    let hashes = collect_hashes(&tree, &root, 4).await.unwrap();
    assert_eq!(hashes.len() as u64, expected_hashes);
}

fn assert_tree_does_not_contain_bytes(path: &std::path::Path, needle: &[u8]) {
    for entry in std::fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            assert_tree_does_not_contain_bytes(&entry.path(), needle);
        } else if file_type.is_file() {
            let bytes = std::fs::read(entry.path()).unwrap();
            assert!(
                !bytes.windows(needle.len()).any(|window| window == needle),
                "replica file leaked plaintext: {}",
                entry.path().display()
            );
        }
    }
}

#[path = "cli_e2e/account_tests.rs"]
mod account_tests;

#[test]
fn whoami_before_init_errors() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("whoami").assert().failure();
}

#[test]
fn status_before_init_reports_uninitialized() {
    let dir = tempdir().unwrap();
    let out = idrive(dir.path()).arg("status").output().unwrap();
    assert!(out.status.success());
    let body = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["initialized"], false);
    assert!(v["drives"].as_array().unwrap().is_empty());
    assert_eq!(v["hashtree"]["local_block_count"], 0);
    assert_eq!(v["hashtree"]["local_block_bytes"], 0);
    assert_eq!(
        v["network"]["blossom_servers"],
        serde_json::json!(["https://upload.iris.to"])
    );
    assert_eq!(v["network"]["fips"]["enabled"], false);
    assert_eq!(v["network"]["fips"]["roster_peer_count"], 0);
    assert_eq!(v["network"]["fips"]["roster_connected_peer_count"], 0);
    assert_eq!(v["network"]["fips"]["other_peer_count"], 0);
    assert!(v["peers"].as_array().unwrap().is_empty());
    assert_eq!(v["conflicts"]["total_count"], 0);
    assert_eq!(v["conflicts"]["unresolved_count"], 0);
}

#[test]
fn status_after_init_reports_initialized() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    let out = idrive(dir.path()).arg("status").output().unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["initialized"], true);
    assert!(v["pubkey_npub"].as_str().unwrap().starts_with("npub1"));
    let drives = v["drives"].as_array().unwrap();
    assert_eq!(drives.len(), 1);
    assert_eq!(drives[0]["drive_id"], "main");
    assert_eq!(drives[0]["role"], "owner");
    assert!(drives[0].get("working_dir").is_none());
    assert_eq!(v["network"]["authorized_device_count"], 1);
    assert_eq!(v["network"]["published_device_roots"], 0);
    assert_eq!(v["network"]["fips"]["enabled"], false);
    assert_eq!(v["network"]["fips"]["roster_peer_count"], 0);
    assert_eq!(v["network"]["fips"]["roster_connected_peer_count"], 0);
    assert_eq!(v["network"]["fips"]["other_peer_count"], 0);
    let peers = v["peers"].as_array().unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0]["is_current_device"], true);
    assert_eq!(peers[0]["authorized"], true);
    assert_eq!(peers[0]["has_root"], false);
}

#[test]
fn backup_targets_can_be_added_listed_and_removed() {
    let dir = tempdir().unwrap();
    let init = run_json(dir.path(), &["init"]);
    let device_npub = init["device_npub"].as_str().unwrap();

    let added_blossom = run_json(
        dir.path(),
        &[
            "backups",
            "add",
            "https://backup.example",
            "--label",
            "Offsite",
        ],
    );
    assert_eq!(added_blossom["backup_targets"].as_array().unwrap().len(), 1);
    assert_eq!(added_blossom["backup_targets"][0]["kind"], "blossom");
    assert_eq!(added_blossom["backup_targets"][0]["label"], "Offsite");

    let added_fips = run_json(
        dir.path(),
        &["backups", "add", device_npub, "--label", "Vault"],
    );
    let targets = added_fips["backup_targets"].as_array().unwrap();
    assert_eq!(targets.len(), 2);
    assert!(targets.iter().any(|target| target["kind"] == "fips"));

    let status = run_json(dir.path(), &["status"]);
    assert_eq!(status["network"]["backup_target_count"], 3);
    assert_eq!(
        status["network"]["backup_targets"]
            .as_array()
            .unwrap()
            .len(),
        3
    );
    assert!(
        status["network"]["backup_targets"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["kind"] == "blossom"
                && target["target"] == "https://upload.iris.to"
                && target["label"] == "Blossom fallback")
    );

    let removed = run_json(dir.path(), &["backups", "remove", "https://backup.example"]);
    let remaining = removed["backup_targets"].as_array().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0]["kind"], "fips");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn configured_blossom_servers_are_listed_as_backup_targets() {
    let blossom = LocalBlossomServer::spawn().await;
    let cfg = tempdir().unwrap();

    run_json(cfg.path(), &["init", "--label", "owner"]);
    run_json(cfg.path(), &["blossom-servers", "add", &blossom.url]);

    let status = run_json(cfg.path(), &["status"]);
    let targets = status["network"]["backup_targets"].as_array().unwrap();
    let target = targets
        .iter()
        .find(|target| target["kind"] == "blossom" && target["target"] == blossom.url)
        .expect("configured Blossom server should be visible in backup targets");
    assert_eq!(target["enabled"], true);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backup_sync_uploads_private_root_to_blossom_target() {
    let blossom = LocalBlossomServer::spawn().await;
    let cfg = tempdir().unwrap();

    run_json(cfg.path(), &["init", "--label", "owner"]);
    let _work = import_one_file(cfg.path(), "backup.txt", b"encrypted backup material");
    run_json(
        cfg.path(),
        &["backups", "add", &blossom.url, "--label", "Local backup"],
    );

    let synced = run_json(cfg.path(), &["backups", "sync"]);
    let reports = synced["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0]["kind"], "blossom");
    assert_eq!(reports[0]["state"], "synced");
    assert!(
        reports[0]["upload"]["uploaded"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );
    let root_cid = Cid::parse(reports[0]["root_cid"].as_str().unwrap()).unwrap();
    assert!(
        root_cid.key.is_some(),
        "backup roots must stay encrypted/private"
    );
    assert!(blossom.blob_count().await > 0);

    let status = run_json(cfg.path(), &["status"]);
    let targets = status["network"]["backup_targets"].as_array().unwrap();
    let target = targets
        .iter()
        .find(|target| target["kind"] == "blossom" && target["target"] == blossom.url)
        .expect("synced Blossom backup target");
    assert_eq!(target["last_sync"]["state"], "synced");
    assert_eq!(target["last_sync"]["root_cid"], reports[0]["root_cid"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backup_check_records_sample_latency_and_bandwidth_for_blossom_target() {
    let blossom = LocalBlossomServer::spawn().await;
    let cfg = tempdir().unwrap();

    run_json(cfg.path(), &["init", "--label", "owner"]);
    let _work = import_one_file(
        cfg.path(),
        "backup.txt",
        b"encrypted backup material large enough for a transfer probe",
    );
    run_json(cfg.path(), &["backups", "add", &blossom.url]);
    run_json(cfg.path(), &["backups", "sync"]);

    let checked = run_json(cfg.path(), &["backups", "check", "--sample-size", "8"]);
    let reports = checked["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0]["kind"], "blossom");
    assert_eq!(reports[0]["state"], "verified");
    assert!(reports[0]["check"]["sampled_hashes"].as_u64().unwrap() > 0);
    assert_eq!(reports[0]["check"]["missing"], 0);
    assert_eq!(reports[0]["check"]["unknown"], 0);
    assert!(reports[0]["check"]["latency_ms"].as_u64().is_some());
    assert!(reports[0]["check"]["download_bytes"].as_u64().unwrap() > 0);
    assert!(
        reports[0]["check"]["download_bytes_per_second"]
            .as_u64()
            .unwrap()
            > 0
    );

    let status = run_json(cfg.path(), &["status"]);
    let target = status["network"]["backup_targets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|target| target["kind"] == "blossom" && target["target"] == blossom.url)
        .expect("checked Blossom backup target");
    assert_eq!(target["last_check"]["state"], "verified");
    assert!(target["last_check"]["latency_ms"].as_u64().is_some());
    assert!(
        target["last_check"]["download_bytes_per_second"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backup_sync_uploads_private_root_to_filesystem_target() {
    let cfg = tempdir().unwrap();
    let replica = tempdir().unwrap();
    let secret = b"filesystem replica plaintext must not leak";

    run_json(cfg.path(), &["init", "--label", "owner"]);
    let _work = import_one_file(cfg.path(), "backup.txt", secret);
    let target = format!("fs:{}", replica.path().display());
    run_json(
        cfg.path(),
        &["backups", "add", &target, "--label", "iCloud"],
    );

    let synced = run_json(cfg.path(), &["backups", "sync"]);
    let reports = synced["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0]["kind"], "filesystem");
    assert_eq!(reports[0]["state"], "synced");
    assert!(
        reports[0]["upload"]["uploaded"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );

    let total_hashes = reports[0]["upload"]["total_hashes"].as_u64().unwrap();
    let store = Arc::new(FsBlobStore::new(replica.path()).unwrap());
    assert_replica_contains_private_root(
        store,
        reports[0]["root_cid"].as_str().unwrap(),
        total_hashes,
    )
    .await;
    assert_tree_does_not_contain_bytes(replica.path(), secret);

    let status = run_json(cfg.path(), &["status"]);
    let targets = status["network"]["backup_targets"].as_array().unwrap();
    assert_eq!(targets[0]["kind"], "filesystem");
    assert_eq!(targets[0]["last_sync"]["state"], "synced");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn backup_sync_uploads_private_root_to_lmdb_target() {
    let cfg = tempdir().unwrap();
    let replica = tempdir().unwrap();
    let secret = b"lmdb replica plaintext must not leak";

    run_json(cfg.path(), &["init", "--label", "owner"]);
    let _work = import_one_file(cfg.path(), "backup.txt", secret);
    let target = format!("lmdb:{}", replica.path().display());
    run_json(
        cfg.path(),
        &["backups", "add", &target, "--label", "Local LMDB"],
    );

    let synced = run_json(cfg.path(), &["backups", "sync"]);
    let reports = synced["reports"].as_array().unwrap();
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0]["kind"], "lmdb");
    assert_eq!(reports[0]["state"], "synced");
    assert!(
        reports[0]["upload"]["uploaded"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );

    let total_hashes = reports[0]["upload"]["total_hashes"].as_u64().unwrap();
    let store = Arc::new(LmdbBlobStore::new(replica.path()).unwrap());
    assert_replica_contains_private_root(
        store,
        reports[0]["root_cid"].as_str().unwrap(),
        total_hashes,
    )
    .await;
    assert_tree_does_not_contain_bytes(replica.path(), secret);

    let status = run_json(cfg.path(), &["status"]);
    let targets = status["network"]["backup_targets"].as_array().unwrap();
    assert_eq!(targets[0]["kind"], "lmdb");
    assert_eq!(targets[0]["last_sync"]["state"], "synced");
}

#[test]
fn status_reports_fips_network_diagnostics_from_daemon_status() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        dir.path().join("daemon.lock"),
        std::process::id().to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("daemon-status.json"),
        serde_json::to_vec(&serde_json::json!({
            "updated_at": now,
            "fips_block_sync": {
                "endpoint_npub": "npub1local",
                "discovery_scope": "fips-overlay-v1",
                "nostr_discovery_app": "fips-overlay-v1",
                "udp_enabled": true,
                "udp_bind_addr": "0.0.0.0:2121",
                "udp_public": true,
                "udp_external_addr": "10.44.94.98:2121",
                "webrtc_enabled": true,
                "webrtc_max_connections": 16,
                "open_discovery_max_pending": 0,
                "mesh_peer_count": 1,
                "mesh_peers": ["npub1remote"],
                "authorized_peers": ["npub1remote"],
                "connected_peers": ["npub1remote", "npub1outside"],
                "peer_statuses": [{
                    "npub": "npub1remote",
                    "transport_addr": "udp:10.44.1.2:2121",
                    "transport_type": "udp",
                    "srtt_ms": 23,
                    "packets_sent": 5,
                    "packets_recv": 7,
                    "bytes_sent": 512,
                    "bytes_recv": 1024,
                }],
                "relay_statuses": [{"url": "wss://relay.example", "status": "connected"}],
            },
        }))
        .unwrap(),
    )
    .unwrap();

    let v = run_json(dir.path(), &["status"]);
    let fips = &v["network"]["fips"];
    assert_eq!(fips["enabled"], true);
    assert_eq!(fips["running"], true);
    assert_eq!(fips["fresh"], true);
    assert_eq!(fips["endpoint_npub"], "npub1local");
    assert_eq!(fips["discovery_scope"], "fips-overlay-v1");
    assert_eq!(fips["nostr_discovery_app"], "fips-overlay-v1");
    assert_eq!(fips["udp_enabled"], true);
    assert_eq!(fips["udp_bind_addr"], "0.0.0.0:2121");
    assert_eq!(fips["udp_public"], true);
    assert_eq!(fips["udp_external_addr"], "10.44.94.98:2121");
    assert_eq!(fips["webrtc_enabled"], true);
    assert_eq!(fips["webrtc_max_connections"], 16);
    assert_eq!(fips["open_discovery_max_pending"], 0);
    assert_eq!(fips["mesh_peer_count"], 1);
    assert_eq!(fips["mesh_peers"], serde_json::json!(["npub1remote"]));
    assert_eq!(fips["roster_peer_count"], 1);
    assert_eq!(fips["roster_connected_peer_count"], 1);
    assert_eq!(fips["other_peer_count"], 1);
    assert_eq!(fips["connected_peer_count"], 2);
    assert_eq!(
        fips["peer_statuses"],
        serde_json::json!([{
            "npub": "npub1remote",
            "transport_addr": "udp:10.44.1.2:2121",
            "transport_type": "udp",
            "srtt_ms": 23,
            "packets_sent": 5,
            "packets_recv": 7,
            "bytes_sent": 512,
            "bytes_recv": 1024,
        }])
    );
    assert_eq!(
        fips["relay_statuses"],
        serde_json::json!([{"url": "wss://relay.example", "status": "connected"}])
    );
}

#[test]
fn status_reports_fips_latency_for_direct_peer() {
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let owner = run_json(owner_dir.path(), &["init", "--label", "macos"]);
    let owner_npub = owner["owner_npub"].as_str().unwrap().to_string();
    let linked = run_json(
        linked_dir.path(),
        &["link", &owner_npub, "--label", "linux-peer"],
    );
    let linked_device_npub = linked["device_npub"].as_str().unwrap().to_string();
    run_json(owner_dir.path(), &["approve", &linked_device_npub]);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        owner_dir.path().join("daemon.lock"),
        std::process::id().to_string(),
    )
    .unwrap();
    std::fs::write(
        owner_dir.path().join("daemon-status.json"),
        serde_json::to_vec(&serde_json::json!({
            "updated_at": now,
            "fips_block_sync": {
                "endpoint_npub": "npub1local",
                "authorized_peers": [linked_device_npub.clone()],
                "connected_peers": [linked_device_npub.clone()],
                "mesh_peers": [],
                "peer_statuses": [{
                    "npub": linked_device_npub.clone(),
                    "transport_addr": "udp:10.44.1.2:2121",
                    "transport_type": "udp",
                    "srtt_ms": 31,
                    "packets_sent": 11,
                    "packets_recv": 13,
                    "bytes_sent": 2048,
                    "bytes_recv": 4096,
                }],
            },
        }))
        .unwrap(),
    )
    .unwrap();

    let status = run_json(owner_dir.path(), &["status"]);
    let linked_peer = status["peers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|peer| peer["device_npub"] == linked_device_npub)
        .expect("linked device peer");
    assert_eq!(linked_peer["fips_online"], true);
    assert_eq!(linked_peer["fips_online_via"], "direct");
    assert_eq!(linked_peer["fips_transport_type"], "udp");
    assert_eq!(linked_peer["fips_transport_addr"], "udp:10.44.1.2:2121");
    assert_eq!(linked_peer["fips_srtt_ms"], 31);
    assert_eq!(linked_peer["fips_ping_ms"], 31);
    assert_eq!(linked_peer["fips_packets_sent"], 11);
    assert_eq!(linked_peer["fips_packets_recv"], 13);
    assert_eq!(linked_peer["fips_bytes_sent"], 2048);
    assert_eq!(linked_peer["fips_bytes_recv"], 4096);
}

#[test]
fn status_marks_mesh_fips_peer_online_without_direct_endpoint_link() {
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let owner = run_json(owner_dir.path(), &["init", "--label", "macos"]);
    let owner_npub = owner["owner_npub"].as_str().unwrap().to_string();
    let linked = run_json(
        linked_dir.path(),
        &["link", &owner_npub, "--label", "linux-peer"],
    );
    let linked_device_npub = linked["device_npub"].as_str().unwrap().to_string();
    run_json(owner_dir.path(), &["approve", &linked_device_npub]);

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        owner_dir.path().join("daemon.lock"),
        std::process::id().to_string(),
    )
    .unwrap();
    std::fs::write(
        owner_dir.path().join("daemon-status.json"),
        serde_json::to_vec(&serde_json::json!({
            "updated_at": now,
            "fips_block_sync": {
                "endpoint_npub": "npub1local",
                "authorized_peers": [linked_device_npub.clone()],
                "connected_peers": [],
                "mesh_peers": [linked_device_npub.clone()],
            },
        }))
        .unwrap(),
    )
    .unwrap();

    let status = run_json(owner_dir.path(), &["status"]);
    let linked_peer = status["peers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|peer| peer["device_npub"] == linked_device_npub)
        .expect("linked device peer");
    assert_eq!(linked_peer["fips_online"], true);
    assert_eq!(linked_peer["fips_direct_online"], false);
    assert_eq!(linked_peer["fips_mesh_online"], true);
    assert_eq!(linked_peer["fips_online_via"], "mesh");
}

#[test]
fn status_drops_stale_fips_mesh_peers() {
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let owner = run_json(owner_dir.path(), &["init", "--label", "macos"]);
    let owner_npub = owner["owner_npub"].as_str().unwrap().to_string();
    let linked = run_json(
        linked_dir.path(),
        &["link", &owner_npub, "--label", "linux-peer"],
    );
    let linked_device_npub = linked["device_npub"].as_str().unwrap().to_string();
    run_json(owner_dir.path(), &["approve", &linked_device_npub]);

    std::fs::write(
        owner_dir.path().join("daemon-status.json"),
        serde_json::to_vec(&serde_json::json!({
            "updated_at": 1,
            "fips_block_sync": {
                "endpoint_npub": "npub1local",
                "authorized_peers": [linked_device_npub.clone()],
                "connected_peers": [linked_device_npub.clone()],
                "mesh_peers": [linked_device_npub.clone()],
                "peer_statuses": [{
                    "npub": linked_device_npub.clone(),
                    "transport_type": "udp",
                    "srtt_ms": 19,
                }],
            },
        }))
        .unwrap(),
    )
    .unwrap();

    let status = run_json(owner_dir.path(), &["status"]);
    let fips = &status["network"]["fips"];
    assert_eq!(fips["fresh"], false);
    assert_eq!(fips["connected_peers"], serde_json::json!([]));
    assert_eq!(fips["mesh_peers"], serde_json::json!([]));
    assert_eq!(fips["peer_statuses"], serde_json::json!([]));
    let linked_peer = status["peers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|peer| peer["device_npub"] == linked_device_npub)
        .expect("linked device peer");
    assert_eq!(linked_peer["fips_online"], false);
    assert_eq!(linked_peer["fips_online_via"], serde_json::Value::Null);
}

#[test]
fn status_marks_current_device_fips_online_when_daemon_is_running() {
    let dir = tempdir().unwrap();
    let init = run_json(dir.path(), &["init", "--label", "macos"]);
    let device_npub = init["device_npub"].as_str().unwrap().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        dir.path().join("daemon.lock"),
        std::process::id().to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("daemon-status.json"),
        serde_json::to_vec(&serde_json::json!({
            "updated_at": now,
            "fips_block_sync": {
                "endpoint_npub": device_npub,
                "authorized_peers": [],
                "connected_peers": [],
                "mesh_peers": [],
            },
        }))
        .unwrap(),
    )
    .unwrap();

    let status = run_json(dir.path(), &["status"]);
    let current_peer = status["peers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|peer| peer["is_current_device"] == true)
        .expect("current device peer");
    assert_eq!(current_peer["fips_online"], true);
    assert_eq!(current_peer["fips_online_via"], "local");
}

#[test]
fn status_marks_current_device_local_online_without_fips_transport_snapshot() {
    let dir = tempdir().unwrap();
    run_json(dir.path(), &["init", "--label", "macos"]);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(
        dir.path().join("daemon.lock"),
        std::process::id().to_string(),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("daemon-status.json"),
        serde_json::to_vec(&serde_json::json!({
            "updated_at": now,
        }))
        .unwrap(),
    )
    .unwrap();

    let status = run_json(dir.path(), &["status"]);
    let current_peer = status["peers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|peer| peer["is_current_device"] == true)
        .expect("current device peer");
    assert_eq!(current_peer["fips_online"], true);
    assert_eq!(current_peer["fips_online_via"], "local");
}

#[test]
fn conflicts_resolve_marks_record_resolved_in_current_root() {
    let cfg = tempdir().unwrap();
    let work = tempdir().unwrap();
    std::fs::write(work.path().join("report.pdf"), b"chosen").unwrap();
    idrive(cfg.path()).arg("init").assert().success();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();
    seed_conflict_record(cfg.path(), "conflict-a");

    let before = idrive(cfg.path()).arg("status").output().unwrap();
    assert!(before.status.success(), "{before:?}");
    let before_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8(before.stdout).unwrap()).unwrap();
    assert_eq!(before_json["conflicts"]["unresolved_count"], 1);

    let resolved = idrive(cfg.path())
        .args(["conflicts", "resolve", "conflict-a"])
        .output()
        .unwrap();
    assert!(resolved.status.success(), "{resolved:?}");
    let resolved_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8(resolved.stdout).unwrap()).unwrap();
    assert_eq!(resolved_json["conflict_id"], "conflict-a");
    assert_eq!(resolved_json["changed"], true);

    let after = idrive(cfg.path()).arg("status").output().unwrap();
    assert!(after.status.success(), "{after:?}");
    let after_json: serde_json::Value =
        serde_json::from_str(&String::from_utf8(after.stdout).unwrap()).unwrap();
    assert_eq!(after_json["conflicts"]["unresolved_count"], 0);
    assert_eq!(after_json["conflicts"]["resolved_count"], 1);
}

fn seed_conflict_record(config_dir: &std::path::Path, conflict_id: &str) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let mut config = AppConfig::load_or_default(config_path_in(config_dir)).unwrap();
        let root_cid = config
            .drive(PRIMARY_DRIVE_ID)
            .and_then(|drive| drive.last_root_cid.clone())
            .unwrap();
        let root = Cid::parse(&root_cid).unwrap();
        let store = FsBlobStore::new(config_dir.join("blocks")).unwrap();
        let tree = HashTree::new(HashTreeConfig::new(Arc::new(store)));
        let record = ConflictRecord {
            schema: ConflictRecord::SCHEMA,
            conflict_id: conflict_id.into(),
            path: "report.pdf".into(),
            visible_conflict_path: "report (conflict from phone).pdf".into(),
            local: ConflictSide {
                device_id: "laptop".into(),
                device_seq: 1,
                root_cid: root_cid.clone(),
                whole_file_hash: "hash-local".into(),
            },
            remote: Some(ConflictSide {
                device_id: "phone".into(),
                device_seq: 1,
                root_cid: "cid-remote".into(),
                whole_file_hash: "hash-remote".into(),
            }),
            deleted: None,
            state: ConflictState::Unresolved,
            created_at: 1234,
        };
        let new_root = iris_drive_core::layer_conflict_records(&tree, root, &[record])
            .await
            .unwrap();
        let account_device = config.account.as_ref().unwrap().device_pubkey.clone();
        let drive = config
            .drives
            .iter_mut()
            .find(|drive| drive.drive_id == PRIMARY_DRIVE_ID)
            .unwrap();
        drive.last_root_cid = Some(new_root.to_string());
        drive
            .device_roots
            .get_mut(&account_device)
            .unwrap()
            .root_cid = new_root.to_string();
        config.save(config_path_in(config_dir)).unwrap();
    });
}

#[path = "cli_e2e/tail_tests.rs"]
mod tail_tests;
