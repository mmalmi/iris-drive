use super::{
    DirectFipsDownloadDecision, NetworkSyncOptions, direct_fips_download_decision,
    drive_root_recipient_app_key_pubkeys, should_materialize_after_sync,
};
use crate::config::{AppConfig, Drive};
use crate::profile::Profile;
use nostr_sdk::Keys;
use tempfile::tempdir;

#[test]
fn default_sync_options_preserve_existing_temporary_fips_download() {
    assert!(NetworkSyncOptions::default().start_direct_fips_download);
    assert_eq!(
        direct_fips_download_decision(false, NetworkSyncOptions::default()),
        DirectFipsDownloadDecision::StartTemporary
    );
}

#[test]
fn sync_options_can_skip_only_temporary_direct_fips_download() {
    let options = NetworkSyncOptions::without_direct_fips_download();

    assert_eq!(
        direct_fips_download_decision(false, options),
        DirectFipsDownloadDecision::SkipTemporary
    );
    assert_eq!(
        direct_fips_download_decision(true, options),
        DirectFipsDownloadDecision::UseSupplied
    );
}

#[test]
fn applied_remote_roots_request_materialization_even_without_peer_root() {
    let config = AppConfig::default();

    assert!(should_materialize_after_sync(&config, &[], true));
    assert!(!should_materialize_after_sync(&config, &[], false));
}

#[test]
fn drive_root_recipients_use_full_cached_app_key_projection() {
    let dir = tempdir().unwrap();
    let mut profile = Profile::create(dir.path(), Some("Owner".into())).unwrap();
    let phone = Keys::generate().public_key().to_hex();
    let tablet = Keys::generate().public_key().to_hex();
    profile
        .approve_app_key(&phone, Some("Phone".into()))
        .unwrap();
    profile
        .approve_app_key(&tablet, Some("Tablet".into()))
        .unwrap();
    let expected = profile
        .state
        .current_app_keys_projection()
        .unwrap()
        .app_actors
        .iter()
        .map(|actor| actor.pubkey.clone())
        .collect::<Vec<_>>();

    let mut partial_state = profile.state.clone();
    partial_state.profile_roster_ops.truncate(1);
    partial_state.profile_roster_projection = None;
    assert_eq!(
        partial_state
            .current_app_keys_projection()
            .unwrap()
            .app_actors
            .len(),
        expected.len()
    );

    let drive = Drive::primary(partial_state.root_scope_id());
    let recipients = drive_root_recipient_app_key_pubkeys(&partial_state, &drive);

    assert_eq!(recipients, expected);
}
