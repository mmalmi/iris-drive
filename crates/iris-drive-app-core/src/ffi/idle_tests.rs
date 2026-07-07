use super::FfiApp;

#[test]
fn fresh_logged_out_runtime_reports_sync_ready() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.state();

    assert!(state.ui.profile.is_none());
    assert!(!state.ui.setup_complete);
    assert!(!state.ui.awaiting_approval);
    assert!(!state.ui.sync.running);
    assert_eq!(state.ui.sync.status, "ready");
    assert_eq!(state.ui.sync.status_label, "Ready");
}
