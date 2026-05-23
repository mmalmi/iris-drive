#[allow(clippy::wildcard_imports)]
use super::*;

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
