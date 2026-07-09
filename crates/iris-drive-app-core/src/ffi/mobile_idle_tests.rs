use std::path::Path;

use iris_drive_core::paths::config_path_in;
use iris_drive_core::{AppConfig, AppKeyAuthorizationState};
use nostr_sdk::{Event, JsonUtil};

use super::FfiApp;
use super::mobile_fips_status::{
    NATIVE_FIPS_STATUS_STABLE_WRITE_MIN_SECS, native_app_key_link_exchange_should_run,
    native_fips_status_write_is_due, write_native_fips_status_value,
};
use crate::NativeAppAction;

#[test]
fn mobile_app_key_link_exchange_runs_only_for_rostered_idle_state_or_approval() {
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
    let linked_profile = linked.ui.profile.expect("linked phone profile");
    let approved = owner_app.dispatch(NativeAppAction::ApproveDevice {
        request: linked_profile.app_key_link_request,
        label: "Phone".to_owned(),
    });
    assert!(approved.error.is_empty(), "{}", approved.error);
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
    assert!(native_app_key_link_exchange_should_run(&config, true));
    assert!(!native_app_key_link_exchange_should_run(&config, false));

    let awaiting_approval = native_exchange_config(AppKeyAuthorizationState::AwaitingApproval);
    assert!(native_app_key_link_exchange_should_run(
        &awaiting_approval,
        false
    ));

    assert!(!native_app_key_link_exchange_should_run(
        &AppConfig::default(),
        true
    ));
}

#[test]
fn mobile_native_fips_status_suppresses_volatile_rewrites_until_heartbeat() {
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
        &roster_frame,
        &owner_state.app_key_pubkey,
    )
    .unwrap();
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
