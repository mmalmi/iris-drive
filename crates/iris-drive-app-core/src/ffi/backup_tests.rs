use super::FfiApp;
use crate::NativeAppAction;
use iris_drive_core::paths::config_path_in;
use iris_drive_core::{
    AppConfig, BackupTarget, BackupTargetCheck, BackupTargetKind, BackupTargetSync,
};

#[test]
fn configured_backup_targets_use_shared_summary_rows() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Pixel".to_owned(),
    });

    let config_path = config_path_in(dir.path());
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    config.backup_targets = vec![BackupTarget {
        id: "backup-1".to_owned(),
        kind: BackupTargetKind::Blossom,
        target: "https://backup.example".to_owned(),
        label: Some("Archive".to_owned()),
        enabled: true,
        last_sync: Some(BackupTargetSync {
            state: "uploading".to_owned(),
            root_cid: "root".to_owned(),
            synced_at: 1_700_000_000,
            total_hashes: 5,
            uploaded: 2,
            already_present: 1,
        }),
        last_check: Some(BackupTargetCheck {
            state: "verified".to_owned(),
            root_cid: "root".to_owned(),
            checked_at: 1_700_000_100,
            total_hashes: 5,
            sample_size: 5,
            sampled_hashes: 5,
            present: 5,
            missing: 0,
            unknown: 0,
            latency_ms: Some(35),
            download_bytes: Some(2048),
            download_ms: Some(1000),
            download_bytes_per_second: Some(2048),
            error: None,
        }),
    }];
    config.save(&config_path).unwrap();

    let state = app.refresh();
    let backup = state
        .ui
        .backups
        .iter()
        .find(|backup| backup.label == "Archive")
        .expect("configured backup target should be exposed through app-core");

    assert_eq!(backup.state, "uploading");
    assert_eq!(backup.id, "backup-1");
    assert_eq!(backup.kind, "blossom");
    assert_eq!(backup.target, "https://backup.example");
    assert_eq!(backup.configured_label, "Archive");
    assert!(backup.enabled);
    assert_eq!(
        backup.detail,
        "https://backup.example | 2/5 | check verified | 35 ms | 2.0 KB/s"
    );
}

#[test]
fn backup_actions_manage_blossom_targets_through_app_core() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Pixel".to_owned(),
    });

    let added = app.dispatch(NativeAppAction::AddBackupTarget {
        target: " https://backup.example/ ".to_owned(),
        label: " Archive ".to_owned(),
    });
    assert!(added.error.is_empty(), "{}", added.error);
    let backup = added
        .ui
        .backups
        .iter()
        .find(|backup| backup.target == "https://backup.example")
        .expect("new backup row should be exposed");
    assert_eq!(backup.id, "blossom:https://backup.example");
    assert_eq!(backup.kind, "blossom");
    assert_eq!(backup.label, "Archive");
    assert_eq!(backup.configured_label, "Archive");

    let config_path = config_path_in(dir.path());
    let saved = AppConfig::load_or_default(&config_path).unwrap();
    assert!(
        saved
            .blossom_servers
            .contains(&"https://backup.example".to_owned())
    );

    let checked = app.dispatch(NativeAppAction::CheckBackups {
        target: "https://backup.example".to_owned(),
    });
    assert!(
        checked
            .error
            .contains("no current drive root; import files first"),
        "{}",
        checked.error
    );

    let removed = app.dispatch(NativeAppAction::RemoveBackupTarget {
        target: "https://backup.example".to_owned(),
    });
    assert!(removed.error.is_empty(), "{}", removed.error);
    assert!(
        !removed
            .ui
            .backups
            .iter()
            .any(|backup| backup.target == "https://backup.example")
    );
    let saved = AppConfig::load_or_default(&config_path).unwrap();
    assert!(
        !saved
            .blossom_servers
            .contains(&"https://backup.example".to_owned())
    );

    let blossom = app.dispatch(NativeAppAction::AddBlossomServer {
        url: "https://mirror.example/".to_owned(),
    });
    assert!(blossom.error.is_empty(), "{}", blossom.error);
    assert!(
        blossom
            .ui
            .backups
            .iter()
            .any(|backup| backup.target == "https://mirror.example")
    );

    let blossom_removed = app.dispatch(NativeAppAction::RemoveBlossomServer {
        url: "https://mirror.example".to_owned(),
    });
    assert!(
        !blossom_removed
            .ui
            .backups
            .iter()
            .any(|backup| backup.target == "https://mirror.example")
    );
}
