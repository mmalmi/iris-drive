//! End-to-end tests for the `idrive` CLI.
//!
//! These exercise the actual compiled binary against a temp config dir,
//! so they catch arg-parsing, exit-code, and IO surprises. No mocks.

use assert_cmd::Command;
use predicates::str::contains;
use tempfile::tempdir;

fn idrive(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("idrive").unwrap();
    cmd.env("IRIS_DRIVE_CONFIG_DIR", dir);
    cmd
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
    assert!(dir.path().join("config.toml").exists());
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
fn whoami_after_init_prints_npub() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    idrive(dir.path())
        .arg("whoami")
        .assert()
        .success()
        .stdout(predicates::str::starts_with("npub1"));
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
}

#[test]
fn status_after_init_reports_initialized() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    let out = idrive(dir.path()).arg("status").output().unwrap();
    let v: serde_json::Value = serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["initialized"], true);
    assert!(v["pubkey_npub"].as_str().unwrap().starts_with("npub1"));
    let drives = v["drives"].as_array().unwrap();
    assert_eq!(drives.len(), 1);
    assert_eq!(drives[0]["drive_id"], "main");
    assert_eq!(drives[0]["role"], "owner");
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
    assert!(v["root_cid"].as_str().unwrap().len() > 10);
}

#[test]
fn import_persists_to_blocks_dir_and_advances_root() {
    let work = tempdir().unwrap();
    std::fs::write(work.path().join("hello.txt"), b"hi there").unwrap();

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
