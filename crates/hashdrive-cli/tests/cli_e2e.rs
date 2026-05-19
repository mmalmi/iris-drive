//! End-to-end tests for the `hdrive` CLI.
//!
//! These exercise the actual compiled binary against a temp config dir,
//! so they catch arg-parsing, exit-code, and IO surprises. No mocks.

use assert_cmd::Command;
use predicates::str::contains;
use tempfile::tempdir;

fn hdrive(dir: &std::path::Path) -> Command {
    let mut cmd = Command::cargo_bin("hdrive").unwrap();
    cmd.env("HASHDRIVE_CONFIG_DIR", dir);
    cmd
}

#[test]
fn init_creates_key_and_config() {
    let dir = tempdir().unwrap();
    hdrive(dir.path())
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
    hdrive(dir.path()).arg("init").assert().success();
    hdrive(dir.path()).arg("init").assert().failure();
}

#[test]
fn double_init_with_force_succeeds() {
    let dir = tempdir().unwrap();
    hdrive(dir.path()).arg("init").assert().success();
    hdrive(dir.path())
        .args(["init", "--force"])
        .assert()
        .success()
        .stdout(contains("npub1"));
}

#[test]
fn whoami_after_init_prints_npub() {
    let dir = tempdir().unwrap();
    hdrive(dir.path()).arg("init").assert().success();
    hdrive(dir.path())
        .arg("whoami")
        .assert()
        .success()
        .stdout(predicates::str::starts_with("npub1"));
}

#[test]
fn whoami_before_init_errors() {
    let dir = tempdir().unwrap();
    hdrive(dir.path()).arg("whoami").assert().failure();
}

#[test]
fn status_before_init_reports_uninitialized() {
    let dir = tempdir().unwrap();
    let out = hdrive(dir.path()).arg("status").output().unwrap();
    assert!(out.status.success());
    let body = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["initialized"], false);
    assert!(v["drives"].as_array().unwrap().is_empty());
}

#[test]
fn status_after_init_reports_initialized() {
    let dir = tempdir().unwrap();
    hdrive(dir.path()).arg("init").assert().success();
    let out = hdrive(dir.path()).arg("status").output().unwrap();
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
    hdrive(dir.path()).arg("init").assert().success();
    hdrive(dir.path())
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
    hdrive(dir.path())
        .arg("drives")
        .assert()
        .success()
        .stdout(contains("hdrive init"));
}

#[test]
fn npub_is_stable_across_invocations() {
    let dir = tempdir().unwrap();
    hdrive(dir.path()).arg("init").assert().success();
    let one = String::from_utf8(hdrive(dir.path()).arg("whoami").output().unwrap().stdout).unwrap();
    let two = String::from_utf8(hdrive(dir.path()).arg("whoami").output().unwrap().stdout).unwrap();
    assert_eq!(one, two);
}
