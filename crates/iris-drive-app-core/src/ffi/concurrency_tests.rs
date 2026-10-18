use super::FfiApp;
use super::tests::sharing_network::SharingTestServices;
use crate::NativeAppAction;
use crate::native_provider::work_pause::WorkPause;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn provider_refresh_keeps_reopened_app_and_device_approval_responsive() {
    approval_during_work(NativeAppAction::Refresh, "provider");
}

#[test]
fn background_sync_keeps_reopened_app_and_device_approval_responsive() {
    approval_during_work(NativeAppAction::RestartSync, "sync");
}

fn approval_during_work(action: NativeAppAction, kind: &'static str) {
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
        sent.send(approved).unwrap();
    });
    let responsive = received.recv_timeout(Duration::from_secs(1));
    // Release real work before asserting, so a regression cannot strand threads.
    drop(pause);
    let finished = worker.join().unwrap();
    foreground.join().unwrap();
    assert!(
        responsive.is_ok(),
        "{kind} work blocked reopened app state/approval"
    );
    assert_eq!(
        finished.ui.app_actors.len(),
        2,
        "old work replaced the approved roster"
    );
}
