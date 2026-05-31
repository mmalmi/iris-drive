use super::{FfiApp, normalize_pubkey};
use crate::NativeAppAction;
use hashtree_provider::{HashTreeProviderFs, ProviderFs};
use iris_drive_core::AppConfig;
use iris_drive_core::paths::config_path_in;

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
    assert!(state.ui.snapshot_link.contains(&account.owner_pubkey));
    assert!(!state.ui.relays.is_empty());
    assert!(!state.ui.backups.is_empty());
    assert_eq!(state.ui.paths.data_dir, dir.path().display().to_string());

    let state = app.dispatch(NativeAppAction::StartSync);
    assert!(state.ui.sync.running);
    assert_eq!(state.ui.sync.status, "running");

    let state = app.dispatch(NativeAppAction::StopSync);
    assert!(!state.ui.sync.running);
    assert_eq!(state.ui.sync.status, "paused");
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
