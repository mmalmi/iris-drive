use super::{
    FfiApp, NativeAppKeyLinkRelayEventApply, apply_native_app_key_link_relay_event_to_config,
    native_profile_roster_ops_pending_publish, normalize_pubkey,
};
use crate::NativeAppAction;
use crate::state::NativeAppState;
use iris_drive_core::paths::config_path_in;
use iris_drive_core::{AppConfig, AppKeyAuthorizationState};
use nostr_sdk::{Event, JsonUtil};
use std::collections::BTreeSet;
use std::path::Path;

fn record_inbound_request(
    config_dir: &Path,
    device: &str,
    label: &str,
    request_url: &str,
    requested_at: u64,
) {
    let config_path = config_path_in(config_dir);
    let device_hex = normalize_pubkey(device).unwrap();
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    let state = config.profile.as_mut().unwrap();
    let profile_id = state.profile_id;
    let invite_pubkey =
        iris_drive_core::app_key_link_invite_pubkey(&state.app_key_link_secret).unwrap();
    state
        .record_inbound_app_key_link_request(
            profile_id,
            &device_hex,
            Some(label.to_owned()),
            &invite_pubkey,
            request_url.to_owned(),
            requested_at,
        )
        .unwrap();
    config.save(&config_path).unwrap();
}

fn pending_request(
    config_dir: &Path,
) -> iris_drive_core::app_key_link_transport::AppKeyApprovalBootstrap {
    let config = AppConfig::load_or_default(config_path_in(config_dir)).unwrap();
    let pending = config
        .profile
        .as_ref()
        .and_then(|profile| profile.outbound_app_key_link_request.as_ref())
        .expect("pending request");
    iris_drive_core::app_key_link_transport::parse_pending_app_key_approval_bootstrap(pending)
        .expect("parse pending bootstrap")
        .0
}

fn approve_owner_from_pending_request(
    owner_app: &FfiApp,
    owner_dir: &Path,
    linked_dir: &Path,
    label: Option<String>,
) -> NativeAppState {
    let bootstrap = pending_request(linked_dir);
    let config_path = config_path_in(owner_dir);
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    let state = config.profile.clone().expect("owner profile");
    let mut account = iris_drive_core::Profile::load(state, owner_dir).unwrap();
    account.approve_device_bootstrap(&bootstrap, label).unwrap();
    config.profile = Some(account.state);
    config.save(&config_path).unwrap();
    owner_app.refresh()
}

#[test]
#[allow(clippy::too_many_lines)]
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
    let rejected_account = rejected.ui.profile.unwrap();
    let rejected_device = rejected_account.current_app_key_npub;
    record_inbound_request(
        owner_dir.path(),
        &rejected_device,
        "Old iPhone",
        &rejected_account.app_key_link_request,
        41,
    );

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
    let approved_account = approved.ui.profile.unwrap();
    let approved_device = approved_account.current_app_key_npub;
    record_inbound_request(
        owner_dir.path(),
        &approved_device,
        "iPhone",
        &approved_account.app_key_link_request,
        42,
    );

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

#[test]
fn roster_ops_cannot_authorize_waiting_native_device_before_bound_receipt() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Web owner".to_owned(),
    });
    let invite = owner.ui.profile.unwrap().app_key_link_invite;

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: invite,
        app_key_label: "iPhone".to_owned(),
    });
    assert!(linked.error.is_empty(), "{}", linked.error);
    let mut linked_config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    assert_eq!(
        linked_config.profile.as_ref().unwrap().authorization_state,
        AppKeyAuthorizationState::AwaitingApproval
    );

    let approved = approve_owner_from_pending_request(
        &owner_app,
        owner_dir.path(),
        linked_dir.path(),
        Some("iPhone".to_owned()),
    );
    assert!(approved.error.is_empty(), "{}", approved.error);
    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let owner_state = owner_config.profile.unwrap();
    assert_eq!(
        owner_state.pending_device_approval_receipts[0].relay_url,
        ""
    );
    let receipt_event = Event::from_json(
        &owner_state
            .pending_device_approval_receipts
            .first()
            .expect("owner queued approval receipt")
            .event_json,
    )
    .unwrap();

    let mut applied = 0;
    for op in &owner_state.profile_roster_ops {
        let event = Event::from_json(&op.event_json).unwrap();
        if apply_native_app_key_link_relay_event_to_config(&mut linked_config, &event).unwrap()
            == NativeAppKeyLinkRelayEventApply::AppliedRoster
        {
            applied += 1;
        }
    }

    assert!(applied >= 1);
    assert_eq!(
        linked_config.profile.as_ref().unwrap().authorization_state,
        AppKeyAuthorizationState::AwaitingApproval
    );
    let receipt_outcome =
        apply_native_app_key_link_relay_event_to_config(&mut linked_config, &receipt_event)
            .unwrap();
    assert!(matches!(
        receipt_outcome,
        NativeAppKeyLinkRelayEventApply::AppliedRoster | NativeAppKeyLinkRelayEventApply::Current
    ));
    for op in &owner_state.profile_roster_ops {
        let event = Event::from_json(&op.event_json).unwrap();
        apply_native_app_key_link_relay_event_to_config(&mut linked_config, &event).unwrap();
    }
    assert_eq!(
        linked_config.profile.as_ref().unwrap().authorization_state,
        AppKeyAuthorizationState::Authorized
    );
}

#[test]
fn start_join_request_tracks_pending_manual_approval() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.dispatch(NativeAppAction::StartJoinRequest {
        app_key_label: "iPhone".to_owned(),
    });

    assert!(state.error.is_empty(), "{}", state.error);
    let account = state.ui.profile.expect("account exists");
    assert_eq!(account.app_key_label, "iPhone");
    assert_eq!(account.authorization_state, "awaiting_approval");
    assert!(!account.can_admin_profile);
    assert!(
        account
            .app_key_link_request
            .starts_with(iris_drive_core::app_key_link_transport::APP_KEY_APPROVAL_REQUEST_PREFIX)
    );
    let request = pending_request(dir.path());
    assert!(!request.request_npub.is_empty());
    assert_eq!(request.request_secret.len(), 43);
    assert_eq!(request.device_app_key_npub, account.current_app_key_npub);
    assert_ne!(request.device_app_key_npub, request.request_npub);
    let config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    let pending = config
        .profile
        .as_ref()
        .and_then(|profile| profile.outbound_app_key_link_request.as_ref())
        .expect("pending request");
    assert!(pending.admin_app_key_pubkey.is_empty());
    assert!(pending.invite_pubkey.is_empty());
    assert_eq!(pending.request_url, account.app_key_link_request);
    assert_eq!(state.ui.setup_state, "awaiting_approval");
    assert!(!state.ui.setup_complete);
    assert!(state.ui.awaiting_approval);
    assert_eq!(state.ui.primary_status, "awaiting_approval");
}

#[test]
fn pending_request_refresh_repairs_stale_compact_approval_url() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Owner".to_owned(),
    });
    let invite = owner.ui.profile.unwrap().app_key_link_invite;

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: invite,
        app_key_label: "iPhone".to_owned(),
    });
    assert!(linked.error.is_empty(), "{}", linked.error);

    let config_path = config_path_in(linked_dir.path());
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    config
        .profile
        .as_mut()
        .unwrap()
        .outbound_app_key_link_request
        .as_mut()
        .unwrap()
        .request_url = format!(
        "iris-drive://app-key-link?app_key={}",
        normalize_pubkey(&linked.ui.profile.as_ref().unwrap().current_app_key_npub).unwrap()
    );
    config.save(&config_path).unwrap();

    let refreshed = linked_app.refresh();
    assert!(refreshed.error.is_empty(), "{}", refreshed.error);
    let saved = AppConfig::load_or_default(&config_path).unwrap();
    assert!(
        saved
            .profile
            .unwrap()
            .outbound_app_key_link_request
            .unwrap()
            .request_url
            .starts_with(iris_drive_core::app_key_link_transport::APP_KEY_APPROVAL_REQUEST_PREFIX)
    );
}

#[test]
fn refresh_recreates_missing_manual_join_request_url() {
    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::StartJoinRequest {
        app_key_label: "Mac waiting".to_owned(),
    });
    assert!(linked.error.is_empty(), "{}", linked.error);
    let linked_account = linked.ui.profile.expect("linked profile");
    assert!(
        linked_account
            .app_key_link_request
            .starts_with(iris_drive_core::app_key_link_transport::APP_KEY_APPROVAL_REQUEST_PREFIX)
    );

    let mut config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    config
        .profile
        .as_mut()
        .unwrap()
        .outbound_app_key_link_request = None;
    config.save(config_path_in(linked_dir.path())).unwrap();

    let refreshed = linked_app.dispatch(NativeAppAction::RefreshProfile);
    assert!(refreshed.error.is_empty(), "{}", refreshed.error);
    let refreshed_account = refreshed.ui.profile.expect("refreshed profile");
    assert!(
        refreshed_account
            .app_key_link_request
            .starts_with(iris_drive_core::app_key_link_transport::APP_KEY_APPROVAL_REQUEST_PREFIX)
    );
    let request = pending_request(linked_dir.path());
    assert_eq!(
        request.device_app_key_npub,
        linked_account.current_app_key_npub
    );
    assert!(!request.request_npub.is_empty());
    assert_eq!(request.request_secret.len(), 43);
    assert_ne!(request.device_app_key_npub, request.request_npub);
    let saved = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    assert!(
        saved
            .profile
            .as_ref()
            .unwrap()
            .outbound_app_key_link_request
            .as_ref()
            .is_some_and(|pending| pending.request_url == refreshed_account.app_key_link_request)
    );
}

#[test]
fn manual_join_request_approval_roster_authorizes_waiting_native_device_e2e() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    assert!(owner.error.is_empty(), "{}", owner.error);

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::StartJoinRequest {
        app_key_label: "iPhone".to_owned(),
    });
    assert!(linked.error.is_empty(), "{}", linked.error);
    let linked_account = linked.ui.profile.unwrap();
    let parsed_request = pending_request(linked_dir.path());
    assert_eq!(
        parsed_request.device_app_key_npub,
        linked_account.current_app_key_npub
    );
    assert!(!parsed_request.request_npub.is_empty());
    assert_eq!(parsed_request.request_secret.len(), 43);
    assert_ne!(
        parsed_request.device_app_key_npub,
        parsed_request.request_npub
    );

    let approved =
        approve_owner_from_pending_request(&owner_app, owner_dir.path(), linked_dir.path(), None);
    assert!(approved.error.is_empty(), "{}", approved.error);
    assert!(approved.ui.app_actors.iter().any(|device| {
        device.pubkey == linked_account.current_app_key_npub && device.role == "member"
    }));

    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let owner_state = owner_config.profile.as_ref().unwrap();
    let frame =
        iris_drive_core::app_key_link_transport::app_key_link_roster_frame(owner_state, 456)
            .expect("owner roster frame");
    let mut linked_config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    for receipt in &owner_state.pending_device_approval_receipts {
        let event = Event::from_json(&receipt.event_json).unwrap();
        iris_drive_core::relay_sync::apply_remote_device_approval_receipt_event(
            &mut linked_config,
            &event,
        )
        .unwrap();
    }
    let outcome = iris_drive_core::relay_sync::apply_app_key_link_roster_frame(
        &mut linked_config,
        &frame,
        &owner_state.app_key_pubkey,
    )
    .unwrap();

    assert!(matches!(
        outcome,
        iris_drive_core::relay_sync::AppKeyLinkRosterApply::Applied(
            iris_drive_core::app_keys::ApplyDecision::Adopted
        )
    ));
    let linked_state = linked_config.profile.as_ref().unwrap();
    assert_eq!(
        linked_state.authorization_state,
        AppKeyAuthorizationState::Authorized
    );
    assert_eq!(linked_state.profile_id, owner_state.profile_id);
    assert!(linked_state.outbound_app_key_link_request.is_none());
    linked_config
        .save(config_path_in(linked_dir.path()))
        .expect("save linked config");
    let refreshed = linked_app.refresh();
    assert!(refreshed.error.is_empty(), "{}", refreshed.error);
    let refreshed_account = refreshed.ui.profile.expect("linked profile");
    assert_eq!(
        refreshed_account.profile_id,
        owner_state.profile_id.to_string()
    );
    assert_eq!(refreshed_account.authorization_state, "authorized");
    assert!(refreshed_account.app_key_link_request.is_empty());
    assert!(refreshed.ui.setup_complete);
    assert!(!refreshed.ui.awaiting_approval);
}

#[test]
fn bound_receipt_then_roster_authorizes_unbound_waiting_native_device_e2e() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iOS owner".to_owned(),
    });
    assert!(owner.error.is_empty(), "{}", owner.error);

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::StartJoinRequest {
        app_key_label: "Waiting phone".to_owned(),
    });
    assert!(linked.error.is_empty(), "{}", linked.error);

    let approved =
        approve_owner_from_pending_request(&owner_app, owner_dir.path(), linked_dir.path(), None);
    assert!(approved.error.is_empty(), "{}", approved.error);
    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let owner_state = owner_config.profile.as_ref().unwrap();
    let receipt_event = Event::from_json(
        &owner_state
            .pending_device_approval_receipts
            .first()
            .expect("owner queued device approval receipt")
            .event_json,
    )
    .unwrap();
    let relay_events = owner_state
        .profile_roster_ops
        .iter()
        .map(|op| Event::from_json(&op.event_json).unwrap())
        .collect::<Vec<_>>();
    let mut linked_config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    let receipt_outcome =
        apply_native_app_key_link_relay_event_to_config(&mut linked_config, &receipt_event)
            .unwrap();
    assert_eq!(
        receipt_outcome,
        NativeAppKeyLinkRelayEventApply::AppliedRoster
    );
    for event in relay_events {
        apply_native_app_key_link_relay_event_to_config(&mut linked_config, &event).unwrap();
    }
    linked_config
        .save(config_path_in(linked_dir.path()))
        .expect("save linked config");

    let refreshed = linked_app.refresh();
    assert!(refreshed.error.is_empty(), "{}", refreshed.error);
    let refreshed_account = refreshed.ui.profile.expect("linked profile");
    assert_eq!(refreshed_account.authorization_state, "authorized");
    assert_eq!(
        refreshed_account.profile_id,
        owner_state.profile_id.to_string()
    );
    assert!(!refreshed.ui.awaiting_approval);
}

#[test]
fn native_owner_approval_selects_roster_ops_for_relay_publish() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iOS owner".to_owned(),
    });
    assert!(owner.error.is_empty(), "{}", owner.error);

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::StartJoinRequest {
        app_key_label: "Mac waiting".to_owned(),
    });
    assert!(linked.error.is_empty(), "{}", linked.error);
    let linked_account = linked.ui.profile.unwrap();

    let before = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let before_state = before.profile.as_ref().unwrap();
    let mut published = before_state
        .profile_roster_ops
        .iter()
        .map(|op| op.op_id.clone())
        .collect::<BTreeSet<_>>();

    let approved =
        approve_owner_from_pending_request(&owner_app, owner_dir.path(), linked_dir.path(), None);
    assert!(approved.error.is_empty(), "{}", approved.error);

    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let owner_state = owner_config.profile.as_ref().unwrap();
    let pending_ops = native_profile_roster_ops_pending_publish(owner_state, &published);
    assert!(
        pending_ops.len() >= 2,
        "approval should publish add-facet plus DCK rotation ops"
    );
    let linked_pubkey = normalize_pubkey(&linked_account.current_app_key_npub).unwrap();
    let relay_events = owner_state
        .profile_roster_ops
        .iter()
        .map(|op| Event::from_json(&op.event_json).unwrap())
        .collect::<Vec<_>>();
    let candidates =
        iris_drive_core::relay_sync::nostr_identity_app_key_approval_candidates_from_events(
            &linked_pubkey,
            &relay_events,
        )
        .unwrap();
    assert_eq!(candidates.len(), 1);

    published.extend(pending_ops.into_iter().map(|op| op.op_id));
    assert!(native_profile_roster_ops_pending_publish(owner_state, &published).is_empty());
}

#[test]
fn revoked_current_device_refresh_logs_out_and_allows_fresh_relink() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());

    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let owner_account = owner.ui.profile.unwrap();
    let owner_invite = owner_account.app_key_link_invite.clone();
    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: owner_invite.clone(),
        app_key_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.profile.unwrap();
    let linked_device = linked_account.current_app_key_npub.clone();
    let approved = approve_owner_from_pending_request(
        &owner_app,
        owner_dir.path(),
        linked_dir.path(),
        Some("Phone".to_owned()),
    );
    assert!(approved.error.is_empty(), "{}", approved.error);
    apply_latest_profile_roster_frame(owner_dir.path(), linked_dir.path());

    let authorized = linked_app.refresh();
    let account = authorized.ui.profile.as_ref().expect("account exists");
    assert_eq!(account.authorization_state, "authorized");
    assert!(authorized.ui.app_actors.iter().any(|device| {
        device.pubkey == linked_device && device.label == "Phone" && device.is_current_app_key
    }));
    assert!(authorized.ui.app_actors.iter().any(|device| {
        device.pubkey == owner_account.current_app_key_npub
            && device.label == "Mac"
            && !device.is_current_app_key
    }));

    let running = linked_app.dispatch(NativeAppAction::StartSync);
    assert!(running.ui.sync.running);

    let revoked = owner_app.dispatch(NativeAppAction::RevokeDevice {
        app_key_pubkey: linked_device.clone(),
    });
    assert!(revoked.error.is_empty(), "{}", revoked.error);
    apply_latest_profile_roster_frame(owner_dir.path(), linked_dir.path());

    let refreshed = linked_app.refresh();
    assert!(refreshed.error.is_empty(), "{}", refreshed.error);
    assert!(refreshed.ui.profile.is_none());
    assert!(refreshed.ui.app_actors.is_empty());
    assert!(refreshed.ui.roots.is_empty());
    assert!(refreshed.ui.snapshot_link.is_empty());
    assert!(!refreshed.ui.sync.running);
    assert_eq!(refreshed.ui.sync.status, "ready");
    assert_eq!(refreshed.ui.sync.status_label, "Ready");
    assert!(!linked_dir.path().join("key").exists());
    let linked_config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    assert!(linked_config.profile.is_none());

    let relinked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: owner_invite,
        app_key_label: "Phone".to_owned(),
    });
    let account = relinked.ui.profile.as_ref().expect("account exists");
    assert!(relinked.error.is_empty(), "{}", relinked.error);
    assert_eq!(account.authorization_state, "awaiting_approval");
    assert_ne!(account.current_app_key_npub, linked_device);
    assert_eq!(account.app_key_label, "Phone");
    assert!(
        account
            .app_key_link_request
            .starts_with(iris_drive_core::app_key_link_transport::APP_KEY_APPROVAL_REQUEST_PREFIX)
    );
}

fn apply_latest_profile_roster_frame(from: &Path, to: &Path) {
    let owner_config = AppConfig::load_or_default(config_path_in(from)).unwrap();
    let owner_state = owner_config.profile.as_ref().unwrap();
    let frame = iris_drive_core::app_key_link_transport::AppKeyLinkRosterFrame {
        schema: 1,
        profile_id: owner_state.profile_id,
        admin_app_key_pubkey: owner_state.app_key_pubkey.clone(),
        profile_roster_ops: owner_state.profile_roster_ops.clone(),
        sent_at: 123,
    };
    let mut linked_config = AppConfig::load_or_default(config_path_in(to)).unwrap();
    for receipt in &owner_state.pending_device_approval_receipts {
        let event = Event::from_json(&receipt.event_json).unwrap();
        iris_drive_core::relay_sync::apply_remote_device_approval_receipt_event(
            &mut linked_config,
            &event,
        )
        .unwrap();
    }
    iris_drive_core::relay_sync::apply_app_key_link_roster_frame(
        &mut linked_config,
        &frame,
        &owner_state.app_key_pubkey,
    )
    .unwrap();
    linked_config.save(config_path_in(to)).unwrap();
}
