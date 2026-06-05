#[allow(clippy::wildcard_imports)]
use super::*;

fn current_app_key_npub(value: &serde_json::Value) -> &str {
    value["current_app_key_npub"].as_str().unwrap()
}

fn device_link_invite_url(value: &serde_json::Value) -> &str {
    value["device_link_invite"]["url"].as_str().unwrap()
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
    assert_eq!(v["can_admin_profile"], true);
    assert_eq!(v["can_write_roots"], true);
    assert_eq!(v["authorization_state"], "authorized");
    assert!(current_app_key_npub(&v).starts_with("npub1"));
    assert!(v["profile_id"].as_str().unwrap().contains('-'));
    assert_eq!(v["profile"]["profile_id"], v["profile_id"]);
    assert_eq!(
        v["profile"]["current_app_key_npub"],
        v["current_app_key_npub"]
    );
    assert_eq!(v["profile"]["can_admin_profile"], true);
    assert_eq!(v["profile"]["can_write_roots"], true);
    assert_eq!(v["profile"]["active_app_key_count"], 1);
    assert_eq!(v["profile"]["current_key_epoch"], 1);
    assert_eq!(v["profile"]["recovery_phrase_facet_count"], 1);
}

#[test]
fn logout_removes_local_account_and_key_material() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    assert!(dir.path().join("key").exists());

    let out = idrive(dir.path()).arg("logout").output().unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["logged_out"], true);
    assert_eq!(v["removed_key"], true);

    assert!(!dir.path().join("key").exists());
    assert!(!dir.path().join("owner_key").exists());

    let config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    assert!(config.profile.is_none());
    assert!(config.user_profile.is_none());
    assert!(config.drives.is_empty());

    let status = run_json(dir.path(), &["status"]);
    assert_eq!(status["initialized"], false);
    assert!(status["drives"].as_array().unwrap().is_empty());

    idrive(dir.path()).arg("whoami").assert().failure();
    idrive(dir.path()).arg("logout").assert().success();
}

#[test]
fn recovery_phrase_restore_keeps_profile_and_uses_fresh_app_key() {
    let dir_a = tempdir().unwrap();
    idrive(dir_a.path()).arg("init").assert().success();
    let recovery_phrase = std::fs::read_to_string(dir_a.path().join("recovery_phrase"))
        .unwrap()
        .trim()
        .to_string();
    let original_owner =
        String::from_utf8(idrive(dir_a.path()).arg("whoami").output().unwrap().stdout).unwrap();
    let original_v: serde_json::Value = serde_json::from_str(&original_owner).unwrap();
    let original_profile_id = original_v["profile_id"].as_str().unwrap().to_string();
    let original_app_key_npub = current_app_key_npub(&original_v).to_string();

    let dir_b = tempdir().unwrap();
    let out = idrive(dir_b.path())
        .args(["restore", &recovery_phrase])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["profile_id"], original_profile_id);
    assert_eq!(v["can_admin_profile"], true);
    assert_ne!(v["current_app_key_npub"], original_app_key_npub);
    assert!(!dir_b.path().join("owner_key").exists());
    assert!(dir_b.path().join("recovery_phrase").exists());
}

#[test]
fn raw_nsec_restore_creates_fresh_profile_and_uses_fresh_app_key() {
    let dir_a = tempdir().unwrap();
    idrive(dir_a.path()).arg("init").assert().success();
    let nsec = std::fs::read_to_string(dir_a.path().join("key"))
        .unwrap()
        .trim()
        .to_string();
    let original = run_json(dir_a.path(), &["whoami"]);
    let original_app_key_npub = current_app_key_npub(&original).to_string();

    let dir_b = tempdir().unwrap();
    let restored = run_json(dir_b.path(), &["restore", &nsec]);
    let dir_c = tempdir().unwrap();
    let restored_again = run_json(dir_c.path(), &["restore", &nsec]);

    assert_ne!(restored["profile_id"], restored_again["profile_id"]);
    assert_ne!(restored["profile_id"], original["profile_id"]);
    assert_ne!(
        restored["current_app_key_npub"].as_str(),
        Some(original_app_key_npub.as_str())
    );
    assert_eq!(restored["can_admin_profile"], true);
    assert_eq!(restored["authorization_state"], "authorized");
    assert!(!dir_b.path().join("owner_key").exists());
    assert!(!dir_b.path().join("recovery_phrase").exists());
}

#[test]
fn macos_login_restore_can_replace_existing_local_setup_with_force() {
    let original_dir = tempdir().unwrap();
    idrive(original_dir.path()).arg("init").assert().success();
    let recovery_phrase = std::fs::read_to_string(original_dir.path().join("recovery_phrase"))
        .unwrap()
        .trim()
        .to_string();
    let original = run_json(original_dir.path(), &["whoami"]);
    let original_profile_id = original["profile_id"].as_str().unwrap();
    let original_app_key = current_app_key_npub(&original);

    let macos_dir = tempdir().unwrap();
    let stale = run_json(
        macos_dir.path(),
        &["init", "--label", "stale macOS profile"],
    );
    assert_ne!(
        stale["current_app_key_npub"].as_str(),
        Some(original_app_key)
    );

    idrive(macos_dir.path())
        .args(["restore", &recovery_phrase])
        .assert()
        .failure()
        .stderr(contains("already initialized"));

    let restored = run_json(macos_dir.path(), &["restore", &recovery_phrase, "--force"]);
    assert_eq!(restored["profile_id"].as_str(), Some(original_profile_id));
    assert_ne!(
        restored["current_app_key_npub"].as_str(),
        Some(original_app_key)
    );
    assert_eq!(restored["can_admin_profile"], true);
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
    let invite_url = device_link_invite_url(&init_v).to_string();

    let out = idrive(dir.path())
        .args(["link", &invite_url])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["can_admin_profile"], false);
    assert_eq!(v["authorization_state"], "awaiting_approval");
    assert_eq!(v["profile_id"], init_v["profile_id"]);
    assert_eq!(
        v["device_link_invite"]["profile_id"],
        serde_json::Value::Null
    );
    assert_eq!(v["device_link_request"]["profile_id"], init_v["profile_id"]);
    assert!(v["device_link_request"].get("owner_npub").is_none());
    assert_eq!(
        v["device_link_request"]["app_key_npub"],
        v["current_app_key_npub"]
    );
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
    let status = run_json(dir.path(), &["status"]);
    let peers = status["peers"].as_array().unwrap();
    assert!(
        peers.is_empty(),
        "pending AppKeys should not appear in roster"
    );
    assert_eq!(status["network"]["authorized_device_count"], 0);
    assert!(dir.path().join("key").exists());
    assert!(!dir.path().join("owner_key").exists()); // never on a linked device
}

#[test]
fn macos_login_link_can_replace_existing_local_setup_with_force() {
    let owner_dir = tempdir().unwrap();
    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let owner_invite = device_link_invite_url(&owner);

    let macos_dir = tempdir().unwrap();
    run_json(
        macos_dir.path(),
        &["init", "--label", "stale macOS profile"],
    );

    idrive(macos_dir.path())
        .args(["link", owner_invite])
        .assert()
        .failure()
        .stderr(contains("already initialized"));

    let linked = run_json(macos_dir.path(), &["link", owner_invite, "--force"]);
    assert_eq!(linked["can_admin_profile"], false);
    assert_eq!(linked["authorization_state"], "awaiting_approval");
    assert!(linked["device_link_request"]["url"].as_str().is_some());
}

#[test]
fn owner_invite_link_queues_fips_request_to_admin_device() {
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();

    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let invite_url = owner["device_link_invite"]["url"].as_str().unwrap();
    assert!(invite_url.starts_with("iris-drive://invite/"));
    let admin_app_key_npub = current_app_key_npub(&owner);

    let linked = run_json(linked_dir.path(), &["link", invite_url, "--label", "phone"]);

    assert!(linked["device_link_request"].get("owner_npub").is_none());
    assert_eq!(linked["profile_id"], owner["profile_id"]);
    assert_eq!(
        linked["device_link_invite"]["profile_id"],
        serde_json::Value::Null
    );
    assert_eq!(
        linked["device_link_request"]["profile_id"],
        owner["profile_id"]
    );
    assert_eq!(
        linked["device_link_request"]["admin_app_key_npub"],
        admin_app_key_npub
    );
    assert!(
        linked["device_link_request"]["requested_at"]
            .as_u64()
            .is_some()
    );

    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let owner_state = owner_config.profile.as_ref().unwrap();
    let config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    let state = config.profile.as_ref().unwrap();
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
fn link_then_approve_authorizes_the_linked_app_key() {
    // Set up owner-capable install + a separate linked install.
    let owner_dir = tempdir().unwrap();
    idrive(owner_dir.path()).arg("init").assert().success();
    let owner = serde_json::from_str::<serde_json::Value>(
        &String::from_utf8(
            idrive(owner_dir.path())
                .arg("whoami")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap();
    let owner_invite = device_link_invite_url(&owner).to_string();

    let linked_dir = tempdir().unwrap();
    let linked_v: serde_json::Value = serde_json::from_str(
        &String::from_utf8(
            idrive(linked_dir.path())
                .args(["link", &owner_invite])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap();
    let linked_app_key_npub = current_app_key_npub(&linked_v).to_string();

    // Owner approves the linked device.
    let approve = idrive(owner_dir.path())
        .args(["approve", &linked_app_key_npub])
        .output()
        .unwrap();
    assert!(approve.status.success(), "{approve:?}");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(approve.stdout).unwrap()).unwrap();
    assert_eq!(v["roster_size"], 2);

    // Roster on the owner side now has 2 AppKey actors.
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
    assert_eq!(
        roster["app_keys"]["app_actors"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn app_keys_can_appoint_and_demote_admin() {
    let owner_dir = tempdir().unwrap();
    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let owner_invite = device_link_invite_url(&owner);

    let linked_dir = tempdir().unwrap();
    let linked = run_json(
        linked_dir.path(),
        &["link", owner_invite, "--label", "laptop"],
    );
    let linked_app_key = current_app_key_npub(&linked);
    run_json(owner_dir.path(), &["approve", linked_app_key]);

    let promoted = run_json(
        owner_dir.path(),
        &["app-keys", "appoint-admin", linked_app_key],
    );
    assert_eq!(promoted["app_key_npub"], linked_app_key);
    assert_eq!(promoted["role"], "admin");
    let status = run_json(owner_dir.path(), &["status"]);
    assert!(status["peers"].as_array().unwrap().iter().any(|device| {
        device["device_npub"].as_str() == Some(linked_app_key)
            && device["role"].as_str() == Some("admin")
    }));
    let roster = run_json(owner_dir.path(), &["app-keys", "list"]);
    assert!(
        roster["app_keys"]["app_actors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|device| device["npub"].as_str() == Some(linked_app_key)
                && device["role"].as_str() == Some("admin"))
    );

    let demoted = run_json(
        owner_dir.path(),
        &["app-keys", "demote-admin", linked_app_key],
    );
    assert_eq!(demoted["role"], "member");
}

#[test]
fn owner_approves_device_request_link() {
    let owner_dir = tempdir().unwrap();
    let other_owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();

    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let owner_invite = device_link_invite_url(&owner).to_string();
    run_json(other_owner_dir.path(), &["init", "--label", "other-admin"]);

    let linked = run_json(
        linked_dir.path(),
        &["link", &owner_invite, "--label", "windows-peer"],
    );
    let request_url = linked["device_link_request"]["url"].as_str().unwrap();

    idrive(other_owner_dir.path())
        .args(["approve", request_url])
        .assert()
        .failure()
        .stderr(contains("different profile"));

    let approved = run_json(owner_dir.path(), &["approve", request_url]);
    assert_eq!(approved["roster_size"], 2);

    let roster = run_json(owner_dir.path(), &["roster"]);
    let app_actors = roster["app_keys"]["app_actors"].as_array().unwrap();
    assert!(
        app_actors
            .iter()
            .any(|device| device["label"].as_str() == Some("windows-peer"))
    );
}

#[test]
fn owner_rejects_device_request_link() {
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();

    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let owner_invite = device_link_invite_url(&owner);
    let linked = run_json(
        linked_dir.path(),
        &["link", owner_invite, "--label", "rejected-phone"],
    );
    let request_url = linked["device_link_request"]["url"].as_str().unwrap();
    let linked_app_key = current_app_key_npub(&linked);

    {
        let config_path = iris_drive_core::paths::config_path_in(owner_dir.path());
        let mut config = iris_drive_core::AppConfig::load_or_default(&config_path).unwrap();
        let state = config.profile.as_mut().unwrap();
        let profile_id = state.profile_id;
        let link_secret = state.device_link_secret.clone();
        let linked_hex = iris_drive_core::AppConfig::load_or_default(
            iris_drive_core::paths::config_path_in(linked_dir.path()),
        )
        .unwrap()
        .profile
        .unwrap()
        .device_pubkey;
        state
            .record_inbound_device_link_request(
                profile_id,
                &linked_hex,
                Some("rejected-phone".into()),
                &link_secret,
                42,
            )
            .unwrap();
        config.save(&config_path).unwrap();
    }

    let rejected = run_json(owner_dir.path(), &["app-keys", "reject", request_url]);
    assert_eq!(rejected["rejected"], true);
    assert!(
        rejected["inbound_device_link_requests"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let roster = run_json(owner_dir.path(), &["app-keys", "list"]);
    assert!(
        roster["app_keys"]["app_actors"]
            .as_array()
            .unwrap()
            .iter()
            .all(|device| device["npub"].as_str() != Some(linked_app_key))
    );
}

#[test]
fn app_keys_group_covers_invite_request_approve_and_list_flow() {
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();

    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let admin_app_key_npub = current_app_key_npub(&owner);
    let invite = run_json(owner_dir.path(), &["app-keys", "invite"]);
    let invite_url = invite["url"].as_str().unwrap();
    assert!(invite_url.starts_with("iris-drive://invite/"));
    assert!(!invite_url.contains("local-owner"));
    assert!(!invite_url.contains("device-"));
    assert!(invite.get("owner_npub").is_none());
    assert_eq!(
        invite["admin_app_key_npub"].as_str(),
        Some(admin_app_key_npub)
    );

    let linked = run_json(
        linked_dir.path(),
        &["app-keys", "request", invite_url, "--label", "laptop"],
    );
    assert_eq!(linked["authorization_state"], "awaiting_approval");
    let request_url = linked["device_link_request"]["url"].as_str().unwrap();
    assert!(request_url.starts_with("iris-drive://device-link?"));
    assert!(!request_url.contains("owner="));
    assert!(request_url.contains("device=npub1"));
    assert!(request_url.contains("secret="));
    assert!(!request_url.contains("local-owner"));
    assert!(!request_url.contains("device=device-"));
    assert_eq!(
        linked["device_link_request"]["admin_app_key_npub"].as_str(),
        Some(admin_app_key_npub)
    );
    assert_eq!(
        linked["device_link_request"]["sent_over_fips"],
        serde_json::Value::Bool(true)
    );

    let requests = run_json(linked_dir.path(), &["app-keys", "requests"]);
    assert!(requests["outbound"].is_object());
    assert!(requests["inbound"].as_array().unwrap().is_empty());

    let approved = run_json(owner_dir.path(), &["app-keys", "approve", request_url]);
    assert_eq!(
        approved["approved_app_key_npub"],
        linked["current_app_key_npub"]
    );
    assert_eq!(approved["roster_size"], 2);

    let devices = run_json(owner_dir.path(), &["app-keys", "list"]);
    assert_eq!(
        devices["app_keys"]["app_actors"].as_array().unwrap().len(),
        2
    );
}

#[test]
fn app_keys_repair_wraps_reports_noop_when_epoch_is_complete() {
    let owner_dir = tempdir().unwrap();
    run_json(owner_dir.path(), &["init", "--label", "admin"]);

    let repaired = run_json(owner_dir.path(), &["app-keys", "repair-wraps"]);

    assert_eq!(repaired["repaired_key_wrap_count"], 0);
    assert!(
        repaired["repaired_key_wraps"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        repaired["remaining_missing_key_wraps"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn app_keys_reset_invite_rotates_secret_and_clears_inbound_requests() {
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();

    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let old_invite = owner["device_link_invite"]["url"].as_str().unwrap();
    let linked = run_json(
        linked_dir.path(),
        &["app-keys", "request", old_invite, "--label", "phone"],
    );
    let linked_config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    let linked_app_key = linked_config
        .profile
        .as_ref()
        .unwrap()
        .device_pubkey
        .clone();

    let config_path = config_path_in(owner_dir.path());
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    let state = config.profile.as_mut().unwrap();
    let profile_id = state.profile_id;
    let old_secret = state.device_link_secret.clone();
    state
        .record_inbound_device_link_request(
            profile_id,
            &linked_app_key,
            Some("phone".to_string()),
            &old_secret,
            linked["device_link_request"]["requested_at"]
                .as_u64()
                .unwrap(),
        )
        .unwrap();
    config.save(&config_path).unwrap();
    let requests = run_json(owner_dir.path(), &["app-keys", "requests"]);
    assert_eq!(requests["inbound"].as_array().unwrap().len(), 1);

    let reset = run_json(owner_dir.path(), &["app-keys", "reset-invite"]);
    let new_invite = reset["device_link_invite"]["url"].as_str().unwrap();
    assert_ne!(new_invite, old_invite);
    assert!(
        reset["inbound_device_link_requests"]
            .as_array()
            .unwrap()
            .is_empty()
    );

    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    let state = config.profile.as_mut().unwrap();
    assert_ne!(state.device_link_secret, old_secret);
    assert!(
        !state
            .record_inbound_device_link_request(
                profile_id,
                &linked_app_key,
                Some("phone".to_string()),
                &old_secret,
                999,
            )
            .unwrap()
    );

    let invite = run_json(owner_dir.path(), &["app-keys", "invite"]);
    assert_eq!(invite["url"].as_str(), Some(new_invite));
}

#[test]
fn app_keys_request_manual_profile_and_admin_app_key_queues_fips_request() {
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();

    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let admin_app_key_npub = current_app_key_npub(&owner);
    let profile_id = owner["profile_id"].as_str().unwrap();

    let linked = run_json(
        linked_dir.path(),
        &[
            "app-keys",
            "request",
            profile_id,
            "--admin-app-key",
            admin_app_key_npub,
            "--label",
            "manual laptop",
        ],
    );

    assert_eq!(linked["authorization_state"], "awaiting_approval");
    assert_eq!(
        linked["device_link_request"]["admin_app_key_npub"].as_str(),
        Some(admin_app_key_npub)
    );
    assert_eq!(
        linked["device_link_request"]["profile_id"].as_str(),
        Some(profile_id)
    );
    assert_eq!(
        linked["device_link_request"]["sent_over_fips"],
        serde_json::Value::Bool(true)
    );
}

#[test]
fn app_keys_request_rejects_manual_app_key_without_profile_id() {
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();

    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let admin_app_key_npub = current_app_key_npub(&owner);

    idrive(linked_dir.path())
        .args([
            "app-keys",
            "request",
            admin_app_key_npub,
            "--admin-app-key",
            admin_app_key_npub,
        ])
        .assert()
        .failure()
        .stderr(contains(
            "manual AppKey linking requires an IrisProfile UUID",
        ));
}

#[test]
fn owner_can_revoke_a_linked_app_key() {
    let owner_dir = tempdir().unwrap();
    idrive(owner_dir.path()).arg("init").assert().success();
    let owner = serde_json::from_str::<serde_json::Value>(
        &String::from_utf8(
            idrive(owner_dir.path())
                .arg("whoami")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap();
    let owner_invite = device_link_invite_url(&owner).to_string();

    let linked_dir = tempdir().unwrap();
    let linked_v: serde_json::Value = serde_json::from_str(
        &String::from_utf8(
            idrive(linked_dir.path())
                .args(["link", &owner_invite])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap();
    let linked_app_key_npub = current_app_key_npub(&linked_v).to_string();

    idrive(owner_dir.path())
        .args(["approve", &linked_app_key_npub])
        .assert()
        .success();
    let revoked = run_json(owner_dir.path(), &["revoke", &linked_app_key_npub]);
    assert_eq!(revoked["revoked_app_key_npub"], linked_app_key_npub);
    assert_eq!(revoked["roster_size"], 1);
    assert!(revoked["dck_generation"].as_u64().unwrap() > 1);

    let roster = run_json(owner_dir.path(), &["roster"]);
    let app_actors = roster["app_keys"]["app_actors"].as_array().unwrap();
    assert_eq!(app_actors.len(), 1);
    assert_ne!(app_actors[0]["npub"], linked_app_key_npub);
}

#[test]
fn approve_without_admin_authority_errors() {
    // Linked-only AppKey tries to approve.
    let owner_dir = tempdir().unwrap();
    idrive(owner_dir.path()).arg("init").assert().success();
    let owner = serde_json::from_str::<serde_json::Value>(
        &String::from_utf8(
            idrive(owner_dir.path())
                .arg("whoami")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap();
    let owner_invite = device_link_invite_url(&owner).to_string();
    let linked_dir = tempdir().unwrap();
    idrive(linked_dir.path())
        .args(["link", &owner_invite])
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
    assert_eq!(v["profile"]["active_app_key_count"], 1);
    assert_eq!(v["profile"]["current_key_epoch"], 1);
    assert_eq!(
        v["profile"]["missing_key_wraps"].as_array().unwrap().len(),
        0
    );
    let app_actors = v["app_keys"]["app_actors"].as_array().unwrap();
    assert_eq!(app_actors.len(), 1);
    assert_eq!(app_actors[0]["has_dck_wrap"], true);
    assert_eq!(app_actors[0]["is_current_app_key"], true);
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
fn rotate_dck_on_linked_app_key_errors() {
    let owner_dir = tempdir().unwrap();
    idrive(owner_dir.path()).arg("init").assert().success();
    let owner = serde_json::from_str::<serde_json::Value>(
        &String::from_utf8(
            idrive(owner_dir.path())
                .arg("whoami")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap();
    let owner_invite = device_link_invite_url(&owner).to_string();
    let linked_dir = tempdir().unwrap();
    idrive(linked_dir.path())
        .args(["link", &owner_invite])
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

    let owner = serde_json::from_str::<serde_json::Value>(
        &String::from_utf8(
            idrive(owner_dir.path())
                .arg("whoami")
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap();
    let owner_invite = device_link_invite_url(&owner).to_string();
    let linked_dir = tempdir().unwrap();
    let linked_v: serde_json::Value = serde_json::from_str(
        &String::from_utf8(
            idrive(linked_dir.path())
                .args(["link", &owner_invite])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap(),
    )
    .unwrap();
    let linked_app_key_npub = current_app_key_npub(&linked_v).to_string();

    idrive(owner_dir.path())
        .args(["approve", &linked_app_key_npub])
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
fn whoami_after_init_reports_profile_and_current_app_key() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    let out = idrive(dir.path()).arg("whoami").output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert!(
        v["current_app_key_npub"]
            .as_str()
            .unwrap()
            .starts_with("npub1")
    );
    assert_eq!(
        v["profile"]["current_app_key_npub"],
        v["current_app_key_npub"]
    );
    assert_eq!(v["profile"]["can_admin_profile"], true);
    assert_eq!(v["can_admin_profile"], true);
    assert_eq!(v["can_write_roots"], true);
}
