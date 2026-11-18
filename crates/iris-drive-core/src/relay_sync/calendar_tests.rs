use super::*;
use crate::config::Drive;
use crate::nostr_events::build_private_hashtree_root_event;
use crate::profile::Profile;
use hashtree_core::Cid;
use nostr_sdk::filter::MatchEventOptions;
use tempfile::tempdir;

fn config_with_owner_account(dir: &std::path::Path) -> (AppConfig, Profile) {
    let acct = Profile::create(dir, None).unwrap();
    let mut cfg = AppConfig {
        profile: Some(acct.state.clone()),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(acct.state.root_scope_id()));
    (cfg, acct)
}

fn encrypted_root(seed: u8, published_at: i64, dck_generation: u64) -> AppKeyRootRef {
    AppKeyRootRef::legacy(
        Cid::encrypted([seed; 32], [seed.wrapping_add(1); 32]).to_string(),
        published_at,
        dck_generation,
    )
}

fn filter_matches(filter: &Filter, event: &Event) -> bool {
    filter.match_event(event, MatchEventOptions::default())
}

#[test]
fn subscription_filters_match_calendar_files_root_for_current_app_key() {
    let dir = tempdir().unwrap();
    let (_cfg, acct) = config_with_owner_account(dir.path());
    let root = encrypted_root(0x71, 20, 1);
    let event = build_private_hashtree_root_event(
        acct.app_key.keys(),
        crate::calendar::CALENDAR_TREE_NAME,
        &root,
    )
    .unwrap();

    assert!(
        subscription_filters(
            &acct.state.app_key_pubkey,
            &acct.state.root_scope_id(),
            crate::PRIMARY_DRIVE_ID,
        )
        .iter()
        .any(|filter| filter_matches(filter, &event))
    );
}

#[test]
fn apply_calendar_files_root_creates_calendar_drive_entry() {
    let dir = tempdir().unwrap();
    let (mut cfg, acct) = config_with_owner_account(dir.path());
    let root = encrypted_root(0x72, 20, 1);
    let event = build_private_hashtree_root_event(
        acct.app_key.keys(),
        crate::calendar::CALENDAR_TREE_NAME,
        &root,
    )
    .unwrap();

    let outcome =
        apply_remote_files_root_event(&mut cfg, &event, Some(acct.app_key.keys())).unwrap();

    assert_eq!(outcome, FilesRootApply::Applied);
    let drive = cfg.drive(crate::calendar::CALENDAR_TREE_NAME).unwrap();
    assert_eq!(drive.display_name, "Calendar");
    assert_eq!(
        drive
            .app_key_roots
            .get(&acct.state.app_key_pubkey)
            .map(|stored| stored.root_cid.as_str()),
        Some(root.root_cid.as_str())
    );
}

#[test]
fn calendar_accepts_new_web_root_after_native_write() {
    let dir = tempdir().unwrap();
    let (mut cfg, acct) = config_with_owner_account(dir.path());
    let path = crate::paths::config_path_in(dir.path());
    let tree = crate::calendar::CALENDAR_TREE_NAME;
    let native_root = encrypted_root(0x73, 20, 1);
    let native_event =
        build_private_hashtree_root_event(acct.app_key.keys(), tree, &native_root).unwrap();
    assert_eq!(
        apply_remote_files_root_event(&mut cfg, &native_event, Some(acct.app_key.keys())).unwrap(),
        FilesRootApply::Applied,
    );
    cfg.drives
        .iter_mut()
        .find(|drive| drive.drive_id == tree)
        .unwrap()
        .app_key_roots
        .get_mut(&acct.state.app_key_pubkey)
        .unwrap()
        .app_key_seq = 1;
    cfg.save(&path).unwrap();
    let stale_config = cfg.clone();

    let web_root = encrypted_root(0x74, 21, 0);
    let web_event =
        build_private_hashtree_root_event(acct.app_key.keys(), tree, &web_root).unwrap();
    assert_eq!(
        apply_remote_files_root_event(&mut cfg, &web_event, Some(acct.app_key.keys())).unwrap(),
        FilesRootApply::Applied,
    );
    assert_eq!(
        apply_remote_files_root_event(&mut cfg, &native_event, Some(acct.app_key.keys())).unwrap(),
        FilesRootApply::StaleTimestamp,
    );
    cfg.save(&path).unwrap();
    stale_config.save(&path).unwrap();
    let reopened = AppConfig::load_or_default(&path).unwrap();
    assert_eq!(
        reopened.drive(tree).unwrap().last_root_cid.as_deref(),
        Some(web_root.root_cid.as_str()),
    );
}
