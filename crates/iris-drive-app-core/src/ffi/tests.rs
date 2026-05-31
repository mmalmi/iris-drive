use super::{FfiApp, normalize_pubkey};
use crate::NativeAppAction;
use hashtree_provider::{HashTreeProviderFs, ProviderFs};
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
    assert_eq!(state.ui.devices[0].role, "admin");
    assert!(state.ui.snapshot_link.is_empty());
    assert!(state.ui.sync.running);
    assert_eq!(state.ui.sync.status, "running");
    assert!(!state.ui.relays.is_empty());
    assert!(!state.ui.backups.is_empty());
    assert_eq!(state.ui.backups[0].label, "Blossom remote");
    assert_eq!(state.ui.paths.data_dir, dir.path().display().to_string());
    assert_eq!(state.ui.setup_state, "authorized");
    assert_eq!(state.ui.primary_status, "ready");
    assert_eq!(state.ui.authorized_device_count, 1);
    assert_eq!(state.ui.online_device_count, 0);

    let state = app.dispatch(NativeAppAction::StartSync);
    assert!(state.ui.sync.running);
    assert_eq!(state.ui.sync.status, "up to date");

    let state = app.dispatch(NativeAppAction::StopSync);
    assert!(!state.ui.sync.running);
    assert_eq!(state.ui.sync.status, "paused");
}

#[test]
fn uninitialized_state_exposes_summary_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.state();

    assert_eq!(state.ui.setup_state, "not_configured");
    assert_eq!(state.ui.primary_status, "not_setup");
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
    let linked = refreshed
        .ui
        .devices
        .iter()
        .find(|device| device.pubkey == linked_device)
        .expect("linked device in roster");
    assert!(!linked.is_current_device);
    assert!(linked.is_online);
    assert_eq!(linked.state, "Linked");

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

    write_native_fips_status_fixture(
        owner_dir.path(),
        &current_device,
        &[],
        &[],
        super::unix_now_seconds().saturating_sub(120),
    );
    let stale = app.refresh();
    assert!(stale.ui.devices.iter().all(|device| !device.is_online));
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
        "connected_peers": connected_peers,
        "mesh_peers": mesh_peers,
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
fn import_file_action_writes_shared_file_into_provider_root() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "iPhone".to_owned(),
    });

    let source = dir.path().join("share-source.txt");
    std::fs::write(&source, b"from share sheet").unwrap();
    let state = app.dispatch(NativeAppAction::ImportFile {
        display_name: "Shared note.txt".to_owned(),
        source_path: source.display().to_string(),
    });

    assert!(state.error.is_empty(), "{}", state.error);
    assert_eq!(state.ui.file_count, 1);
    assert_eq!(state.ui.visible_file_bytes, 16);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let daemon = iris_drive_core::Daemon::open(dir.path()).unwrap();
        let visible = iris_drive_core::primary_merged_root(daemon.tree(), daemon.config())
            .await
            .unwrap();
        let provider = HashTreeProviderFs::open(daemon.tree_handle(), visible.root_cid)
            .await
            .unwrap();
        let path = "Shared note.txt".to_owned();
        let item = provider.item(&path).await.unwrap();
        let bytes = provider.read(&path, 0, item.size).await.unwrap();
        assert_eq!(bytes, b"from share sheet");
    });
}

#[test]
fn provider_list_includes_summary_and_change_key() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "iPhone".to_owned(),
    });
    let source = dir.path().join("nested.txt");
    std::fs::write(&source, b"nested bytes").unwrap();

    assert!(
        super::native_provider_mkdir_json(&dir.path().display().to_string(), "Reports")["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
    assert!(
        super::native_provider_write_json(
            &dir.path().display().to_string(),
            "Reports/nested.txt",
            &source.display().to_string(),
        )["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    let provider = super::native_provider_list_json(&dir.path().display().to_string());

    assert_eq!(provider["file_count"], 1);
    assert_eq!(provider["visible_file_bytes"], 12);
    assert_eq!(
        provider["directory_paths"].as_array().unwrap(),
        &vec![serde_json::json!("Reports")]
    );
    assert!(
        provider["change_key"]
            .as_str()
            .is_some_and(|key| { key.contains("Reports/nested.txt") && key.contains("file") })
    );
    let entries = provider["entries"].as_array().unwrap();
    let reports = entries
        .iter()
        .find(|entry| entry["path"] == "Reports")
        .unwrap();
    assert_eq!(reports["parent_path"], "");
    assert_eq!(reports["display_name"], "Reports");
    let nested = entries
        .iter()
        .find(|entry| entry["path"] == "Reports/nested.txt")
        .unwrap();
    assert_eq!(nested["parent_path"], "Reports");
    assert_eq!(nested["display_name"], "nested.txt");
}

#[test]
fn provider_resolve_path_normalizes_name_and_avoids_collisions() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "iPhone".to_owned(),
    });
    let source = dir.path().join("shared.txt");
    std::fs::write(&source, b"first").unwrap();

    assert!(
        super::native_provider_mkdir_json(&dir.path().display().to_string(), "Reports")["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
    assert!(
        super::native_provider_write_json(
            &dir.path().display().to_string(),
            "Reports/Shared_file.txt",
            &source.display().to_string(),
        )["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    let resolved = super::native_provider_resolve_path_json(
        &dir.path().display().to_string(),
        "/Reports/",
        "Shared/file.txt",
        "",
    );

    assert_eq!(resolved["parent_path"], "Reports");
    assert_eq!(resolved["display_name"], "Shared_file (2).txt");
    assert_eq!(resolved["path"], "Reports/Shared_file (2).txt");
    assert!(resolved["error"].as_str().unwrap_or_default().is_empty());
}

#[test]
fn native_sync_applies_remote_drive_root_into_provider_listing() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner_state = owner_app.dispatch(NativeAppAction::CreateProfile {
        device_label: "Mac".to_owned(),
    });
    let owner_account = owner_state.ui.account.unwrap();

    let source_dir = tempfile::tempdir().unwrap();
    std::fs::write(source_dir.path().join("owner-note.txt"), b"from owner").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let mut daemon = iris_drive_core::Daemon::open(owner_dir.path()).unwrap();
        daemon.import_source_dir(source_dir.path()).await.unwrap();
    });

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        owner_pubkey: owner_account.device_link_invite,
        device_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.account.unwrap();
    let approved = owner_app.dispatch(NativeAppAction::ApproveDevice {
        request: linked_account.device_link_request,
        label: "Phone".to_owned(),
    });
    assert!(approved.error.is_empty(), "{}", approved.error);

    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
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
    let mut linked_config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    iris_drive_core::relay_sync::apply_remote_app_keys_event(&mut linked_config, &app_keys_event)
        .unwrap();
    linked_config
        .save(config_path_in(linked_dir.path()))
        .unwrap();

    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let owner_account_state = owner_config.account.as_ref().unwrap();
    let owner =
        iris_drive_core::Account::load(owner_account_state.clone(), owner_dir.path()).unwrap();
    let drive = owner_config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .unwrap();
    let root = drive
        .device_roots
        .get(&owner_account_state.device_pubkey)
        .unwrap();
    let authorized = owner_account_state
        .app_keys
        .as_ref()
        .unwrap()
        .devices
        .iter()
        .map(|device| device.pubkey.clone())
        .collect::<Vec<_>>();
    let drive_root_event = iris_drive_core::nostr_events::build_drive_root_event(
        owner.device.keys(),
        &owner_account_state.owner_pubkey,
        iris_drive_core::PRIMARY_DRIVE_ID,
        root,
        &authorized,
    )
    .unwrap();
    copy_blocks(owner_dir.path(), linked_dir.path());

    super::run_native_sync_once_with_drive_root_events_for_test(
        linked_dir.path(),
        &[drive_root_event],
    )
    .unwrap();

    let provider = super::native_provider_list_json(&linked_dir.path().display().to_string());
    let entries = provider["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|entry| entry["path"] == "owner-note.txt")
    );
    let owner_note = entries
        .iter()
        .find(|entry| entry["path"] == "owner-note.txt")
        .expect("provider list includes owner note");
    assert!(
        owner_note["modified_at"]
            .as_i64()
            .is_some_and(|modified_at| modified_at > 0),
        "provider list should include non-epoch modification time: {owner_note:#?}"
    );
}

#[test]
fn provider_modified_at_index_ignores_unix_epoch_sentinel() {
    let mut index = std::collections::BTreeMap::new();
    crate::provider_metadata::remember_provider_modified_at(&mut index, "old-note.txt", 1);
    crate::provider_metadata::remember_provider_modified_at(
        &mut index,
        "new-note.txt",
        1_700_000_000,
    );

    assert!(!index.contains_key("old-note.txt"));
    assert_eq!(index.get("new-note.txt"), Some(&1_700_000_000));
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

fn copy_blocks(from: &Path, to: &Path) {
    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let from_path = entry.path();
            let to_path = to.join(entry.file_name());
            if from_path.is_dir() {
                copy_dir(&from_path, &to_path);
            } else {
                std::fs::copy(&from_path, &to_path).unwrap();
            }
        }
    }

    copy_dir(&from.join("blocks"), &to.join("blocks"));
}
