#[allow(dead_code)]
#[path = "../../iris-drive-cli/tests/support/mod.rs"]
mod support;

use iris_drive_app_core::{FfiApp, NativeAppAction, generate_recovery_key};
use iris_drive_core::{
    AppConfig, Daemon, Drive, PRIMARY_DRIVE_ID, Profile, drive_root_key_recipient_pubkeys,
    nostr_events::parse_drive_root_event_for_device,
    paths::{config_path_in, recovery_phrase_path_in},
    primary_merged_view,
    recovery_acceptance::queued_facet_acceptances,
    recovery_phrase::{generate_recovery_phrase, recovery_phrase_to_keys},
    relay_sync,
};
use std::time::{Duration, Instant};
use support::{LocalBlossomServer, LocalNostrRelay, add_config_relay, configure_local_blossom};
use tempfile::tempdir;

/// The only device with the files publishes them and is lost. A new install
/// with just the recovery phrase finds the account and gets the files back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(clippy::too_many_lines)]
async fn recovery_phrase_alone_brings_back_the_files() {
    let relay = LocalNostrRelay::spawn().await;
    let blossom = LocalBlossomServer::spawn().await;
    let phrase = generate_recovery_phrase().unwrap();
    let recovery_keys = recovery_phrase_to_keys(&phrase).unwrap();

    // The lost laptop: it added the phrase and published one file.
    let laptop_dir = tempdir().unwrap();
    let mut laptop = Profile::create(laptop_dir.path(), Some("Laptop".into())).unwrap();
    laptop.add_recovery_phrase(&phrase).unwrap();
    let mut config = AppConfig {
        profile: Some(laptop.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(laptop.state.root_scope_id()));
    config.save(config_path_in(laptop_dir.path())).unwrap();
    add_config_relay(laptop_dir.path(), &relay.url);
    configure_local_blossom(laptop_dir.path(), &blossom.url);
    let files = tempdir().unwrap();
    std::fs::write(files.path().join("notes.txt"), b"hello").unwrap();
    let mut daemon = Daemon::open(laptop_dir.path()).unwrap();
    let import = daemon.import_source_dir(files.path()).await.unwrap();
    let laptop_root = import.root_cid.clone();
    let blossom_client = iris_drive_core::blossom_sync_client(
        laptop.app_key.keys().clone(),
        std::slice::from_ref(&blossom.url),
    );
    iris_drive_core::blossom_sync::upload_tree(
        daemon.tree(),
        &hashtree_core::Cid::parse(&laptop_root).unwrap(),
        &blossom_client,
    )
    .await
    .unwrap();
    let config = AppConfig::load_or_default(config_path_in(laptop_dir.path())).unwrap();
    let drive = config.drive(PRIMARY_DRIVE_ID).unwrap();
    let client = relay_sync::connect(std::slice::from_ref(&relay.url))
        .await
        .unwrap();
    relay_sync::publish_nostr_identity_roster_ops(&client, &laptop.state.profile_roster_ops)
        .await
        .unwrap();
    relay_sync::publish_drive_root(
        laptop_dir.path(),
        &client,
        laptop.app_key.keys(),
        &laptop.state.root_scope_id(),
        PRIMARY_DRIVE_ID,
        &drive.app_key_roots[&laptop.state.app_key_pubkey],
        &drive_root_key_recipient_pubkeys(&laptop.state, drive),
    )
    .await
    .unwrap();
    drop(daemon);

    // A new phone with the phrase alone.
    let phone_dir = tempdir().unwrap();
    add_config_relay(phone_dir.path(), &relay.url);
    configure_local_blossom(phone_dir.path(), &blossom.url);
    let phone_path = phone_dir.path().to_owned();
    let restore_phrase = phrase.clone();
    let state = tokio::task::spawn_blocking(move || {
        FfiApp::new(phone_path.display().to_string(), "test".into()).dispatch(
            NativeAppAction::RestoreProfile {
                recovery_secret: restore_phrase,
                app_key_label: "Phone".into(),
            },
        )
    })
    .await
    .unwrap();
    assert!(state.error.is_empty(), "{}", state.error);
    assert!(state.ui.setup_complete);
    let config = AppConfig::load_or_default(config_path_in(phone_dir.path())).unwrap();
    let phone = config.profile.clone().unwrap();
    assert_eq!(phone.profile_id, laptop.state.profile_id);
    assert_ne!(phone.app_key_pubkey, laptop.state.app_key_pubkey);
    assert!(phone.can_write_roots());
    assert!(!recovery_phrase_path_in(phone_dir.path()).exists());
    assert_eq!(
        config.drive(PRIMARY_DRIVE_ID).unwrap().app_key_roots[&laptop.state.app_key_pubkey]
            .root_cid,
        laptop_root
    );

    // In the background the phone downloads the files and publishes its own
    // root, wrapped for itself and the phrase.
    let deadline = Instant::now() + Duration::from_secs(30);
    let phone_root = loop {
        let config = AppConfig::load_or_default(config_path_in(phone_dir.path())).unwrap();
        if let Some(root) = config
            .drive(PRIMARY_DRIVE_ID)
            .and_then(|drive| drive.app_key_roots.get(&phone.app_key_pubkey))
            .filter(|root| !root.local_only)
        {
            break root.clone();
        }
        assert!(
            Instant::now() < deadline,
            "the phone never made its own root"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    let daemon = Daemon::open(phone_dir.path()).unwrap();
    let merged = primary_merged_view(daemon.tree(), daemon.config())
        .await
        .unwrap();
    let notes = merged
        .view
        .files
        .iter()
        .find(|entry| entry.path == "notes.txt")
        .expect("notes.txt came back");
    assert_eq!(notes.size, 5);

    let published = relay_sync::fetch_drive_roots(
        &client,
        &phone.root_scope_id(),
        PRIMARY_DRIVE_ID,
        std::slice::from_ref(&phone.app_key_pubkey),
        Duration::from_secs(5),
    )
    .await
    .unwrap();
    let (_, _, _, opened) = parse_drive_root_event_for_device(&published[0], &recovery_keys)
        .expect("the phone's root is wrapped for the phrase");
    assert_eq!(opened.root_cid, phone_root.root_cid);
    relay_sync::shutdown_client(&client).await;
}

/// A phrase no account uses still makes a new account with it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_unknown_phrase_makes_a_new_account() {
    let relay = LocalNostrRelay::spawn().await;
    let dir = tempdir().unwrap();
    add_config_relay(dir.path(), &relay.url);
    let phrase = generate_recovery_phrase().unwrap();
    let path = dir.path().to_owned();
    let state = tokio::task::spawn_blocking(move || {
        FfiApp::new(path.display().to_string(), "test".into()).dispatch(
            NativeAppAction::RestoreProfile {
                recovery_secret: phrase,
                app_key_label: String::new(),
            },
        )
    })
    .await
    .unwrap();
    assert!(state.error.is_empty(), "{}", state.error);
    assert!(state.ui.setup_complete);
}

/// Adding a phrase the app just showed signs the acceptance the web's phrase
/// sign-in looks for.
#[test]
fn adding_a_shown_phrase_queues_its_acceptance() {
    let dir = tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".into());
    let created = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Laptop".into(),
    });
    assert!(created.error.is_empty(), "{}", created.error);
    let shown = generate_recovery_key();
    let phrase = shown.words.join(" ");
    let added = app.dispatch(NativeAppAction::AddRecoveryDevice {
        recovery_pubkey: shown.recovery_pubkey.clone(),
    });
    assert!(added.error.is_empty(), "{}", added.error);
    let queued = queued_facet_acceptances(dir.path());
    assert_eq!(queued.len(), 1);
    assert_eq!(
        queued[0].pubkey,
        recovery_phrase_to_keys(&phrase).unwrap().public_key()
    );
    let acceptance =
        iris_drive_core::parse_nostr_identity_facet_acceptance_event(&queued[0]).unwrap();
    let profile = AppConfig::load_or_default(config_path_in(dir.path()))
        .unwrap()
        .profile
        .unwrap();
    assert!(acceptance.is_active_in_roster(&profile.profile_projection()));
}
