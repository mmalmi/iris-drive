use super::FfiApp;
use super::tests::sharing_network::SharingTestServices;
use crate::NativeAppAction;
use crate::native_provider::work_pause::WorkPause;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn provider_refresh_keeps_reopened_app_and_device_roster_responsive() {
    edit_roster_during_work(NativeAppAction::Refresh, "provider");
}

#[test]
fn background_sync_keeps_reopened_app_and_device_roster_responsive() {
    edit_roster_during_work(NativeAppAction::RestartSync, "sync");
}

#[test]
fn sync_started_before_approval_preserves_its_durable_receipt() {
    let services = SharingTestServices::new();
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().display().to_string();
    let owner = FfiApp::new(data_dir.clone(), "test".into());
    let created = owner.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Owner".into(),
    });
    services.prepare(dir.path());
    let linked_dir = tempfile::tempdir().unwrap();
    let linked = FfiApp::new(linked_dir.path().display().to_string(), "test".into());
    let request = linked
        .dispatch(NativeAppAction::LinkDevice {
            link_target: created.ui.profile.unwrap().app_key_link_invite,
            app_key_label: "Phone".into(),
        })
        .ui
        .profile
        .unwrap()
        .app_key_link_request;
    let pause = services.pause_relay_queries();
    let sync_owner = owner.clone();
    let worker = std::thread::spawn(move || sync_owner.dispatch(NativeAppAction::RestartSync));
    pause.await_query();
    let (sent, received) = mpsc::channel();
    let approval = std::thread::spawn(move || {
        let result = owner.dispatch(NativeAppAction::ApproveDevice {
            request,
            label: String::new(),
        });
        sent.send(result).unwrap();
    });
    let responsive = received.recv_timeout(Duration::from_secs(1));
    drop(pause);
    worker.join().unwrap();
    approval.join().unwrap();
    let approved = responsive.expect("network sync blocked approval");
    assert!(approved.error.is_empty(), "{}", approved.error);
    assert_eq!(
        approved
            .ui
            .profile
            .unwrap()
            .pending_device_approval_receipt_count,
        1
    );
    let persisted = iris_drive_core::AppConfig::load_or_default(
        iris_drive_core::paths::config_path_in(dir.path()),
    )
    .unwrap();
    assert_eq!(
        persisted
            .profile
            .unwrap()
            .pending_device_approval_receipts
            .len(),
        1,
        "older network sync lost the durable approval receipt"
    );
}

fn edit_roster_during_work(action: NativeAppAction, kind: &'static str) {
    let services = SharingTestServices::new();
    let owner_dir = tempfile::tempdir().unwrap();
    let data_dir = owner_dir.path().display().to_string();
    let owner = FfiApp::new(data_dir.clone(), "test".into());
    let state = owner.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Android owner".into(),
    });
    services.prepare(owner_dir.path());
    let linked_dir = tempfile::tempdir().unwrap();
    let linked = FfiApp::new(linked_dir.path().display().to_string(), "test".into());
    let request = linked
        .dispatch(NativeAppAction::LinkDevice {
            link_target: state.ui.profile.unwrap().app_key_link_invite,
            app_key_label: "Phone".into(),
        })
        .ui
        .profile
        .unwrap()
        .app_key_link_request;

    let pause = WorkPause::new(&data_dir, kind);
    let worker = std::thread::spawn(move || owner.dispatch(action));
    pause.await_work();
    let (sent, received) = mpsc::channel();
    let foreground = std::thread::spawn(move || {
        let reopened = FfiApp::new(data_dir, "test".into());
        assert_eq!(reopened.state().ui.app_actors.len(), 1);
        let approved = reopened.dispatch(NativeAppAction::ApproveDevice {
            request,
            label: String::new(),
        });
        assert!(approved.error.is_empty(), "{}", approved.error);
        assert_eq!(approved.ui.app_actors.len(), 2);
        let phone = approved
            .ui
            .app_actors
            .iter()
            .find(|actor| actor.label == "Phone")
            .unwrap();
        let removed = reopened.dispatch(NativeAppAction::RevokeDevice {
            app_key_pubkey: phone.pubkey.clone(),
        });
        assert!(removed.error.is_empty(), "{}", removed.error);
        assert_eq!(removed.ui.app_actors.len(), 1);
        sent.send(removed).unwrap();
    });
    let responsive = received.recv_timeout(Duration::from_secs(1));
    // Release real work before asserting, so a regression cannot strand threads.
    drop(pause);
    let finished = worker.join().unwrap();
    foreground.join().unwrap();
    let removed =
        responsive.unwrap_or_else(|_| panic!("{kind} work blocked a local roster change"));
    assert_eq!(finished.ui.app_actors, removed.ui.app_actors);
    assert_eq!(
        finished.ui.profile.unwrap().current_key_epoch,
        removed.ui.profile.unwrap().current_key_epoch,
        "old work restored the key epoch from before removal"
    );
}

#[test]
fn removing_a_device_while_waiting_for_its_approval_ack() {
    let services = SharingTestServices::new();
    let dir = tempfile::tempdir().unwrap();
    let owner = FfiApp::new(dir.path().display().to_string(), "test".into());
    let state = owner.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Owner".into(),
    });
    services.prepare(dir.path());
    let linked_dir = tempfile::tempdir().unwrap();
    let linked = FfiApp::new(linked_dir.path().display().to_string(), "test".into());
    let phone = linked
        .dispatch(NativeAppAction::LinkDevice {
            link_target: state.ui.profile.unwrap().app_key_link_invite,
            app_key_label: "Phone".into(),
        })
        .ui
        .profile
        .unwrap();
    let approved = owner.dispatch(NativeAppAction::ApproveDevice {
        request: phone.app_key_link_request,
        label: "Phone".into(),
    });
    assert_eq!(
        approved
            .ui
            .profile
            .unwrap()
            .pending_device_approval_receipt_count,
        1
    );
    let pause = services.pause_relay_queries();
    let sync_owner = owner.clone();
    let worker = std::thread::spawn(move || sync_owner.dispatch(NativeAppAction::SyncApprovalAcks));
    pause.await_query();
    let (sent, received) = mpsc::channel();
    let remove = std::thread::spawn(move || {
        let removed = owner.dispatch(NativeAppAction::RevokeDevice {
            app_key_pubkey: phone.current_app_key_npub,
        });
        sent.send(removed).unwrap();
    });
    let result = received.recv_timeout(Duration::from_secs(1));
    drop(pause);
    let after_ack = worker.join().unwrap();
    remove.join().unwrap();
    let removed = result.expect("local revocation waited for a relay");
    assert!(removed.error.is_empty(), "{}", removed.error);
    assert_eq!(removed.ui.app_actors.len(), 1);
    assert_eq!(
        removed
            .ui
            .profile
            .unwrap()
            .pending_device_approval_receipt_count,
        0
    );
    assert_eq!(after_ack.ui.app_actors, removed.ui.app_actors);
}
