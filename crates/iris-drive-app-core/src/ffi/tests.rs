use super::{FfiApp, normalize_pubkey};
use crate::NativeAppAction;
use iris_drive_core::paths::config_path_in;
use iris_drive_core::{AppConfig, DeviceRootRef, Drive};
use nostr_sdk::JsonUtil;
use std::path::Path;

#[test]
fn dispatch_adds_updates_and_removes_roots() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.dispatch(NativeAppAction::AddRoot {
        name: "My Drive".to_owned(),
        local_path: "/virtual/iris".to_owned(),
    });
    assert_eq!(state.ui.roots.len(), 1);
    assert_eq!(state.ui.roots[0].name, "My Drive");
    assert!(state.error.is_empty());

    let state = app.dispatch(NativeAppAction::AddRoot {
        name: "My Drive".to_owned(),
        local_path: "/virtual/iris-renamed".to_owned(),
    });
    assert_eq!(state.ui.roots.len(), 1);
    assert_eq!(state.ui.roots[0].local_path, "/virtual/iris-renamed");

    let state = app.dispatch(NativeAppAction::RemoveRoot {
        name: "My Drive".to_owned(),
    });
    assert!(state.ui.roots.is_empty());
    assert!(state.error.is_empty());
}

#[test]
fn dispatch_rejects_empty_roots() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.dispatch(NativeAppAction::AddRoot {
        name: String::new(),
        local_path: "/virtual/iris".to_owned(),
    });

    assert!(state.ui.roots.is_empty());
    assert_eq!(state.error, "root name is required");
}

#[test]
fn profile_actions_populate_mobile_parity_state() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "Pixel".to_owned(),
    });

    let account = state.ui.account.as_ref().expect("account exists");
    assert_eq!(account.device_label, "Pixel");
    assert_eq!(account.authorization_state, "authorized");
    assert!(account.has_owner_signing_authority);
    assert!(account.device_link_request.is_empty());
    assert!(
        account
            .device_link_invite
            .starts_with("iris-drive://invite/")
    );
    assert!(!account.device_link_invite.contains("local-owner"));
    assert!(!account.device_link_invite.contains("device-"));
    assert_eq!(state.ui.devices.len(), 1);
    assert_eq!(state.ui.devices[0].label, "Pixel");
    assert_eq!(state.ui.devices[0].display_label, "This device");
    assert_eq!(state.ui.devices[0].role, "admin");
    assert_eq!(state.ui.devices[0].role_label, "Admin");
    assert_eq!(state.ui.devices[0].state_label, "Linked");
    assert_eq!(state.ui.devices[0].connection_state, "local");
    assert_eq!(state.ui.devices[0].connection_label, "This device");
    assert!(state.ui.snapshot_link.is_empty());
    assert!(state.ui.sync.running);
    assert_eq!(state.ui.sync.status, "running");
    assert_eq!(state.ui.sync.status_label, "Sync on");
    assert!(!state.ui.relays.is_empty());
    assert_eq!(state.ui.relay_statuses.len(), state.ui.relays.len());
    assert_eq!(state.ui.relay_statuses[0].status_label, "saved");
    assert_eq!(state.ui.relay_statuses[0].health, "configured");
    assert!(!state.ui.backups.is_empty());
    assert_eq!(state.ui.backups[0].label, "Blossom remote");
    assert_eq!(state.ui.paths.data_dir, dir.path().display().to_string());
    assert_eq!(state.ui.setup_state, "authorized");
    assert!(state.ui.setup_complete);
    assert!(!state.ui.awaiting_approval);
    assert!(!state.ui.revoked);
    assert_eq!(state.ui.setup_label, "Linked");
    assert_eq!(state.ui.primary_status, "ready");
    assert_eq!(state.ui.primary_status_label, "Ready");
    assert_eq!(state.ui.authorized_device_count, 1);
    assert_eq!(state.ui.online_device_count, 0);

    let state = app.dispatch(NativeAppAction::StartSync);
    assert!(state.ui.sync.running);
    assert_eq!(state.ui.sync.status, "up to date");
    assert_eq!(state.ui.sync.status_label, "Up to date");

    let state = app.dispatch(NativeAppAction::StopSync);
    assert!(!state.ui.sync.running);
    assert_eq!(state.ui.sync.status, "paused");
    assert_eq!(state.ui.sync.status_label, "Sync paused");
}

#[test]
fn relay_actions_normalize_and_dedupe_urls() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.dispatch(NativeAppAction::AddRelay {
        url: " relay.example/ ".to_owned(),
    });

    assert!(state.error.is_empty());
    assert!(state.ui.relays.contains(&"wss://relay.example".to_owned()));
    assert!(!state.ui.relays.contains(&"relay.example/".to_owned()));
    assert_eq!(
        state
            .ui
            .relays
            .iter()
            .filter(|relay| relay.as_str() == "wss://relay.example")
            .count(),
        1
    );

    let state = app.dispatch(NativeAppAction::AddRelay {
        url: "wss://relay.example".to_owned(),
    });
    assert_eq!(
        state
            .ui
            .relays
            .iter()
            .filter(|relay| relay.as_str() == "wss://relay.example")
            .count(),
        1
    );

    let relay_status = state
        .ui
        .relay_statuses
        .iter()
        .find(|relay| relay.url == "wss://relay.example")
        .expect("normalized relay status is emitted");
    assert_eq!(relay_status.status_label, "saved");
    assert_eq!(relay_status.health, "configured");

    let state = app.dispatch(NativeAppAction::RemoveRelay {
        url: "relay.example/".to_owned(),
    });
    assert!(state.error.is_empty());
    assert!(!state.ui.relays.contains(&"wss://relay.example".to_owned()));
}

#[test]
fn uninitialized_state_exposes_summary_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.state();

    assert_eq!(state.ui.setup_state, "not_configured");
    assert!(!state.ui.setup_complete);
    assert!(!state.ui.awaiting_approval);
    assert!(!state.ui.revoked);
    assert_eq!(state.ui.setup_label, "Not linked");
    assert_eq!(state.ui.primary_status, "not_setup");
    assert_eq!(state.ui.primary_status_label, "Ready");
    assert_eq!(state.ui.authorized_device_count, 0);
    assert_eq!(state.ui.online_device_count, 0);
    assert_eq!(state.ui.file_count, 0);
    assert_eq!(state.ui.visible_file_bytes, 0);
}

#[test]
fn classify_link_input_uses_core_invite_and_key_parsing() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "Mac".to_owned(),
    });
    let owner_account = owner.ui.account.unwrap();

    let invite = super::classify_link_input(owner_account.device_link_invite.clone());
    assert_eq!(invite.kind, "invite");
    assert!(invite.is_complete);
    assert!(invite.is_valid);
    assert_eq!(invite.owner_pubkey, owner_account.owner_pubkey);
    assert!(!invite.admin_device_pubkey.is_empty());
    assert!(invite.has_link_secret);

    let npub = super::classify_link_input(owner_account.owner_pubkey.clone());
    assert_eq!(npub.kind, "owner_pubkey");
    assert!(npub.is_complete);
    assert!(npub.is_valid);
    assert_eq!(npub.owner_pubkey, owner_account.owner_pubkey);

    let short_invite = super::classify_link_input("iris-drive://invite/abc".to_owned());
    assert_eq!(short_invite.kind, "invite");
    assert!(!short_invite.is_complete);
    assert!(!short_invite.is_valid);

    let short_npub = super::classify_link_input(owner_account.owner_pubkey[..20].to_owned());
    assert_eq!(short_npub.kind, "owner_pubkey");
    assert!(!short_npub.is_complete);
    assert!(!short_npub.is_valid);

    let approval = super::classify_link_input(format!(
        "iris-drive://device-link?owner={}&device={}",
        owner_account.owner_pubkey, owner_account.owner_pubkey
    ));
    assert_eq!(approval.kind, "device_approval");
    assert!(approval.is_complete);
    assert!(approval.is_valid);
    assert_eq!(approval.owner_pubkey, owner_account.owner_pubkey);
    assert_eq!(approval.device_pubkey, owner_account.owner_pubkey);

    let web_invite_route =
        super::classify_link_input("https://drive.iris.to/invite/demo".to_owned());
    assert_eq!(web_invite_route.kind, "invite");
    assert!(!web_invite_route.is_complete);

    let unrelated = super::classify_link_input(
        "https://drive.iris.to/device-linker?owner=npub1example".to_owned(),
    );
    assert_eq!(unrelated.kind, "unknown");
}

#[test]
fn snapshot_link_uses_drive_iris_nhash_route() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let created = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "Pixel".to_owned(),
    });
    let account = created.ui.account.as_ref().expect("account exists");
    let root_cid = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f\
                    :1f1e1d1c1b1a191817161514131211100f0e0d0c0b0a09080706050403020100"
        .replace(char::is_whitespace, "");

    let config_path = config_path_in(dir.path());
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    let mut drive = Drive::primary(&account.owner_pubkey);
    drive.last_root_cid = Some(root_cid.clone());
    drive.device_roots.insert(
        account.device_pubkey.clone(),
        DeviceRootRef::legacy(root_cid, 1, 0),
    );
    config.upsert_drive(drive);
    config.save(&config_path).unwrap();

    let refreshed = app.refresh();

    assert!(
        refreshed
            .ui
            .snapshot_link
            .starts_with("https://drive.iris.to/#/nhash1")
    );
    assert!(!refreshed.ui.snapshot_link.contains("/snapshot/"));
}

#[test]
fn logout_clears_local_profile_state_and_key_material() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let created = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "iPhone".to_owned(),
    });
    assert!(created.ui.account.is_some());
    assert!(dir.path().join("key").exists());

    let state = app.dispatch(NativeAppAction::Logout);

    assert!(state.error.is_empty());
    assert!(state.ui.account.is_none());
    assert!(state.ui.devices.is_empty());
    assert!(state.ui.roots.is_empty());
    assert!(!state.ui.sync.running);
    assert_eq!(state.ui.sync.status, "paused");
    assert_eq!(state.ui.sync.status_label, "Sync paused");
    assert!(!dir.path().join("key").exists());
    let config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    assert!(config.account.is_none());
    assert!(config.drives.is_empty());
}

#[test]
fn link_action_tracks_pending_approval() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        device_label: "Owner".to_owned(),
    });
    let owner_account = owner.ui.account.unwrap();
    let owner_npub = owner_account.owner_pubkey.clone();
    let invite = owner_account.device_link_invite.clone();

    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.dispatch(NativeAppAction::LinkDevice {
        owner_pubkey: invite,
        device_label: "iPhone".to_owned(),
    });

    let account = state.ui.account.expect("account exists");
    assert_eq!(account.owner_pubkey, owner_npub);
    assert_eq!(account.device_label, "iPhone");
    assert_eq!(account.authorization_state, "awaiting_approval");
    assert!(!account.has_owner_signing_authority);
    assert!(account.device_link_request.contains("device=npub1"));
    assert!(account.device_link_request.contains("secret="));
    assert!(!account.device_link_request.contains("local-owner"));
    assert!(!account.device_link_request.contains("device=device-"));
    assert!(
        state.ui.devices.is_empty(),
        "pending devices should not appear in the authorized-device roster"
    );
    assert_eq!(state.ui.setup_state, "awaiting_approval");
    assert!(!state.ui.setup_complete);
    assert!(state.ui.awaiting_approval);
    assert!(!state.ui.revoked);
    assert_eq!(state.ui.primary_status, "awaiting_approval");
    assert_eq!(state.ui.authorized_device_count, 0);
}

#[test]
fn owner_can_approve_and_revoke_linked_devices() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());

    let owner = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "Mac".to_owned(),
    });
    let owner_npub = owner.ui.account.unwrap().owner_pubkey;
    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        owner_pubkey: owner_npub,
        device_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.account.unwrap();
    let request = linked_account.device_link_request;
    let linked_device = linked_account.device_pubkey;
    let state = app.dispatch(NativeAppAction::ApproveDevice {
        request,
        label: "Phone".to_owned(),
    });

    assert!(state.ui.devices.iter().any(|device| {
        device.pubkey == linked_device
            && device.label == "Phone"
            && device.role == "member"
            && device.can_revoke
            && device.can_appoint_admin
    }));

    let state = app.dispatch(NativeAppAction::AppointAdmin {
        device_pubkey: linked_device.clone(),
    });
    assert!(state.ui.devices.iter().any(|device| {
        device.pubkey == linked_device
            && device.role == "admin"
            && device.can_demote_admin
            && !device.can_appoint_admin
    }));

    let state = app.dispatch(NativeAppAction::DemoteAdmin {
        device_pubkey: linked_device.clone(),
    });
    assert!(state.ui.devices.iter().any(|device| {
        device.pubkey == linked_device
            && device.role == "member"
            && !device.can_demote_admin
            && device.can_appoint_admin
    }));

    let state = app.dispatch(NativeAppAction::RevokeDevice {
        device_pubkey: linked_device.clone(),
    });

    assert!(
        !state
            .ui
            .devices
            .iter()
            .any(|device| device.pubkey == linked_device)
    );
    assert!(state.error.is_empty());
}

#[test]
fn delete_device_json_action_revokes_linked_device() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());

    let owner = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "Mac".to_owned(),
    });
    let owner_npub = owner.ui.account.unwrap().owner_pubkey;
    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        owner_pubkey: owner_npub,
        device_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.account.unwrap();
    let linked_device = linked_account.device_pubkey.clone();
    let state = app.dispatch(NativeAppAction::ApproveDevice {
        request: linked_account.device_link_request,
        label: "Phone".to_owned(),
    });
    assert!(
        state
            .ui
            .devices
            .iter()
            .any(|device| device.pubkey == linked_device)
    );

    let action: NativeAppAction = serde_json::from_value(serde_json::json!({
        "type": "delete_device",
        "device_pubkey": linked_device,
    }))
    .unwrap();
    let state = app.dispatch(action);

    assert!(
        state
            .ui
            .devices
            .iter()
            .all(|device| device.label != "Phone")
    );
    assert!(state.error.is_empty());
}

#[test]
fn revoked_current_device_refresh_pauses_sync_and_keeps_relink_context() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());

    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        device_label: "Mac".to_owned(),
    });
    let owner_account = owner.ui.account.unwrap();
    let owner_npub = owner_account.owner_pubkey.clone();
    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        owner_pubkey: owner_account.device_link_invite,
        device_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.account.unwrap();
    let linked_device = linked_account.device_pubkey.clone();
    let approved = owner_app.dispatch(NativeAppAction::ApproveDevice {
        request: linked_account.device_link_request,
        label: "Phone".to_owned(),
    });
    assert!(approved.error.is_empty(), "{}", approved.error);
    apply_latest_app_keys_event(owner_dir.path(), linked_dir.path());

    let authorized = linked_app.refresh();
    let account = authorized.ui.account.as_ref().expect("account exists");
    assert_eq!(account.authorization_state, "authorized");
    assert!(authorized.ui.devices.iter().any(|device| {
        device.pubkey == linked_device && device.label == "Phone" && device.is_current_device
    }));

    let running = linked_app.dispatch(NativeAppAction::StartSync);
    assert!(running.ui.sync.running);

    let revoked = owner_app.dispatch(NativeAppAction::RevokeDevice {
        device_pubkey: linked_device.clone(),
    });
    assert!(revoked.error.is_empty(), "{}", revoked.error);
    apply_latest_app_keys_event(owner_dir.path(), linked_dir.path());

    let refreshed = linked_app.refresh();
    let account = refreshed.ui.account.as_ref().expect("account exists");
    assert_eq!(account.authorization_state, "revoked");
    assert_eq!(account.owner_pubkey, owner_npub);
    assert_eq!(account.device_pubkey, linked_device);
    assert_eq!(account.device_label, "Phone");
    assert!(account.device_link_request.is_empty());
    assert!(account.device_link_invite.is_empty());
    assert!(account.inbound_device_link_requests.is_empty());
    assert!(refreshed.ui.devices.is_empty());
    assert!(refreshed.ui.roots.is_empty());
    assert!(refreshed.ui.snapshot_link.is_empty());
    assert!(!refreshed.ui.sync.running);
    assert_eq!(refreshed.ui.sync.status, "paused");
    assert_eq!(refreshed.ui.sync.status_label, "Sync paused");

    let relinked = linked_app.dispatch(NativeAppAction::LinkDevice {
        owner_pubkey: owner_npub,
        device_label: "Phone".to_owned(),
    });
    let account = relinked.ui.account.as_ref().expect("account exists");
    assert!(relinked.error.is_empty(), "{}", relinked.error);
    assert_eq!(account.authorization_state, "awaiting_approval");
    assert_ne!(account.device_pubkey, linked_device);
    assert_eq!(account.device_label, "Phone");
    assert!(account.device_link_request.contains("device=npub1"));
}

#[test]
fn native_fips_status_drives_device_online_presence() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());

    let owner = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "Mac".to_owned(),
    });
    let owner_account = owner.ui.account.unwrap();
    let owner_npub = owner_account.owner_pubkey;
    let current_device = owner_account.device_pubkey;

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        owner_pubkey: owner_npub,
        device_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.account.unwrap();
    let linked_device = linked_account.device_pubkey;

    let approved = app.dispatch(NativeAppAction::ApproveDevice {
        request: linked_account.device_link_request,
        label: "Phone".to_owned(),
    });
    assert!(approved.error.is_empty());
    assert!(approved.ui.devices.iter().all(|device| !device.is_online));

    write_native_fips_status_fixture(
        owner_dir.path(),
        &current_device,
        &[linked_device.as_str()],
        &[],
        super::unix_now_seconds(),
    );
    let refreshed = app.refresh();
    let current = refreshed
        .ui
        .devices
        .iter()
        .find(|device| device.pubkey == current_device)
        .expect("current device in roster");
    assert!(current.is_current_device);
    assert!(current.is_online);
    assert_eq!(current.state, "Linked");
    assert_eq!(current.connection_state, "local");
    assert_eq!(current.connection_label, "This device");
    assert_eq!(refreshed.ui.fips.state, "running");
    assert_eq!(refreshed.ui.fips.state_label, "Running");
    assert_eq!(refreshed.ui.fips.roster_label, "1/1 online");
    assert_eq!(refreshed.ui.fips.roster_peer_count, 1);
    assert_eq!(refreshed.ui.fips.roster_online_device_count, 1);
    assert_eq!(refreshed.ui.fips.roster_direct_device_count, 1);
    assert_eq!(refreshed.ui.fips.other_peer_count, 0);
    assert_eq!(refreshed.ui.fips.peer_statuses.len(), 1);
    assert_eq!(refreshed.ui.fips.peer_statuses[0].npub, linked_device);
    assert_eq!(
        refreshed.ui.fips.peer_statuses[0].connection_label,
        "TCP, 12 ms"
    );
    let linked = refreshed
        .ui
        .devices
        .iter()
        .find(|device| device.pubkey == linked_device)
        .expect("linked device in roster");
    assert!(!linked.is_current_device);
    assert!(linked.is_online);
    assert_eq!(linked.state, "Linked");
    assert_eq!(linked.connection_state, "direct");
    assert_eq!(linked.connection_label, "Online");

    write_native_fips_status_fixture(
        owner_dir.path(),
        &current_device,
        &[],
        &[linked_device.as_str()],
        super::unix_now_seconds(),
    );
    let mesh_only = app.refresh();
    let linked = mesh_only
        .ui
        .devices
        .iter()
        .find(|device| device.pubkey == linked_device)
        .expect("linked device in roster");
    assert!(linked.is_online);
    assert_eq!(linked.connection_state, "mesh");
    assert_eq!(linked.connection_label, "Online (Mesh)");

    write_native_fips_status_fixture(
        owner_dir.path(),
        &current_device,
        &[],
        &[linked_device.as_str()],
        super::unix_now_seconds().saturating_sub(120),
    );
    let stale = app.refresh();
    assert!(stale.ui.devices.iter().all(|device| !device.is_online));
    assert_eq!(stale.ui.fips.state, "stale");
    assert_eq!(stale.ui.fips.state_label, "Stale");
    assert_eq!(stale.ui.fips.roster_label, "0/1 online");
    assert_eq!(stale.ui.fips.roster_online_device_count, 0);
    let linked = stale
        .ui
        .devices
        .iter()
        .find(|device| device.pubkey == linked_device)
        .expect("linked device in stale roster");
    assert_eq!(linked.connection_state, "offline");
    assert_eq!(linked.connection_label, "Offline");
    assert_eq!(stale.ui.fips.online_device_count, 0);
    assert!(!stale.ui.fips.fresh);
}

#[test]
fn owner_state_surfaces_inbound_requests_for_accept_flow() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "Mac".to_owned(),
    });
    let invite = owner.ui.account.unwrap().device_link_invite;

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        owner_pubkey: invite,
        device_label: "Phone".to_owned(),
    });
    let linked_device = linked.ui.account.unwrap().device_pubkey;
    let linked_device_hex = normalize_pubkey(&linked_device).unwrap();

    let config_path = config_path_in(owner_dir.path());
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    let state = config.account.as_mut().unwrap();
    let owner_hex = state.owner_pubkey.clone();
    let link_secret = state.device_link_secret.clone();
    state
        .record_inbound_device_link_request(
            &owner_hex,
            &linked_device_hex,
            Some("Phone".to_owned()),
            &link_secret,
            42,
        )
        .unwrap();
    config.save(&config_path).unwrap();

    let refreshed = app.refresh();
    let account = refreshed.ui.account.unwrap();
    assert_eq!(account.inbound_device_link_requests.len(), 1);
    let request = &account.inbound_device_link_requests[0];
    assert_eq!(request.device_pubkey, linked_device);
    assert_eq!(request.label, "Phone");
    assert_eq!(request.requested_at, 42);
    assert!(
        request
            .request_link
            .starts_with("iris-drive://device-link?")
    );
    assert!(request.request_link.contains("secret="));

    let approved = app.dispatch(NativeAppAction::ApproveDevice {
        request: request.request_link.clone(),
        label: String::new(),
    });
    assert!(approved.error.is_empty());
    assert!(approved.ui.devices.iter().any(|device| {
        device.pubkey == linked_device && device.label == "Phone" && device.role == "member"
    }));
}

fn write_native_fips_status_fixture(
    dir: &Path,
    endpoint_npub: &str,
    connected_peers: &[&str],
    mesh_peers: &[&str],
    updated_at: u64,
) {
    let path = dir.join(super::NATIVE_FIPS_STATUS_FILE_NAME);
    let value = serde_json::json!({
        "running": true,
        "updated_at": updated_at,
        "endpoint_npub": endpoint_npub,
        "authorized_peers": connected_peers.iter().chain(mesh_peers.iter()).copied().collect::<Vec<_>>(),
        "connected_peers": connected_peers,
        "mesh_peers": mesh_peers,
        "peer_statuses": connected_peers.iter().map(|peer| serde_json::json!({
            "npub": peer,
            "transport_type": "tcp",
            "srtt_ms": 12
        })).collect::<Vec<_>>(),
        "error": null,
    });
    std::fs::write(path, value.to_string()).unwrap();
}

#[test]
fn reset_invite_action_rotates_invite_and_clears_requests() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "Mac".to_owned(),
    });
    let old_invite = owner.ui.account.unwrap().device_link_invite;

    let config_path = config_path_in(owner_dir.path());
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    let state = config.account.as_mut().unwrap();
    let owner_hex = state.owner_pubkey.clone();
    let link_secret = state.device_link_secret.clone();
    let linked_device =
        iris_drive_core::DeviceIdentity::generate(owner_dir.path().join("tmp-key")).pubkey_hex();
    state
        .record_inbound_device_link_request(
            &owner_hex,
            &linked_device,
            Some("Phone".to_owned()),
            &link_secret,
            42,
        )
        .unwrap();
    config.save(&config_path).unwrap();

    let reset = app.dispatch(NativeAppAction::ResetInvite);
    assert!(reset.error.is_empty());
    let account = reset.ui.account.unwrap();
    assert_ne!(account.device_link_invite, old_invite);
    assert!(account.inbound_device_link_requests.is_empty());
}

#[test]
fn native_direct_root_app_keys_refreshes_authorized_member_roster() {
    let owner_dir = tempfile::tempdir().unwrap();
    let linked_dir = tempfile::tempdir().unwrap();
    let mut owner = iris_drive_core::Account::create(owner_dir.path(), Some("Mac".into())).unwrap();
    let mut linked = iris_drive_core::Account::link(
        linked_dir.path(),
        owner.state.owner_pubkey.clone(),
        Some("Phone".into()),
    )
    .unwrap();
    let linked_pubkey = linked.state.device_pubkey.clone();
    linked
        .state
        .queue_outbound_device_link_request(
            owner.state.device_pubkey.clone(),
            &owner.state.device_link_secret,
            123,
        )
        .unwrap();
    owner
        .approve_device(&linked_pubkey, Some("Phone".into()))
        .unwrap();

    let first_roster_event = iris_drive_core::nostr_events::build_app_keys_event(
        owner.device.keys(),
        owner.state.app_keys.as_ref().unwrap(),
    )
    .unwrap();
    let mut linked_config = AppConfig {
        account: Some(linked.state.clone()),
        ..AppConfig::default()
    };
    linked_config.upsert_drive(iris_drive_core::Drive::primary(&owner.state.owner_pubkey));
    iris_drive_core::relay_sync::apply_device_link_roster_event(
        &mut linked_config,
        &first_roster_event,
        &owner.state.device_pubkey,
    )
    .unwrap();
    linked_config
        .save(config_path_in(linked_dir.path()))
        .unwrap();

    let third_device = nostr_sdk::Keys::generate().public_key().to_hex();
    owner
        .approve_device(&third_device, Some("Pixel".into()))
        .unwrap();
    let updated_roster_event = iris_drive_core::nostr_events::build_app_keys_event(
        owner.device.keys(),
        owner.state.app_keys.as_ref().unwrap(),
    )
    .unwrap();

    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime
        .block_on(iris_drive_core::apply_direct_root_event(
            linked_dir.path(),
            &updated_roster_event,
            None,
        ))
        .unwrap();

    let linked_config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    let linked_roster = linked_config
        .account
        .as_ref()
        .unwrap()
        .app_keys
        .as_ref()
        .unwrap();
    assert!(linked_roster.contains(&third_device));
}

fn apply_latest_app_keys_event(from: &Path, to: &Path) {
    let owner_config = AppConfig::load_or_default(config_path_in(from)).unwrap();
    let app_keys_event = nostr_sdk::Event::from_json(
        &owner_config
            .account
            .as_ref()
            .unwrap()
            .app_keys_event
            .as_ref()
            .unwrap()
            .event_json,
    )
    .unwrap();
    let mut linked_config = AppConfig::load_or_default(config_path_in(to)).unwrap();
    iris_drive_core::relay_sync::apply_remote_app_keys_event(&mut linked_config, &app_keys_event)
        .unwrap();
    linked_config.save(config_path_in(to)).unwrap();
}
