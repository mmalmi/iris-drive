//! End-to-end tests for the `idrive` CLI.
//!
//! These exercise the actual compiled binary against a temp config dir,
//! so they catch arg-parsing, exit-code, and IO surprises. No mocks.

use assert_cmd::Command;
use axum::{
    Router,
    body::{Body, Bytes},
    extract::{
        Path as AxumPath, State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::Response,
    routing::{get, put},
};
use futures::StreamExt;
use hashtree_core::{
    Cid, HashTree, HashTreeConfig, Store, diff::collect_hashes, nhash_decode, sha256, to_hex,
};
use hashtree_fs::FsBlobStore;
use hashtree_lmdb::LmdbBlobStore;
use iris_drive_core::{
    AppConfig, ConflictRecord, ConflictSide, ConflictState, PRIMARY_DRIVE_ID, paths::config_path_in,
};
use predicates::str::contains;
use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::process::Output;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio::sync::Mutex;

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

#[test]
fn init_creates_key_and_config() {
    let dir = tempdir().unwrap();
    idrive(dir.path())
        .arg("init")
        .assert()
        .success()
        .stdout(contains("npub1"))
        .stdout(contains("main"));
    assert!(dir.path().join("key").exists());
    assert!(dir.path().join("owner_key").exists()); // create flow also writes owner
    assert!(dir.path().join("config.toml").exists());
}

#[test]
fn init_yields_authorized_owner_capable_account() {
    let dir = tempdir().unwrap();
    let out = idrive(dir.path()).arg("init").output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["has_owner_signing_authority"], true);
    assert_eq!(v["authorization_state"], "authorized");
    assert!(v["owner_npub"].as_str().unwrap().starts_with("npub1"));
    assert!(v["device_npub"].as_str().unwrap().starts_with("npub1"));
}

#[test]
fn restore_uses_provided_nsec_and_grants_owner_authority() {
    // Capture original owner npub from an init.
    let dir_a = tempdir().unwrap();
    idrive(dir_a.path()).arg("init").assert().success();
    // Read the persisted owner nsec from disk to drive `restore`.
    let nsec = std::fs::read_to_string(dir_a.path().join("owner_key"))
        .unwrap()
        .trim()
        .to_string();
    let original_owner =
        String::from_utf8(idrive(dir_a.path()).arg("whoami").output().unwrap().stdout).unwrap();
    let original_v: serde_json::Value = serde_json::from_str(&original_owner).unwrap();
    let original_owner_npub = original_v["owner_npub"].as_str().unwrap().to_string();

    let dir_b = tempdir().unwrap();
    let out = idrive(dir_b.path())
        .args(["restore", &nsec])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["owner_npub"], original_owner_npub);
    assert_eq!(v["has_owner_signing_authority"], true);
    // Device key must differ.
    assert_ne!(v["device_npub"], original_v["device_npub"]);
    assert!(dir_b.path().join("owner_key").exists());
}

#[test]
fn link_creates_awaiting_device_with_no_owner_key() {
    let dir = tempdir().unwrap();
    // Use the test owner's npub from a separate init.
    let owner_dir = tempdir().unwrap();
    let init_v: serde_json::Value = serde_json::from_str(
        &String::from_utf8(
            idrive(owner_dir.path())
                .arg("init")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap();
    let owner_npub = init_v["owner_npub"].as_str().unwrap().to_string();

    let out = idrive(dir.path())
        .args(["link", &owner_npub])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["owner_npub"], owner_npub);
    assert_eq!(v["has_owner_signing_authority"], false);
    assert_eq!(v["authorization_state"], "awaiting_approval");
    assert_eq!(v["device_link_request"]["owner_npub"], owner_npub);
    assert_eq!(v["device_link_request"]["device_npub"], v["device_npub"]);
    assert!(
        v["device_link_request"]["url"]
            .as_str()
            .unwrap()
            .starts_with("iris-drive://device-link?")
    );
    assert!(dir.path().join("key").exists());
    assert!(!dir.path().join("owner_key").exists()); // never on a linked device
}

#[test]
fn link_then_approve_authorizes_the_linked_device() {
    // Set up owner-capable install + a separate linked install.
    let owner_dir = tempdir().unwrap();
    idrive(owner_dir.path()).arg("init").assert().success();
    let owner_npub = serde_json::from_str::<serde_json::Value>(
        &String::from_utf8(
            idrive(owner_dir.path())
                .arg("whoami")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap()["owner_npub"]
        .as_str()
        .unwrap()
        .to_string();

    let linked_dir = tempdir().unwrap();
    let linked_v: serde_json::Value = serde_json::from_str(
        &String::from_utf8(
            idrive(linked_dir.path())
                .args(["link", &owner_npub])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap();
    let linked_device_npub = linked_v["device_npub"].as_str().unwrap().to_string();

    // Owner approves the linked device.
    let approve = idrive(owner_dir.path())
        .args(["approve", &linked_device_npub])
        .output()
        .unwrap();
    assert!(approve.status.success(), "{approve:?}");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(approve.stdout).unwrap()).unwrap();
    assert_eq!(v["roster_size"], 2);

    // Roster on the owner side now has 2 devices.
    let roster = serde_json::from_str::<serde_json::Value>(
        &String::from_utf8(
            idrive(owner_dir.path())
                .arg("roster")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(roster["app_keys"]["devices"].as_array().unwrap().len(), 2);
}

#[test]
fn owner_approves_device_request_link() {
    let owner_dir = tempdir().unwrap();
    let other_owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();

    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let owner_npub = owner["owner_npub"].as_str().unwrap().to_string();
    run_json(other_owner_dir.path(), &["init", "--label", "other-admin"]);

    let linked = run_json(
        linked_dir.path(),
        &["link", &owner_npub, "--label", "windows-peer"],
    );
    let request_url = linked["device_link_request"]["url"].as_str().unwrap();

    idrive(other_owner_dir.path())
        .args(["approve", request_url])
        .assert()
        .failure()
        .stderr(contains("different owner"));

    let approved = run_json(owner_dir.path(), &["approve", request_url]);
    assert_eq!(approved["roster_size"], 2);

    let roster = run_json(owner_dir.path(), &["roster"]);
    let devices = roster["app_keys"]["devices"].as_array().unwrap();
    assert!(
        devices
            .iter()
            .any(|device| device["label"].as_str() == Some("windows-peer"))
    );
}

#[test]
fn owner_can_revoke_a_linked_device() {
    let owner_dir = tempdir().unwrap();
    idrive(owner_dir.path()).arg("init").assert().success();
    let owner_npub = serde_json::from_str::<serde_json::Value>(
        &String::from_utf8(
            idrive(owner_dir.path())
                .arg("whoami")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap()["owner_npub"]
        .as_str()
        .unwrap()
        .to_string();

    let linked_dir = tempdir().unwrap();
    let linked_v: serde_json::Value = serde_json::from_str(
        &String::from_utf8(
            idrive(linked_dir.path())
                .args(["link", &owner_npub])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap();
    let linked_device_npub = linked_v["device_npub"].as_str().unwrap().to_string();

    idrive(owner_dir.path())
        .args(["approve", &linked_device_npub])
        .assert()
        .success();
    let revoked = run_json(owner_dir.path(), &["revoke", &linked_device_npub]);
    assert_eq!(revoked["roster_size"], 1);
    assert!(revoked["dck_generation"].as_u64().unwrap() > 1);

    let roster = run_json(owner_dir.path(), &["roster"]);
    let devices = roster["app_keys"]["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 1);
    assert_ne!(devices[0]["npub"], linked_device_npub);
}

#[test]
fn approve_without_owner_authority_errors() {
    // Linked-only device tries to approve.
    let owner_dir = tempdir().unwrap();
    idrive(owner_dir.path()).arg("init").assert().success();
    let owner_npub = serde_json::from_str::<serde_json::Value>(
        &String::from_utf8(
            idrive(owner_dir.path())
                .arg("whoami")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap()["owner_npub"]
        .as_str()
        .unwrap()
        .to_string();
    let linked_dir = tempdir().unwrap();
    idrive(linked_dir.path())
        .args(["link", &owner_npub])
        .assert()
        .success();
    idrive(linked_dir.path())
        .args(["approve", &"ff".repeat(32)])
        .assert()
        .failure();
}

#[test]
fn roster_before_init_errors() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("roster").assert().failure();
}

#[test]
fn roster_after_init_shows_dck_generation_and_self_wrap() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    let out = idrive(dir.path()).arg("roster").output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["app_keys"]["dck_generation"], 1);
    let devices = v["app_keys"]["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(devices[0]["has_dck_wrap"], true);
    assert_eq!(devices[0]["is_current_device"], true);
}

#[test]
fn rotate_dck_advances_generation() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    let before: serde_json::Value = serde_json::from_str(
        &String::from_utf8(idrive(dir.path()).arg("roster").output().unwrap().stdout).unwrap(),
    )
    .unwrap();
    let gen_before = before["app_keys"]["dck_generation"].as_u64().unwrap();

    let out = idrive(dir.path()).arg("rotate-dck").output().unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    let gen_after = v["dck_generation"].as_u64().unwrap();
    assert!(
        gen_after > gen_before,
        "{gen_after} should exceed {gen_before}"
    );
}

#[test]
fn rotate_dck_on_linked_device_errors() {
    let owner_dir = tempdir().unwrap();
    idrive(owner_dir.path()).arg("init").assert().success();
    let owner_npub = serde_json::from_str::<serde_json::Value>(
        &String::from_utf8(
            idrive(owner_dir.path())
                .arg("whoami")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap()["owner_npub"]
        .as_str()
        .unwrap()
        .to_string();
    let linked_dir = tempdir().unwrap();
    idrive(linked_dir.path())
        .args(["link", &owner_npub])
        .assert()
        .success();
    idrive(linked_dir.path())
        .arg("rotate-dck")
        .assert()
        .failure();
}

#[test]
fn approve_advances_dck_generation() {
    let owner_dir = tempdir().unwrap();
    idrive(owner_dir.path()).arg("init").assert().success();
    let gen_before = serde_json::from_str::<serde_json::Value>(
        &String::from_utf8(
            idrive(owner_dir.path())
                .arg("roster")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap()["app_keys"]["dck_generation"]
        .as_u64()
        .unwrap();

    let owner_npub = serde_json::from_str::<serde_json::Value>(
        &String::from_utf8(
            idrive(owner_dir.path())
                .arg("whoami")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap()["owner_npub"]
        .as_str()
        .unwrap()
        .to_string();
    let linked_dir = tempdir().unwrap();
    let linked_v: serde_json::Value = serde_json::from_str(
        &String::from_utf8(
            idrive(linked_dir.path())
                .args(["link", &owner_npub])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap();
    let linked_device_npub = linked_v["device_npub"].as_str().unwrap().to_string();

    idrive(owner_dir.path())
        .args(["approve", &linked_device_npub])
        .assert()
        .success();

    let gen_after = serde_json::from_str::<serde_json::Value>(
        &String::from_utf8(
            idrive(owner_dir.path())
                .arg("roster")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap()["app_keys"]["dck_generation"]
        .as_u64()
        .unwrap();
    assert!(
        gen_after > gen_before,
        "{gen_after} should exceed {gen_before}"
    );
}

#[test]
fn double_init_errors_without_force() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    idrive(dir.path()).arg("init").assert().failure();
}

#[test]
fn double_init_with_force_succeeds() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    idrive(dir.path())
        .args(["init", "--force"])
        .assert()
        .success()
        .stdout(contains("npub1"));
}

#[test]
fn whoami_after_init_reports_owner_and_device() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    let out = idrive(dir.path()).arg("whoami").output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert!(v["owner_npub"].as_str().unwrap().starts_with("npub1"));
    assert!(v["device_npub"].as_str().unwrap().starts_with("npub1"));
    assert_eq!(v["has_owner_signing_authority"], true);
}

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
    assert_eq!(drives[0]["working_dir"], serde_json::Value::Null);
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
    assert_eq!(status["network"]["backup_target_count"], 2);
    assert_eq!(
        status["network"]["backup_targets"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let removed = run_json(dir.path(), &["backups", "remove", "https://backup.example"]);
    let remaining = removed["backup_targets"].as_array().unwrap();
    assert_eq!(remaining.len(), 1);
    assert_eq!(remaining[0]["kind"], "fips");
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
    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0]["last_sync"]["state"], "synced");
    assert_eq!(targets[0]["last_sync"]["root_cid"], reports[0]["root_cid"]);
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
                "discovery_scope": "iris-drive-v1:owner",
                "authorized_peers": ["npub1remote"],
                "connected_peers": ["npub1remote", "npub1outside"],
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
    assert_eq!(fips["discovery_scope"], "iris-drive-v1:owner");
    assert_eq!(fips["roster_peer_count"], 1);
    assert_eq!(fips["roster_connected_peer_count"], 1);
    assert_eq!(fips["other_peer_count"], 1);
    assert_eq!(fips["connected_peer_count"], 2);
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

#[test]
fn relays_can_be_edited_from_cli() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();

    idrive(dir.path())
        .args(["relays", "add", "relay.example"])
        .assert()
        .success();
    let added: serde_json::Value =
        serde_json::from_slice(&idrive(dir.path()).arg("relays").output().unwrap().stdout).unwrap();
    assert_eq!(
        added.as_array().unwrap().last().unwrap(),
        "wss://relay.example"
    );

    idrive(dir.path())
        .args(["relays", "update", "relay.example", "wss://relay2.example"])
        .assert()
        .success();
    let updated: serde_json::Value =
        serde_json::from_slice(&idrive(dir.path()).arg("relays").output().unwrap().stdout).unwrap();
    assert!(
        updated
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("wss://relay2.example"))
    );
    assert!(
        !updated
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("wss://relay.example"))
    );

    idrive(dir.path())
        .args(["relays", "remove", "wss://relay2.example"])
        .assert()
        .success();
    let removed: serde_json::Value =
        serde_json::from_slice(&idrive(dir.path()).arg("relays").output().unwrap().stdout).unwrap();
    assert!(
        !removed
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("wss://relay2.example"))
    );

    idrive(dir.path())
        .args(["relays", "reset"])
        .assert()
        .success();
    let reset: serde_json::Value =
        serde_json::from_slice(&idrive(dir.path()).arg("relays").output().unwrap().stdout).unwrap();
    assert_eq!(
        reset,
        serde_json::json!([
            "wss://temp.iris.to",
            "wss://relay.damus.io",
            "wss://relay.snort.social",
            "wss://relay.primal.net",
            "wss://upload.iris.to/nostr"
        ])
    );
}

#[test]
fn drives_lists_primary_after_init() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    idrive(dir.path())
        .arg("drives")
        .assert()
        .success()
        .stdout(contains("main"))
        .stdout(contains("owner"))
        .stdout(contains("My Drive"));
}

#[test]
fn drives_before_init_shows_empty_message() {
    let dir = tempdir().unwrap();
    idrive(dir.path())
        .arg("drives")
        .assert()
        .success()
        .stdout(contains("idrive init"));
}

#[test]
fn index_command_prints_root_cid_and_count() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"alpha").unwrap();
    std::fs::write(dir.path().join("b.txt"), b"beta").unwrap();
    let cfg = tempdir().unwrap();
    let out = idrive(cfg.path())
        .arg("index")
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let body = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["top_level_entries"], 2);
    let root_cid = Cid::parse(v["root_cid"].as_str().unwrap()).unwrap();
    assert!(
        root_cid.key.is_some(),
        "index roots should be private by default"
    );
}

#[test]
fn import_persists_to_blocks_dir_and_advances_root() {
    let work = tempdir().unwrap();
    std::fs::create_dir(work.path().join("junk")).unwrap();
    std::fs::write(work.path().join("junk").join("hello.txt"), b"hi there").unwrap();
    std::fs::write(work.path().join("junk").join("again.txt"), b"still here").unwrap();

    let cfg = tempdir().unwrap();
    idrive(cfg.path()).arg("init").assert().success();

    let out = idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    let root_cid = v["root_cid"].as_str().unwrap();
    assert_eq!(v["top_level_entries"], 1);
    assert!(cfg.path().join("blocks").is_dir());
    assert!(!root_cid.is_empty());

    // status now reports the recorded root CID on the primary drive.
    let status_out = idrive(cfg.path()).arg("status").output().unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(status_out.stdout).unwrap()).unwrap();
    assert_eq!(v["drives"][0]["last_root_cid"], root_cid);
    assert_eq!(
        v["drives"][0]["working_dir"],
        work.path().display().to_string()
    );
    assert_eq!(v["hashtree"]["current_root_cid"], root_cid);
    assert_eq!(v["hashtree"]["current_root_private"], true);
    let owner_npub = v["account"]["owner_npub"].as_str().unwrap();
    assert_eq!(
        v["hashtree"]["drive_iris_to_url"],
        format!("https://drive.iris.to/#/{owner_npub}/main")
    );
    assert_eq!(
        v["hashtree"]["files_iris_to_url"],
        v["hashtree"]["drive_iris_to_url"]
    );
    let snapshot_url = v["hashtree"]["snapshot_url"].as_str().unwrap();
    assert_eq!(v["hashtree"]["permalink_url"], snapshot_url);
    let nhash = snapshot_url
        .strip_prefix("https://drive.iris.to/#/")
        .expect("snapshot link should use drive.iris.to nhash route");
    let decoded = nhash_decode(nhash).expect("decode snapshot link nhash");
    let cid = Cid::parse(root_cid).expect("parse root cid");
    assert_eq!(decoded.hash, cid.hash);
    assert_eq!(decoded.decrypt_key, cid.key);
    assert_eq!(v["hashtree"]["file_count"], 2);
    assert_eq!(v["hashtree"]["top_level_entries"], 1);
    assert!(v["hashtree"]["local_block_count"].as_u64().unwrap() > 0);
    assert!(v["hashtree"]["local_block_bytes"].as_u64().unwrap() > 0);
    assert_eq!(v["network"]["published_device_roots"], 1);
    assert_eq!(v["peers"][0]["has_root"], true);
    assert_eq!(v["peers"][0]["root_cid"], root_cid);
    assert_eq!(v["peers"][0]["root_private"], true);
}

#[test]
fn import_before_init_errors() {
    let work = tempdir().unwrap();
    let cfg = tempdir().unwrap();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .failure();
}

#[test]
fn npub_is_stable_across_invocations() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    let one = String::from_utf8(idrive(dir.path()).arg("whoami").output().unwrap().stdout).unwrap();
    let two = String::from_utf8(idrive(dir.path()).arg("whoami").output().unwrap().stdout).unwrap();
    assert_eq!(one, two);
}

#[test]
fn list_after_import_shows_merged_view() {
    let cfg = tempdir().unwrap();
    let work = tempdir().unwrap();
    std::fs::write(work.path().join("alpha.txt"), b"alpha").unwrap();
    std::fs::write(work.path().join("beta.txt"), b"beta-bytes").unwrap();

    idrive(cfg.path()).arg("init").assert().success();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();

    let out = idrive(cfg.path()).arg("list").output().unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["authorized_devices"], 1);
    assert_eq!(v["device_roots_present"], 1);
    let files = v["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    let paths: Vec<&str> = files.iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert_eq!(paths, vec!["alpha.txt", "beta.txt"]);
    // Sizes recorded.
    assert_eq!(files[0]["size"], 5);
    assert_eq!(files[1]["size"], 10);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn linked_devices_sync_each_others_files_through_cli() {
    let relay = LocalNostrRelay::spawn().await;
    let blossom = LocalBlossomServer::spawn().await;

    let cfg_a = tempdir().unwrap();
    let cfg_b = tempdir().unwrap();
    let work_a = tempdir().unwrap();
    let work_b = tempdir().unwrap();

    configure_local_blossom(cfg_a.path(), &blossom.url);
    configure_local_blossom(cfg_b.path(), &blossom.url);

    let init_a = run_json(cfg_a.path(), &["init", "--label", "device-a"]);
    let owner_npub = init_a["owner_npub"].as_str().unwrap().to_string();

    let linked_b = run_json(cfg_b.path(), &["link", &owner_npub, "--label", "device-b"]);
    let device_b_request = linked_b["device_link_request"]["url"]
        .as_str()
        .unwrap()
        .to_string();

    let approved = run_json(cfg_a.path(), &["approve", &device_b_request]);
    assert_eq!(approved["roster_size"], 2);

    run_json(cfg_b.path(), &["import", work_b.path().to_str().unwrap()]);

    std::fs::write(work_a.path().join("from-a.txt"), b"hello from a").unwrap();
    run_json(cfg_a.path(), &["import", work_a.path().to_str().unwrap()]);
    let publish_a = run_json(
        cfg_a.path(),
        &["publish", "--relay", &relay.url, "--timeout", "2"],
    );
    assert_eq!(publish_a["published_app_keys"], true);
    assert_eq!(publish_a["published_drive_root"], true);
    assert!(
        publish_a["blossom_upload"]["uploaded"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );

    let sync_b = run_json(
        cfg_b.path(),
        &["sync", "--relay", &relay.url, "--timeout", "2"],
    );
    assert_eq!(sync_b["drive_root_events_applied"], 1);
    assert!(
        sync_b["blossom_download"]["fetched"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );
    assert_list_paths(cfg_b.path(), &["from-a.txt"]);
    assert_eq!(
        std::fs::read(work_b.path().join("from-a.txt")).unwrap(),
        b"hello from a"
    );

    std::fs::write(work_b.path().join("from-b.txt"), b"hello from b").unwrap();
    run_json(cfg_b.path(), &["import", work_b.path().to_str().unwrap()]);
    let publish_b = run_json(
        cfg_b.path(),
        &["publish", "--relay", &relay.url, "--timeout", "2"],
    );
    assert_eq!(publish_b["published_app_keys"], false);
    assert_eq!(publish_b["published_drive_root"], true);

    let sync_a = run_json(
        cfg_a.path(),
        &["sync", "--relay", &relay.url, "--timeout", "2"],
    );
    assert_eq!(sync_a["drive_root_events_applied"], 1);
    assert_list_paths(cfg_a.path(), &["from-a.txt", "from-b.txt"]);
    assert_eq!(
        std::fs::read(work_a.path().join("from-b.txt")).unwrap(),
        b"hello from b"
    );
}

fn assert_list_paths(config_dir: &std::path::Path, expected: &[&str]) {
    let listing = run_json(config_dir, &["list"]);
    let paths: Vec<&str> = listing["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, expected, "listing: {listing:#}");
}

#[test]
fn list_before_import_is_empty() {
    let cfg = tempdir().unwrap();
    idrive(cfg.path()).arg("init").assert().success();
    let out = idrive(cfg.path()).arg("list").output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["authorized_devices"], 1);
    assert_eq!(v["device_roots_present"], 0);
    assert_eq!(v["files"].as_array().unwrap().len(), 0);
}

#[test]
fn list_before_init_errors() {
    let cfg = tempdir().unwrap();
    idrive(cfg.path()).arg("list").assert().failure();
}

#[test]
fn delete_then_reimport_marks_path_suppressed() {
    let cfg = tempdir().unwrap();
    let work = tempdir().unwrap();
    std::fs::write(work.path().join("keep.txt"), b"k").unwrap();
    std::fs::write(work.path().join("drop.txt"), b"d").unwrap();
    idrive(cfg.path()).arg("init").assert().success();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();

    // Delete one file, re-import, list should hide it and show as suppressed.
    std::fs::remove_file(work.path().join("drop.txt")).unwrap();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();

    let out = idrive(cfg.path()).arg("list").output().unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    let paths: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["keep.txt"]);
    let suppressed: Vec<&str> = v["suppressed_by_tombstone"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert_eq!(suppressed, vec!["drop.txt"]);
}

#[test]
fn deleted_file_can_return_and_tombstone_drops() {
    let cfg = tempdir().unwrap();
    let work = tempdir().unwrap();
    std::fs::write(work.path().join("file.txt"), b"v1").unwrap();
    idrive(cfg.path()).arg("init").assert().success();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();
    std::fs::remove_file(work.path().join("file.txt")).unwrap();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();

    // File comes back.
    std::fs::write(work.path().join("file.txt"), b"v2-back").unwrap();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();

    let out = idrive(cfg.path()).arg("list").output().unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    let paths: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["file.txt"]);
    assert!(v["suppressed_by_tombstone"].as_array().unwrap().is_empty());
}

#[test]
fn restore_after_init_errors_without_force_path() {
    // For now restore refuses to overwrite an existing install.
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    let nsec = std::fs::read_to_string(dir.path().join("owner_key"))
        .unwrap()
        .trim()
        .to_string();
    idrive(dir.path())
        .args(["restore", &nsec])
        .assert()
        .failure();
}

#[derive(Clone)]
struct LocalRelayState {
    events: Arc<Mutex<Vec<serde_json::Value>>>,
}

struct LocalNostrRelay {
    url: String,
    task: tokio::task::JoinHandle<()>,
}

impl LocalNostrRelay {
    async fn spawn() -> Self {
        let state = LocalRelayState {
            events: Arc::new(Mutex::new(Vec::new())),
        };
        let app = Router::new().route("/", get(relay_ws)).with_state(state);
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url: format!("ws://{addr}"),
            task,
        }
    }
}

impl Drop for LocalNostrRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn relay_ws(ws: WebSocketUpgrade, State(state): State<LocalRelayState>) -> Response {
    ws.on_upgrade(move |socket| relay_socket(socket, state))
}

async fn relay_socket(mut socket: WebSocket, state: LocalRelayState) {
    while let Some(Ok(message)) = socket.next().await {
        let WsMessage::Text(text) = message else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        let Some(items) = value.as_array() else {
            continue;
        };
        let Some(command) = items.first().and_then(serde_json::Value::as_str) else {
            continue;
        };
        match command {
            "EVENT" => {
                let Some(event) = items.get(1).cloned() else {
                    continue;
                };
                let event_id = event["id"].as_str().unwrap_or_default().to_string();
                state.events.lock().await.push(event);
                let _ = socket
                    .send(WsMessage::Text(
                        serde_json::json!(["OK", event_id, true, ""]).to_string(),
                    ))
                    .await;
            }
            "REQ" => {
                let Some(subscription_id) = items.get(1).and_then(serde_json::Value::as_str) else {
                    continue;
                };
                let filters = items.iter().skip(2).cloned().collect::<Vec<_>>();
                let events = state.events.lock().await.clone();
                for event in events {
                    if filters
                        .iter()
                        .any(|filter| event_matches_filter(&event, filter))
                    {
                        let _ = socket
                            .send(WsMessage::Text(
                                serde_json::json!(["EVENT", subscription_id, event]).to_string(),
                            ))
                            .await;
                    }
                }
                let _ = socket
                    .send(WsMessage::Text(
                        serde_json::json!(["EOSE", subscription_id]).to_string(),
                    ))
                    .await;
            }
            _ => {}
        }
    }
}

fn event_matches_filter(event: &serde_json::Value, filter: &serde_json::Value) -> bool {
    if let Some(kinds) = filter.get("kinds").and_then(serde_json::Value::as_array) {
        let Some(kind) = event.get("kind").and_then(serde_json::Value::as_u64) else {
            return false;
        };
        if !kinds
            .iter()
            .any(|candidate| candidate.as_u64() == Some(kind))
        {
            return false;
        }
    }
    if let Some(authors) = filter.get("authors").and_then(serde_json::Value::as_array) {
        let Some(author) = event.get("pubkey").and_then(serde_json::Value::as_str) else {
            return false;
        };
        if !authors
            .iter()
            .any(|candidate| candidate.as_str() == Some(author))
        {
            return false;
        }
    }
    if let Some(d_values) = filter.get("#d").and_then(serde_json::Value::as_array) {
        let Some(tags) = event.get("tags").and_then(serde_json::Value::as_array) else {
            return false;
        };
        let has_matching_d_tag = tags.iter().any(|tag| {
            let Some(tag_items) = tag.as_array() else {
                return false;
            };
            tag_items.first().and_then(serde_json::Value::as_str) == Some("d")
                && tag_items
                    .get(1)
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|value| {
                        d_values
                            .iter()
                            .any(|candidate| candidate.as_str() == Some(value))
                    })
        });
        if !has_matching_d_tag {
            return false;
        }
    }
    true
}

#[derive(Clone)]
struct LocalBlossomState {
    blobs: Arc<Mutex<HashMap<String, Vec<u8>>>>,
}

struct LocalBlossomServer {
    url: String,
    state: LocalBlossomState,
    task: tokio::task::JoinHandle<()>,
}

impl LocalBlossomServer {
    async fn spawn() -> Self {
        let state = LocalBlossomState {
            blobs: Arc::new(Mutex::new(HashMap::new())),
        };
        let app = Router::new()
            .route("/upload", put(blossom_upload))
            .route("/:name", get(blossom_get).head(blossom_head))
            .with_state(state.clone());
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url: format!("http://{addr}"),
            state,
            task,
        }
    }

    async fn blob_count(&self) -> usize {
        self.state.blobs.lock().await.len()
    }
}

impl Drop for LocalBlossomServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn blossom_upload(
    State(state): State<LocalBlossomState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let hash = to_hex(&sha256(&body));
    if let Some(expected) = headers
        .get("x-sha-256")
        .and_then(|value| value.to_str().ok())
        && expected != hash
    {
        return text_response(StatusCode::BAD_REQUEST, "hash mismatch");
    }
    let mut blobs = state.blobs.lock().await;
    if blobs.contains_key(&hash) {
        return text_response(StatusCode::CONFLICT, "already exists");
    }
    blobs.insert(hash, body.to_vec());
    text_response(StatusCode::CREATED, "created")
}

async fn blossom_get(
    State(state): State<LocalBlossomState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    let Some(hash) = name.strip_suffix(".bin") else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };
    let Some(bytes) = state.blobs.lock().await.get(hash).cloned() else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };
    blob_response(StatusCode::OK, bytes)
}

async fn blossom_head(
    State(state): State<LocalBlossomState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    let Some(hash) = name.strip_suffix(".bin") else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };
    let Some(size) = state.blobs.lock().await.get(hash).map(Vec::len) else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, size.to_string())
        .body(Body::empty())
        .unwrap()
}

fn blob_response(status: StatusCode, bytes: Vec<u8>) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .body(Body::from(bytes))
        .unwrap()
}

fn text_response(status: StatusCode, text: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(text.to_string()))
        .unwrap()
}
