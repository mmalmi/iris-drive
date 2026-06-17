use super::{FfiApp, normalize_pubkey};
use crate::NativeAppAction;
use iris_drive_core::paths::config_path_in;
use iris_drive_core::AppConfig;
use std::path::Path;

fn record_inbound_request(config_dir: &Path, device: &str, label: &str, requested_at: u64) {
    let config_path = config_path_in(config_dir);
    let device_hex = normalize_pubkey(device).unwrap();
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    let state = config.profile.as_mut().unwrap();
    let profile_id = state.profile_id;
    let link_secret = state.app_key_link_secret.clone();
    state
        .record_inbound_app_key_link_request(
            profile_id,
            &device_hex,
            Some(label.to_owned()),
            &link_secret,
            requested_at,
        )
        .unwrap();
    config.save(&config_path).unwrap();
}

#[test]
fn owner_can_reject_then_approve_join_requests_e2e() {
    let owner_dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let invite = owner.ui.profile.unwrap().app_key_link_invite;

    let rejected_dir = tempfile::tempdir().unwrap();
    let rejected_app = FfiApp::new(rejected_dir.path().display().to_string(), "test".to_owned());
    let rejected = rejected_app.dispatch(NativeAppAction::LinkDevice {
        link_target: invite.clone(),
        app_key_label: "Old iPhone".to_owned(),
    });
    let rejected_device = rejected.ui.profile.unwrap().current_app_key_npub;
    record_inbound_request(owner_dir.path(), &rejected_device, "Old iPhone", 41);

    let refreshed = app.refresh();
    let rejected_request = refreshed.ui.profile.unwrap().inbound_app_key_link_requests[0]
        .request_link
        .clone();
    let after_reject = app.dispatch(NativeAppAction::RejectDevice {
        request: rejected_request,
    });
    assert!(after_reject.error.is_empty(), "{}", after_reject.error);
    assert!(
        after_reject
            .ui
            .profile
            .as_ref()
            .unwrap()
            .inbound_app_key_link_requests
            .is_empty()
    );
    assert!(
        after_reject
            .ui
            .app_actors
            .iter()
            .all(|device| device.pubkey != rejected_device)
    );

    let after_reject_refresh = app.refresh();
    assert!(after_reject_refresh.error.is_empty());
    assert!(
        after_reject_refresh
            .ui
            .profile
            .as_ref()
            .unwrap()
            .inbound_app_key_link_requests
            .is_empty()
    );
    assert!(
        after_reject_refresh
            .ui
            .app_actors
            .iter()
            .all(|device| device.pubkey != rejected_device)
    );

    let approved_dir = tempfile::tempdir().unwrap();
    let approved_app = FfiApp::new(approved_dir.path().display().to_string(), "test".to_owned());
    let approved = approved_app.dispatch(NativeAppAction::LinkDevice {
        link_target: invite,
        app_key_label: "iPhone".to_owned(),
    });
    let approved_device = approved.ui.profile.unwrap().current_app_key_npub;
    record_inbound_request(owner_dir.path(), &approved_device, "iPhone", 42);

    let refreshed = app.refresh();
    let account = refreshed.ui.profile.unwrap();
    assert_eq!(account.inbound_app_key_link_requests.len(), 1);
    let approve_request = &account.inbound_app_key_link_requests[0];
    assert_eq!(approve_request.app_key_pubkey, approved_device);
    assert_eq!(approve_request.label, "iPhone");

    let after_approve = app.dispatch(NativeAppAction::ApproveDevice {
        request: approve_request.request_link.clone(),
        label: String::new(),
    });
    assert!(after_approve.error.is_empty(), "{}", after_approve.error);
    assert!(
        after_approve
            .ui
            .profile
            .as_ref()
            .unwrap()
            .inbound_app_key_link_requests
            .is_empty()
    );
    assert!(after_approve.ui.app_actors.iter().any(|device| {
        device.pubkey == approved_device && device.label == "iPhone" && device.role == "member"
    }));
    assert!(
        after_approve
            .ui
            .app_actors
            .iter()
            .all(|device| device.pubkey != rejected_device)
    );

    let final_refresh = app.refresh();
    assert!(final_refresh.error.is_empty());
    assert!(final_refresh.ui.app_actors.iter().any(|device| {
        device.pubkey == approved_device && device.label == "iPhone" && device.role == "member"
    }));
    assert!(
        final_refresh
            .ui
            .app_actors
            .iter()
            .all(|device| device.pubkey != rejected_device)
    );
    assert!(
        AppConfig::load_or_default(config_path_in(owner_dir.path()))
            .unwrap()
            .profile
            .unwrap()
            .inbound_app_key_link_requests
            .is_empty()
    );
}
