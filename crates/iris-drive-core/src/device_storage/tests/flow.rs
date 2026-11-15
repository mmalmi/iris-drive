//! A device measures what it holds, tells the profile's other devices, and
//! takes in what they tell it, end to end over a real profile and Drive.

use nostr_sdk::Keys;
use tempfile::tempdir;

use super::super::reporter::DeviceStorageReporter;
use super::super::rows::device_storage_view;
use super::super::*;
use crate::Daemon;
use crate::config::{AppConfig, Drive};
use crate::paths::config_path_in;

fn own_published_again(config: &mut AppConfig, own: &str) {
    let drive = config
        .drives
        .iter_mut()
        .find(|drive| drive.drive_id == crate::PRIMARY_DRIVE_ID)
        .unwrap();
    drive.app_key_roots.get_mut(own).unwrap().local_only = false;
}

fn now() -> u64 {
    super::super::unix_now()
}

#[tokio::test]
async fn a_device_reports_what_it_holds_and_shows_what_the_others_hold() {
    let dir = tempdir().unwrap();
    let mut account = crate::profile::Profile::create(dir.path(), Some("Mac".into())).unwrap();
    let web = Keys::generate();
    let web_hex = web.public_key().to_hex();
    account
        .approve_app_key(&web_hex, Some("Browser".into()))
        .unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(account.state.root_scope_id()));
    config.save(config_path_in(dir.path())).unwrap();
    let work = tempdir().unwrap();
    std::fs::create_dir(work.path().join("Photos")).unwrap();
    std::fs::write(work.path().join("Photos/trip.jpg"), vec![7_u8; 300_000]).unwrap();
    std::fs::write(work.path().join("notes.txt"), b"hello").unwrap();
    Daemon::open(dir.path())
        .unwrap()
        .import_source_dir(work.path())
        .await
        .unwrap();
    let profile = account.state.root_scope_id();

    let mut reporter = DeviceStorageReporter::new(dir.path());
    reporter.measure_now();
    let event = reporter
        .due_event()
        .await
        .unwrap()
        .expect("the first report is sent");
    let first_at = event.created_at.as_secs();
    let sent = open_device_storage_event(&event, &profile, &web).unwrap();
    assert_eq!((sent.files, sent.file_bytes), (2, 300_005));
    assert_eq!((sent.held_files, sent.held_bytes), (2, 300_005));
    assert_eq!(sent.app, report::current_app());
    assert!(sent.stored_bytes > 0);
    assert!(
        sent.capacity_bytes
            .is_some_and(|capacity| capacity > sent.stored_bytes)
    );
    assert_eq!(
        sent.loose,
        Some(DeviceStorageFiles {
            files: 1,
            bytes: 5,
            held: 5
        })
    );
    assert_eq!(
        sent.folders,
        Some(vec![DeviceStorageFolder {
            path: vec!["Photos".into()],
            bytes: 300_000,
            held: 300_000
        }])
    );
    assert_eq!(sent.seen, Some(std::collections::BTreeMap::new()));

    // Measured again with nothing moved: nothing to send.
    reporter.measure_now();
    assert!(reporter.due_event().await.unwrap().is_none());

    // The browser's report arrives and shows; one from a stranger does not.
    let config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    let theirs = read_device_storage_report(&serde_json::json!({
        "v": 1, "measured_at": now(), "app": "web", "root": "ab".repeat(32),
        "files": 2, "file_bytes": 300_005, "held_files": 1, "held_bytes": 5,
        "stored_bytes": 9000, "capacity_bytes": 1_000_000_000_u64, "seen": { account.state.app_key_pubkey.clone(): 1 },
    }))
    .unwrap();
    let devices = profile_devices(&config, &account.state);
    let from_web = build_device_storage_event(&web, &profile, &theirs, &devices, now()).unwrap();
    let applied = apply_device_storage_event(dir.path(), &config, &from_web).unwrap();
    assert_eq!(applied.outcome, DeviceStorageApply::Applied);
    assert!(!applied.sender_not_told);
    assert_eq!(
        apply_device_storage_event(dir.path(), &config, &from_web)
            .unwrap()
            .outcome,
        DeviceStorageApply::Current
    );
    let stranger = Keys::generate();
    let from_stranger =
        build_device_storage_event(&stranger, &profile, &theirs, &devices, now()).unwrap();
    assert_eq!(
        apply_device_storage_event(dir.path(), &config, &from_stranger)
            .unwrap()
            .outcome,
        DeviceStorageApply::Unauthorized
    );

    let state = &config.profile.clone().unwrap();
    let view = device_storage_view(dir.path(), &config, state, |_| None, |_| false, now());
    assert_eq!(
        view.summary,
        "Your Drive is 293.0 KB in 2 files. All of it is on this device."
    );
    let rows = view
        .devices
        .iter()
        .map(|row| {
            (
                row.is_current,
                row.held_label.as_str(),
                row.sync_label.as_str(),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        rows,
        vec![
            (true, "All of it", ""),
            (false, "5 B · under 1%", "In sync")
        ]
    );

    // A local bookkeeping root of this device was never sent: the browser cannot have it.
    let mut config = config;
    let drive = config
        .drives
        .iter_mut()
        .find(|drive| drive.drive_id == crate::PRIMARY_DRIVE_ID)
        .unwrap();
    let own = drive
        .app_key_roots
        .get_mut(&account.state.app_key_pubkey)
        .unwrap();
    own.app_key_seq = 9;
    own.local_only = true;
    let view = device_storage_view(dir.path(), &config, state, |_| None, |_| false, now());
    assert_eq!(view.devices[1].sync_label, "In sync");
    own_published_again(&mut config, &account.state.app_key_pubkey);
    let view = device_storage_view(dir.path(), &config, state, |_| None, |_| false, now());
    assert_eq!(view.devices[1].sync_label, "Missing recent changes");
    assert_eq!(
        view.devices[1].space_label,
        "Uses 8.8 KB of 953.7 MB allowed"
    );
    assert_eq!(view.devices[1].icon, "web");

    // A device linked after the last report is told at once.
    let phone = Keys::generate();
    account
        .approve_app_key(&phone.public_key().to_hex(), Some("Phone".into()))
        .unwrap();
    let mut config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    config.profile = Some(account.state.clone());
    config.save(config_path_in(dir.path())).unwrap();
    let event = reporter
        .due_event()
        .await
        .unwrap()
        .expect("sent again to the new device");
    assert!(open_device_storage_event(&event, &profile, &phone).is_ok());
    // Relays keep the later of two reports, also within one second.
    assert!(event.created_at.as_secs() > first_at);
}
