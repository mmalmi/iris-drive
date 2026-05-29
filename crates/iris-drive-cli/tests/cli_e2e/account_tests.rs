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
    assert!(!dir.path().join("owner_key").exists());
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
fn restore_uses_provided_device_nsec_and_grants_admin_authority() {
    // Capture original account/device npub from an init.
    let dir_a = tempdir().unwrap();
    idrive(dir_a.path()).arg("init").assert().success();
    // Read the persisted device nsec from disk to drive `restore`.
    let nsec = std::fs::read_to_string(dir_a.path().join("key"))
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
    assert_eq!(v["device_npub"], original_v["device_npub"]);
    assert!(!dir_b.path().join("owner_key").exists());
}

#[test]
fn macos_login_restore_can_replace_existing_local_setup_with_force() {
    let original_dir = tempdir().unwrap();
    idrive(original_dir.path()).arg("init").assert().success();
    let nsec = std::fs::read_to_string(original_dir.path().join("key"))
        .unwrap()
        .trim()
        .to_string();
    let original = run_json(original_dir.path(), &["whoami"]);
    let original_device = original["device_npub"].as_str().unwrap();

    let macos_dir = tempdir().unwrap();
    let stale = run_json(
        macos_dir.path(),
        &["init", "--label", "stale macOS profile"],
    );
    assert_ne!(stale["device_npub"].as_str(), Some(original_device));

    idrive(macos_dir.path())
        .args(["restore", &nsec])
        .assert()
        .failure()
        .stderr(contains("already initialized"));

    let restored = run_json(macos_dir.path(), &["restore", &nsec, "--force"]);
    assert_eq!(restored["device_npub"].as_str(), Some(original_device));
    assert_eq!(restored["has_owner_signing_authority"], true);
    assert_eq!(restored["authorization_state"], "authorized");
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
    assert!(
        v["device_link_request"]["url"]
            .as_str()
            .unwrap()
            .contains("device=npub1")
    );
    assert!(
        v["device_link_request"]["url"]
            .as_str()
            .unwrap()
            .contains("secret=")
    );
    assert!(dir.path().join("key").exists());
    assert!(!dir.path().join("owner_key").exists()); // never on a linked device
}

#[test]
fn macos_login_link_can_replace_existing_local_setup_with_force() {
    let owner_dir = tempdir().unwrap();
    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let owner_npub = owner["owner_npub"].as_str().unwrap();

    let macos_dir = tempdir().unwrap();
    run_json(
        macos_dir.path(),
        &["init", "--label", "stale macOS profile"],
    );

    idrive(macos_dir.path())
        .args(["link", owner_npub])
        .assert()
        .failure()
        .stderr(contains("already initialized"));

    let linked = run_json(macos_dir.path(), &["link", owner_npub, "--force"]);
    assert_eq!(linked["owner_npub"].as_str(), Some(owner_npub));
    assert_eq!(linked["has_owner_signing_authority"], false);
    assert_eq!(linked["authorization_state"], "awaiting_approval");
    assert!(linked["device_link_request"]["url"].as_str().is_some());
}

#[test]
fn owner_invite_link_queues_fips_request_to_admin_device() {
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();

    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let invite_url = owner["device_link_invite"]["url"].as_str().unwrap();
    assert!(invite_url.contains("secret="));
    let owner_npub = owner["owner_npub"].as_str().unwrap();
    let admin_device_npub = owner["device_npub"].as_str().unwrap();

    let linked = run_json(linked_dir.path(), &["link", invite_url, "--label", "phone"]);

    assert_eq!(linked["owner_npub"], owner_npub);
    assert_eq!(
        linked["device_link_request"]["admin_device_npub"],
        admin_device_npub
    );
    assert!(
        linked["device_link_request"]["requested_at"]
            .as_u64()
            .is_some()
    );

    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let owner_state = owner_config.account.as_ref().unwrap();
    let config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    let state = config.account.as_ref().unwrap();
    assert_eq!(
        state
            .outbound_device_link_request
            .as_ref()
            .unwrap()
            .admin_device_pubkey
            .as_str(),
        owner_state.device_pubkey.as_str()
    );
    assert_eq!(
        state
            .outbound_device_link_request
            .as_ref()
            .unwrap()
            .link_secret
            .as_str(),
        owner_state.device_link_secret.as_str()
    );
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
fn devices_can_appoint_and_demote_admin() {
    let owner_dir = tempdir().unwrap();
    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let owner_npub = owner["owner_npub"].as_str().unwrap();

    let linked_dir = tempdir().unwrap();
    let linked = run_json(
        linked_dir.path(),
        &["link", owner_npub, "--label", "laptop"],
    );
    let linked_device = linked["device_npub"].as_str().unwrap();
    run_json(owner_dir.path(), &["approve", linked_device]);

    let promoted = run_json(
        owner_dir.path(),
        &["devices", "appoint-admin", linked_device],
    );
    assert_eq!(promoted["role"], "admin");
    let status = run_json(owner_dir.path(), &["status"]);
    assert!(status["peers"].as_array().unwrap().iter().any(|device| {
        device["device_npub"].as_str() == Some(linked_device)
            && device["role"].as_str() == Some("admin")
    }));
    let roster = run_json(owner_dir.path(), &["devices", "list"]);
    assert!(
        roster["app_keys"]["devices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|device| device["npub"].as_str() == Some(linked_device)
                && device["role"].as_str() == Some("admin"))
    );

    let demoted = run_json(
        owner_dir.path(),
        &["devices", "demote-admin", linked_device],
    );
    assert_eq!(demoted["role"], "member");
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
fn devices_group_covers_invite_request_approve_and_list_flow() {
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();

    run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let invite = run_json(owner_dir.path(), &["devices", "invite"]);
    let invite_url = invite["url"].as_str().unwrap();
    assert!(invite_url.starts_with("iris-drive://link-device?"));
    assert!(invite_url.contains("secret="));

    let linked = run_json(
        linked_dir.path(),
        &["devices", "request", invite_url, "--label", "laptop"],
    );
    assert_eq!(linked["authorization_state"], "awaiting_approval");
    let request_url = linked["device_link_request"]["url"].as_str().unwrap();
    assert!(request_url.starts_with("iris-drive://device-link?"));
    assert!(request_url.contains("device=npub1"));
    assert!(request_url.contains("secret="));
    assert_eq!(
        linked["device_link_request"]["sent_over_fips"],
        serde_json::Value::Bool(true)
    );

    let requests = run_json(linked_dir.path(), &["devices", "requests"]);
    assert!(requests["outbound"].is_object());
    assert!(requests["inbound"].as_array().unwrap().is_empty());

    let approved = run_json(owner_dir.path(), &["devices", "approve", request_url]);
    assert_eq!(approved["roster_size"], 2);

    let devices = run_json(owner_dir.path(), &["devices", "list"]);
    assert_eq!(devices["app_keys"]["devices"].as_array().unwrap().len(), 2);
}

#[test]
fn devices_request_manual_owner_and_admin_device_queues_fips_request() {
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();

    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let owner_npub = owner["owner_npub"].as_str().unwrap();
    let admin_device_npub = owner["device_npub"].as_str().unwrap();

    let linked = run_json(
        linked_dir.path(),
        &[
            "devices",
            "request",
            owner_npub,
            "--admin-device",
            admin_device_npub,
            "--label",
            "manual laptop",
        ],
    );

    assert_eq!(linked["authorization_state"], "awaiting_approval");
    assert_eq!(
        linked["device_link_request"]["admin_device_npub"].as_str(),
        Some(admin_device_npub)
    );
    assert_eq!(
        linked["device_link_request"]["sent_over_fips"],
        serde_json::Value::Bool(true)
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
