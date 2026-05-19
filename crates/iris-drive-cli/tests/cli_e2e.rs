//! End-to-end tests for the `idrive` CLI.
//!
//! These exercise the actual compiled binary against a temp config dir,
//! so they catch arg-parsing, exit-code, and IO surprises. No mocks.

use assert_cmd::Command;
use hashtree_core::Cid;
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
    assert!(v["peers"].as_array().unwrap().is_empty());
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
    assert_eq!(v["network"]["authorized_device_count"], 1);
    assert_eq!(v["network"]["published_device_roots"], 0);
    let peers = v["peers"].as_array().unwrap();
    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0]["is_current_device"], true);
    assert_eq!(peers[0]["authorized"], true);
    assert_eq!(peers[0]["has_root"], false);
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
    assert_eq!(
        v["drives"][0]["working_dir"],
        work.path().display().to_string()
    );
    assert_eq!(v["hashtree"]["current_root_cid"], root_cid);
    assert_eq!(v["hashtree"]["current_root_private"], true);
    assert!(
        v["hashtree"]["files_iris_to_url"]
            .as_str()
            .unwrap()
            .starts_with("https://files.iris.to/#/nhash1")
    );
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
