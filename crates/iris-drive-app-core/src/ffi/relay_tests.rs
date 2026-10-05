use super::FfiApp;
use crate::NativeAppAction;
use iris_drive_core::AppConfig;
use iris_drive_core::paths::config_path_in;

#[test]
fn relay_actions_normalize_and_dedupe_urls() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.dispatch(NativeAppAction::AddRelay {
        url: " relay.example/ ".to_owned(),
    });

    assert_eq!(state.error.len(), 0);
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
    assert_eq!(state.error.len(), 0);
    assert!(!state.ui.relays.contains(&"wss://relay.example".to_owned()));
}

#[test]
fn relay_actions_wait_for_the_cross_process_config_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let disk_mutation =
        iris_drive_core::config_lock::ConfigMutationLock::acquire_blocking(dir.path()).unwrap();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();

    let action = std::thread::spawn(move || {
        let state = app.dispatch(NativeAppAction::AddRelay {
            url: "wss://transaction.example".to_owned(),
        });
        finished_tx.send(state).unwrap();
    });

    assert!(
        finished_rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err(),
        "relay mutation must not race a different config writer"
    );
    drop(disk_mutation);
    let state = finished_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    action.join().unwrap();
    assert!(state.error.is_empty(), "{}", state.error);
    assert!(
        state
            .ui
            .relays
            .contains(&"wss://transaction.example".to_owned())
    );
}

#[test]
fn blossom_actions_wait_for_the_cross_process_config_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let disk_mutation =
        iris_drive_core::config_lock::ConfigMutationLock::acquire_blocking(dir.path()).unwrap();
    let (finished_tx, finished_rx) = std::sync::mpsc::channel();

    let action = std::thread::spawn(move || {
        let state = app.dispatch(NativeAppAction::AddBlossomServer {
            url: "http://127.0.0.1:49152".to_owned(),
        });
        finished_tx.send(state).unwrap();
    });

    assert!(
        finished_rx
            .recv_timeout(std::time::Duration::from_millis(100))
            .is_err(),
        "Blossom mutation must not race a different config writer"
    );
    drop(disk_mutation);
    let state = finished_rx
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    action.join().unwrap();
    assert!(state.error.is_empty(), "{}", state.error);

    let saved = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    assert!(
        saved
            .blossom_servers
            .contains(&"http://127.0.0.1:49152".to_owned())
    );
}

#[test]
fn replace_relays_commits_one_normalized_exact_set() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.dispatch(NativeAppAction::ReplaceRelays {
        urls: vec![
            " relay.example/ ".to_owned(),
            "wss://relay.example".to_owned(),
            "ws://127.0.0.1:49152".to_owned(),
        ],
    });

    assert!(state.error.is_empty(), "{}", state.error);
    assert_eq!(
        state.ui.relays,
        vec![
            "wss://relay.example".to_owned(),
            "ws://127.0.0.1:49152".to_owned()
        ]
    );
    let saved = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    assert_eq!(saved.relays, state.ui.relays);
}

#[test]
fn replace_blossom_servers_commits_one_normalized_exact_set() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let state = app.dispatch(NativeAppAction::ReplaceBlossomServers {
        urls: vec![
            "http://127.0.0.1:49152/".to_owned(),
            "http://127.0.0.1:49152".to_owned(),
        ],
    });

    assert!(state.error.is_empty(), "{}", state.error);
    let saved = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    assert_eq!(saved.blossom_servers, ["http://127.0.0.1:49152"]);
    assert_eq!(
        saved
            .backup_targets
            .iter()
            .filter(|target| target.kind == iris_drive_core::BackupTargetKind::Blossom)
            .count(),
        1
    );
}
