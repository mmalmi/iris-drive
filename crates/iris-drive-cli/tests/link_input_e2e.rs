//! End-to-end tests for shared link-input classification and validation.

use assert_cmd::Command;
use std::process::Output;
use tempfile::tempdir;

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
fn link_input_classify_uses_app_core_completion() {
    let dir = tempdir().unwrap();

    let short = run_json(dir.path(), &["link-input", "classify", "npub1short"]);
    assert_eq!(short["kind"], "owner_pubkey");
    assert_eq!(short["is_complete"], false);

    let owner = run_json(dir.path(), &["init", "--force", "--label", "CLI owner"]);
    let app_key_npub = owner["current_app_key_npub"].as_str().unwrap();
    let complete = run_json(dir.path(), &["link-input", "classify", app_key_npub]);
    assert_eq!(complete["kind"], "owner_pubkey");
    assert_eq!(complete["is_complete"], true);
    assert_eq!(complete["normalized_input"], app_key_npub);
}

#[test]
fn link_input_validate_uses_app_core_completion() {
    let dir = tempdir().unwrap();

    let short = run_json(dir.path(), &["link-input", "validate", "npub1short"]);
    assert_eq!(short["kind"], "owner_pubkey");
    assert_eq!(short["is_complete"], false);

    let owner = run_json(dir.path(), &["init", "--force", "--label", "CLI owner"]);
    let app_key_npub = owner["current_app_key_npub"].as_str().unwrap();
    let complete = run_json(dir.path(), &["link-input", "validate", app_key_npub]);
    assert_eq!(complete["kind"], "owner_pubkey");
    assert_eq!(complete["is_complete"], true);
    assert_eq!(complete["normalized_input"], app_key_npub);
}
