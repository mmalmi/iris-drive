use nostr_sdk::Keys;
use tempfile::tempdir;

use super::event::{ShareIndexEventError, build_share_index_event, parse_share_index_event};
use super::{ShareIndex, ShareIndexPlace};
use crate::config::AppConfig;
use crate::sharing::{ShareShortcut, create_shared_folder};
use crate::{NostrIdentityId, Profile};

/// A device with one folder it shares from `Projects/Alpha`.
fn owner_config(dir: &std::path::Path) -> (AppConfig, NostrIdentityId) {
    let profile = Profile::create(dir, Some("Laptop".into())).unwrap();
    let folder = create_shared_folder(
        profile.app_key.keys(),
        profile.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        None,
        Vec::new(),
        10,
    )
    .unwrap();
    let share_id = folder.share_id;
    let mut config = AppConfig {
        profile: Some(profile.state),
        ..AppConfig::default()
    };
    config.upsert_shared_folder(folder);
    (config, share_id)
}

#[test]
fn a_shared_folder_is_listed_with_its_place_and_the_device_that_answers_for_it() {
    let dir = tempdir().unwrap();
    let (config, share_id) = owner_config(dir.path());
    let device = config.profile.as_ref().unwrap().app_key_pubkey.clone();
    let mut index = ShareIndex::default();

    assert!(index.capture_local(&config, 100));
    let entry = index.entry(share_id).unwrap();
    assert_eq!(entry.admin, device);
    assert_eq!(entry.source_path, "Projects/Alpha");
    assert_eq!(entry.updated_at, 100);
    assert_eq!(index.wanted().count(), 0);
    // Nothing changed since: the list stays as it is.
    assert!(!index.capture_local(&config, 200));
}

#[test]
fn moving_and_removing_a_folder_are_newer_than_anything_seen_before() {
    let dir = tempdir().unwrap();
    let (mut config, share_id) = owner_config(dir.path());
    let mut index = ShareIndex::default();
    index.capture_local(&config, 100);

    // The clock went backwards; the move still wins over the earlier state.
    config.shared_folders[0].source_path = "Work/Alpha".into();
    assert!(index.capture_local(&config, 50));
    let moved = index.entry(share_id).unwrap().clone();
    assert_eq!(moved.source_path, "Work/Alpha");
    assert_eq!(moved.updated_at, 101);

    config.remove_shared_folder(share_id);
    assert!(index.capture_local(&config, 50));
    assert!(index.entry(share_id).is_none());
    let removed = &index.entries[&share_id.to_string()];
    assert!(removed.removed);
    assert_eq!(removed.updated_at, 102);
}

#[test]
fn another_devices_list_adds_folders_to_wait_for_and_removes_the_ones_it_left() {
    let laptop_dir = tempdir().unwrap();
    let (laptop_config, share_id) = owner_config(laptop_dir.path());
    let mut laptop = ShareIndex::default();
    laptop.capture_local(&laptop_config, 100);

    // The phone has the same profile and none of its folders yet.
    let mut phone_config = AppConfig {
        profile: laptop_config.profile.clone(),
        ..AppConfig::default()
    };
    let mut phone = ShareIndex::default();
    assert!(phone.merge(laptop.entries.values().cloned()));
    assert!(!phone.apply_to_config(&mut phone_config));
    assert_eq!(
        phone
            .wanted()
            .map(|entry| entry.share_id)
            .collect::<Vec<_>>(),
        [share_id]
    );
    // The same list again changes nothing.
    assert!(!phone.merge(laptop.entries.values().cloned()));

    // Once the phone has the folder, the laptop's shortcut for it applies there.
    phone_config.upsert_shared_folder(laptop_config.shared_folders[0].clone());
    phone.capture_local(&phone_config, 100);
    assert_eq!(phone.wanted().count(), 0);
    let mut moved = laptop_config.clone();
    moved.upsert_share_shortcut(ShareShortcut::new(share_id, "Pinned/Alpha", "").unwrap());
    laptop.capture_local(&moved, 100);
    assert!(phone.merge(laptop.entries.values().cloned()));
    assert!(phone.apply_to_config(&mut phone_config));
    assert_eq!(phone_config.share_shortcuts.len(), 1);
    assert_eq!(phone_config.share_shortcuts[0].path, "Pinned/Alpha");
    // What the list put there is not a change of the phone's own.
    assert!(!phone.capture_local(&phone_config, 300));

    // The laptop stops sharing: the phone drops the folder too.
    moved.remove_shared_folder(share_id);
    moved.remove_share_shortcuts_for_share(share_id);
    laptop.capture_local(&moved, 100);
    assert!(phone.merge(laptop.entries.values().cloned()));
    assert!(phone.apply_to_config(&mut phone_config));
    assert_eq!(phone_config.shared_folders, Vec::new());
    assert_eq!(phone_config.share_shortcuts, Vec::new());
    assert!(!phone.capture_local(&phone_config, 300));
}

#[test]
fn an_older_change_never_replaces_a_newer_one() {
    let dir = tempdir().unwrap();
    let (mut config, share_id) = owner_config(dir.path());
    let mut index = ShareIndex::default();
    index.capture_local(&config, 100);
    let mut older = index.entry(share_id).unwrap().clone();
    older.source_path = "Old/Alpha".into();
    config.shared_folders[0].source_path = "Work/Alpha".into();
    index.capture_local(&config, 100);

    assert!(!index.merge([older]));
    assert_eq!(index.entry(share_id).unwrap().source_path, "Work/Alpha");
}

#[test]
fn leaving_wins_over_a_move_made_before_the_device_heard_of_it() {
    let dir = tempdir().unwrap();
    let (config, share_id) = owner_config(dir.path());
    let mut laptop = ShareIndex::default();
    laptop.capture_local(&config, 100);
    let mut phone = laptop.clone();

    // The laptop stops sharing; the phone, not having heard, moves the folder
    // and its clock stamps the move later.
    let mut stopped = config.clone();
    stopped.remove_shared_folder(share_id);
    laptop.capture_local(&stopped, 100);
    let mut moved = config.clone();
    moved.shared_folders[0].source_path = "Work/Alpha".into();
    phone.capture_local(&moved, 900);

    assert!(phone.merge(laptop.entries.values().cloned()));
    assert!(phone.apply_to_config(&mut moved));
    assert_eq!(moved.shared_folders, Vec::new());
    assert!(!laptop.merge(phone.entries.values().cloned()));

    // Sharing it again later is a new beginning that wins over the removal.
    laptop.capture_local(&config, 100);
    let again = laptop.entry(share_id).unwrap().clone();
    assert_eq!(again.generation, 1);
    assert!(phone.merge([again]));
    assert_eq!(phone.wanted().count(), 1);
}

#[test]
fn entries_that_are_not_well_formed_are_left_out() {
    let dir = tempdir().unwrap();
    let (config, share_id) = owner_config(dir.path());
    let mut index = ShareIndex::default();
    index.capture_local(&config, 100);
    let good = index.entry(share_id).unwrap().clone();

    let mut bad_admin = good.clone();
    bad_admin.admin = "not a key".into();
    bad_admin.updated_at = 500;
    let mut bad_place = good.clone();
    bad_place.shortcuts = vec![ShareIndexPlace {
        path: "../outside".into(),
        target_path: String::new(),
    }];
    bad_place.updated_at = 500;

    assert!(!ShareIndex::default().merge([bad_admin, bad_place]));
}

#[test]
fn the_list_opens_only_for_the_devices_it_was_wrapped_for() {
    let dir = tempdir().unwrap();
    let (config, _) = owner_config(dir.path());
    let mut index = ShareIndex::default();
    index.capture_local(&config, 100);
    let laptop = Keys::generate();
    let phone = Keys::generate();
    let stranger = Keys::generate();

    let event = build_share_index_event(
        &laptop,
        "scope",
        index.entries.values(),
        &[phone.public_key().to_hex()],
        1_000,
    )
    .unwrap();

    assert!(super::is_share_index_event(&event));
    let entries = parse_share_index_event(&event, &phone).unwrap();
    assert_eq!(entries, index.entries.values().cloned().collect::<Vec<_>>());
    assert_eq!(parse_share_index_event(&event, &laptop).unwrap(), entries);
    assert!(matches!(
        parse_share_index_event(&event, &stranger),
        Err(ShareIndexEventError::KeyUnavailable)
    ));
    // The folder list itself is nowhere in what relays see.
    assert!(!event.content.contains("Alpha"));
}
