use std::path::Path;

use iris_drive_core::paths::config_path_in;
use iris_drive_core::{AppConfig, AppKeyAuthorizationState};
use nostr_sdk::{Event, JsonUtil};

use super::mobile_fips_status::{
    NATIVE_FIPS_STATUS_STABLE_WRITE_MIN_SECS, native_app_key_link_exchange_should_run,
    native_fips_status_write_is_due, write_native_fips_status_value,
};
use super::{
    APP_KEY_LINK_EXCHANGE_ACTIVE_TICK_MILLIS, APP_KEY_LINK_EXCHANGE_IDLE_TICK_MILLIS, FfiApp,
    NATIVE_FIPS_STATUS_FRESH_SECS, NativeAppConfigCache, app_key_link_exchange_tick_millis,
};
use crate::NativeAppAction;

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
        .unwrap()
        .0
}

fn approve_owner_from_pending_request(owner_dir: &Path, linked_dir: &Path, label: Option<String>) {
    let bootstrap = pending_request(linked_dir);
    let config_path = config_path_in(owner_dir);
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    let state = config.profile.clone().expect("owner profile");
    let mut account = iris_drive_core::Profile::load(state, owner_dir).unwrap();
    account.approve_device_bootstrap(&bootstrap, label).unwrap();
    config.profile = Some(account.state);
    config.save(&config_path).unwrap();
}

#[test]
fn mobile_app_key_link_exchange_stays_on_for_authorized_or_awaiting_approval() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let owner_profile = owner.ui.profile.expect("owner profile");

    let phone_dir = tempfile::tempdir().unwrap();
    let phone_app = FfiApp::new(phone_dir.path().display().to_string(), "test".to_owned());
    let linked = phone_app.dispatch(NativeAppAction::LinkDevice {
        link_target: owner_profile.app_key_link_invite.clone(),
        app_key_label: "Phone".to_owned(),
    });
    assert!(linked.error.is_empty(), "{}", linked.error);
    approve_owner_from_pending_request(
        owner_dir.path(),
        phone_dir.path(),
        Some("Phone".to_owned()),
    );
    apply_owner_approval_receipt_to_linked_config(owner_dir.path(), phone_dir.path());
    let receipt_only = AppConfig::load_or_default(config_path_in(phone_dir.path())).unwrap();
    assert_eq!(
        receipt_only
            .profile
            .as_ref()
            .map(|profile| profile.authorization_state),
        Some(AppKeyAuthorizationState::Authorized)
    );
    assert!(native_app_key_link_exchange_should_run(&receipt_only));

    apply_owner_profile_roster_to_linked_config(owner_dir.path(), phone_dir.path());
    mark_daemon_live(phone_dir.path());

    let source = phone_dir.path().join("idle-note.txt");
    std::fs::write(&source, b"idle bytes").unwrap();
    let data_dir = phone_dir.path().display().to_string();
    assert!(
        super::native_provider_mkdir_json(&data_dir, "Photos")["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
    assert!(
        super::native_provider_write_json(
            &data_dir,
            "Photos/idle-note.txt",
            &source.display().to_string(),
        )["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    let running = phone_app.dispatch(NativeAppAction::StartSync);
    let profile = running.ui.profile.as_ref().expect("phone profile");
    assert_eq!(profile.authorization_state, "authorized");
    assert!(running.ui.setup_complete);
    assert!(running.ui.sync.running);
    assert_eq!(running.ui.file_count, 1);
    assert_eq!(running.ui.provider_directory_paths, vec!["Photos"]);
    assert!(running.ui.app_actors.iter().any(|device| {
        device.is_current_app_key && device.label == "Phone" && device.state == "Linked"
    }));
    assert!(running.ui.app_actors.iter().any(|device| {
        !device.is_current_app_key && device.label == "Mac" && device.state == "Linked"
    }));

    let config = AppConfig::load_or_default(config_path_in(phone_dir.path())).unwrap();
    assert!(native_app_key_link_exchange_should_run(&config));

    let awaiting_approval = native_exchange_config(AppKeyAuthorizationState::AwaitingApproval);
    assert!(native_app_key_link_exchange_should_run(&awaiting_approval));

    assert!(!native_app_key_link_exchange_should_run(
        &AppConfig::default()
    ));
}

#[test]
fn native_app_key_link_config_cache_reports_only_real_changes() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = config_path_in(dir.path());
    let mut cache = NativeAppConfigCache::default();
    AppConfig::default().save(&config_path).unwrap();

    let (_, first_changed) = cache.load_with_change(dir.path()).unwrap();
    let (_, second_changed) = cache.load_with_change(dir.path()).unwrap();

    assert!(first_changed);
    assert!(!second_changed);
}

#[test]
fn app_key_link_exchange_uses_fast_ticks_only_while_approval_is_pending() {
    let owner_dir = tempfile::tempdir().unwrap();
    let linked_dir = tempfile::tempdir().unwrap();
    let owner = iris_drive_core::Profile::create(owner_dir.path(), Some("Mac".into())).unwrap();
    let mut linked = iris_drive_core::Profile::link_to_profile(
        linked_dir.path(),
        owner.state.profile_id,
        owner.state.app_key_pubkey.clone(),
        Some("Phone".into()),
    )
    .unwrap();
    let approval_request =
        iris_drive_core::app_key_link_transport::create_app_key_approval_bootstrap(
            linked.app_key.keys(),
            linked.state.app_key_label.as_deref(),
        )
        .unwrap();
    linked
        .state
        .queue_outbound_app_key_link_request(
            owner.state.app_key_pubkey.clone(),
            &iris_drive_core::app_key_link_invite_pubkey(&owner.state.app_key_link_secret).unwrap(),
            123,
            approval_request.url,
            approval_request.request_keys.secret_key().to_secret_hex(),
        )
        .unwrap();

    assert_eq!(
        app_key_link_exchange_tick_millis(Some(&linked.state)),
        APP_KEY_LINK_EXCHANGE_ACTIVE_TICK_MILLIS
    );
    assert_eq!(
        app_key_link_exchange_tick_millis(Some(&owner.state)),
        APP_KEY_LINK_EXCHANGE_IDLE_TICK_MILLIS
    );
    assert_eq!(
        app_key_link_exchange_tick_millis(None),
        APP_KEY_LINK_EXCHANGE_IDLE_TICK_MILLIS
    );
}

#[test]
fn mobile_native_fips_status_suppresses_volatile_rewrites_until_heartbeat() {
    assert!(
        NATIVE_FIPS_STATUS_FRESH_SECS > NATIVE_FIPS_STATUS_STABLE_WRITE_MIN_SECS,
        "the stable status heartbeat must refresh before UI freshness expires"
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(super::NATIVE_FIPS_STATUS_FILE_NAME);
    let now = super::unix_now_seconds();
    let first = native_fips_status_test_value(now, &["peer-a"], 10, 25);
    write_native_fips_status_value(dir.path(), &first).unwrap();

    let volatile_only = native_fips_status_test_value(now + 1, &["peer-a"], 42, 80);
    assert!(!native_fips_status_write_is_due(
        &path,
        &volatile_only,
        now + 1
    ));
    assert!(native_fips_status_write_is_due(
        &path,
        &volatile_only,
        now + NATIVE_FIPS_STATUS_STABLE_WRITE_MIN_SECS
    ));

    let changed_peers = native_fips_status_test_value(now + 2, &["peer-a", "peer-b"], 42, 80);
    assert!(native_fips_status_write_is_due(
        &path,
        &changed_peers,
        now + 2
    ));
}

fn native_exchange_config(authorization_state: AppKeyAuthorizationState) -> AppConfig {
    let dir = tempfile::tempdir().unwrap();
    let mut profile = iris_drive_core::Profile::create(dir.path(), Some("Phone".to_owned()))
        .unwrap()
        .state;
    profile.authorization_state = authorization_state;
    AppConfig {
        profile: Some(profile),
        ..AppConfig::default()
    }
}

fn apply_owner_profile_roster_to_linked_config(owner_dir: &Path, linked_dir: &Path) {
    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir)).unwrap();
    let owner_state = owner_config.profile.as_ref().unwrap();
    let roster_frame = iris_drive_core::app_key_link_transport::AppKeyLinkRosterFrame {
        schema: 1,
        profile_id: owner_state.profile_id,
        admin_app_key_pubkey: owner_state.app_key_pubkey.clone(),
        profile_roster_ops: owner_state.profile_roster_ops.clone(),
        sent_at: 123,
    };
    let mut linked_config = AppConfig::load_or_default(config_path_in(linked_dir)).unwrap();
    iris_drive_core::relay_sync::apply_app_key_link_roster_frame(
        &mut linked_config,
        &roster_frame,
        &owner_state.app_key_pubkey,
    )
    .unwrap();
    linked_config.save(config_path_in(linked_dir)).unwrap();
}

fn apply_owner_approval_receipt_to_linked_config(owner_dir: &Path, linked_dir: &Path) {
    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir)).unwrap();
    let owner_state = owner_config.profile.as_ref().unwrap();
    let mut linked_config = AppConfig::load_or_default(config_path_in(linked_dir)).unwrap();
    for receipt in &owner_state.pending_device_approval_receipts {
        let event = Event::from_json(&receipt.event_json).unwrap();
        iris_drive_core::relay_sync::apply_remote_device_approval_receipt_event(
            &mut linked_config,
            &event,
        )
        .unwrap();
    }
    linked_config.save(config_path_in(linked_dir)).unwrap();
}

fn mark_daemon_live(config_dir: &Path) {
    std::fs::write(
        iris_drive_core::daemon_liveness::daemon_lock_path(config_dir),
        format!("{}\n", std::process::id()),
    )
    .unwrap();
}

fn native_fips_status_test_value(
    updated_at: u64,
    peers: &[&str],
    bytes_recv: u64,
    srtt_ms: u64,
) -> serde_json::Value {
    serde_json::json!({
        "running": true,
        "fresh": true,
        "updated_at": updated_at,
        "state": "running",
        "state_label": "Connected",
        "direct_devices": peers,
        "direct_peers": peers,
        "connected_peers": peers,
        "online_devices": peers,
        "online_peers": peers,
        "mesh_devices": [],
        "mesh_peers": [],
        "peer_statuses": peers.iter().map(|peer| serde_json::json!({
            "npub": peer,
            "transport_type": "direct",
            "bytes_recv": bytes_recv,
            "bytes_sent": bytes_recv + 1,
            "packets_recv": bytes_recv + 2,
            "packets_sent": bytes_recv + 3,
            "srtt_ms": srtt_ms,
            "connection_label": format!("{srtt_ms} ms"),
        })).collect::<Vec<_>>(),
        "error": null,
    })
}
