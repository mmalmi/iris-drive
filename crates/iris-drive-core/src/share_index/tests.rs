use nostr_sdk::Keys;
use tempfile::tempdir;

use super::event::{
    ShareIndexEventError, build_share_index_event, parse_share_index_event, split_into_parts,
};
use super::{ShareIndex, ShareIndexEntry, ShareIndexPlace};
use crate::config::AppConfig;
use crate::paths::config_path_in;
use crate::sharing::{ShareRole, ShareShortcut, create_shared_folder};
use crate::{NostrIdentityId, Profile};

mod concurrency;

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

fn device_of(config: &AppConfig) -> String {
    config.profile.as_ref().unwrap().app_key_pubkey.clone()
}

/// A folder as `device` would list it after a change at `updated_at`.
fn listed(device: &Keys, name: &str, updated_at: i64) -> ShareIndexEntry {
    ShareIndexEntry {
        share_id: NostrIdentityId::new_v4(),
        owner_profile_id: NostrIdentityId::new_v4(),
        admin: device.public_key().to_hex(),
        name: name.to_owned(),
        role: ShareRole::Editor,
        link_key: Some(Keys::generate().secret_key().to_secret_hex()),
        source_path: String::new(),
        shortcuts: vec![ShareIndexPlace {
            path: name.to_owned(),
            target_path: String::new(),
        }],
        removed: false,
        generation: 0,
        updated_at,
        updated_by: device.public_key().to_hex(),
    }
}

#[test]
fn a_shared_folder_is_listed_with_its_place_and_the_device_that_answers_for_it() {
    let dir = tempdir().unwrap();
    let (config, share_id) = owner_config(dir.path());
    let mut index = ShareIndex::default();

    assert!(index.capture_local(&config, 100));
    let entry = index.entry(share_id).unwrap();
    assert_eq!(entry.admin, device_of(&config));
    assert_eq!(entry.source_path, "Projects/Alpha");
    assert_eq!(entry.updated_at, 100);
    assert_eq!(index.wanted().count(), 0);
    // Nothing changed since: the list stays as it is.
    assert!(!index.capture_local(&config, 200));
}

#[test]
fn moving_and_leaving_a_folder_are_newer_than_anything_seen_before() {
    let dir = tempdir().unwrap();
    let (mut config, share_id) = owner_config(dir.path());
    let device = device_of(&config);
    let mut index = ShareIndex::default();
    index.capture_local(&config, 100);

    // The clock went backwards; the move still wins over the earlier state.
    config.shared_folders[0].source_path = "Work/Alpha".into();
    assert!(index.capture_local(&config, 50));
    let moved = index.entry(share_id).unwrap().clone();
    assert_eq!(moved.source_path, "Work/Alpha");
    assert_eq!(moved.updated_at, 101);

    assert!(index.note_removed(share_id, &device, 50));
    assert!(index.entry(share_id).is_none());
    let removed = &index.entries[&share_id.to_string()];
    assert!(removed.removed);
    assert_eq!(removed.updated_at, 102);
    // Leaving twice says nothing new.
    assert!(!index.note_removed(share_id, &device, 50));
}

#[test]
fn another_devices_list_adds_folders_to_wait_for_and_removes_the_ones_it_left() {
    let laptop_dir = tempdir().unwrap();
    let (laptop_config, share_id) = owner_config(laptop_dir.path());
    let laptop_device = device_of(&laptop_config);
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
    laptop.note_removed(share_id, &laptop_device, 100);
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
    let device = device_of(&config);
    let mut laptop = ShareIndex::default();
    laptop.capture_local(&config, 100);
    let mut phone = laptop.clone();

    // The laptop stops sharing; the phone, not having heard, moves the folder
    // and its clock stamps the move later.
    laptop.note_removed(share_id, &device, 100);
    let mut moved = config.clone();
    moved.shared_folders[0].source_path = "Work/Alpha".into();
    phone.capture_local(&moved, 900);

    assert!(phone.merge(laptop.entries.values().cloned()));
    assert!(phone.apply_to_config(&mut moved));
    assert_eq!(moved.shared_folders, Vec::new());
    assert!(!laptop.merge(phone.entries.values().cloned()));

    // Sharing it again later is a new beginning that wins over the removal.
    assert!(laptop.note_joined(&config, share_id, 100));
    let again = laptop.entry(share_id).unwrap().clone();
    assert_eq!(again.generation, 1);
    assert!(phone.merge([again]));
    assert_eq!(phone.wanted().count(), 1);
}

#[test]
fn devices_that_change_the_list_at_once_agree_whatever_order_the_lists_arrive_in() {
    let laptop = Keys::generate();
    let phone = Keys::generate();
    let tablet = Keys::generate();
    // Each device adds a folder of its own; all three also change one folder
    // at the same moment, one of them by leaving it.
    let contested = listed(&laptop, "Shared", 100);
    let mut by_phone = contested.clone();
    by_phone.shortcuts[0].path = "Phone/Shared".into();
    by_phone.updated_by = phone.public_key().to_hex();
    by_phone.updated_at = 200;
    let mut by_tablet = contested.clone();
    by_tablet.removed = true;
    by_tablet.shortcuts.clear();
    by_tablet.updated_by = tablet.public_key().to_hex();
    by_tablet.updated_at = 150;
    let lists = [
        vec![listed(&laptop, "Alpha", 100), contested],
        vec![listed(&phone, "Beta", 100), by_phone],
        vec![listed(&tablet, "Gamma", 100), by_tablet.clone()],
    ];

    let orders = [
        [0, 1, 2],
        [0, 2, 1],
        [1, 0, 2],
        [1, 2, 0],
        [2, 0, 1],
        [2, 1, 0],
    ];
    let merged = orders.map(|order| {
        let mut index = ShareIndex::default();
        for device in order {
            index.merge(lists[device].clone());
            // The same list arriving twice changes nothing.
            assert!(!index.merge(lists[device].clone()));
        }
        index.entries
    });

    for other in &merged[1..] {
        assert_eq!(other, &merged[0]);
    }
    // Changes to different folders are all kept; the one folder all three
    // changed has one outcome everywhere: it was left.
    assert_eq!(merged[0].len(), 4);
    assert_eq!(merged[0][&by_tablet.share_id.to_string()], by_tablet);
}

#[test]
fn a_folder_that_goes_missing_from_the_config_is_added_again_not_left() {
    let dir = tempdir().unwrap();
    let (config, share_id) = owner_config(dir.path());
    let mut index = ShareIndex::default();
    index.capture_local(&config, 100);

    // A stale copy of the config without the folder was written over it.
    let mut stale = config.clone();
    stale.remove_shared_folder(share_id);
    assert!(index.capture_local(&stale, 200));

    // The profile did not leave: the folder is waited for again.
    let entry = index.entry(share_id).unwrap();
    assert!(!entry.removed);
    assert_eq!(entry.updated_at, 100);
    assert_eq!(
        index
            .wanted()
            .map(|entry| entry.share_id)
            .collect::<Vec<_>>(),
        [share_id]
    );
}

#[test]
fn a_folder_the_profile_left_does_not_come_back_with_a_stale_config() {
    let dir = tempdir().unwrap();
    let (config, share_id) = owner_config(dir.path());
    let device = device_of(&config);
    let mut index = ShareIndex::default();
    index.capture_local(&config, 100);
    index.note_removed(share_id, &device, 100);
    let left = index.clone();

    // A stale copy of the config that still has the folder was written back.
    let mut stale = config.clone();
    assert!(!index.capture_local(&stale, 500));
    assert_eq!(index, left);
    // The folder goes again; nothing is said to the other devices.
    assert!(index.apply_to_config(&mut stale));
    assert_eq!(stale.shared_folders, Vec::new());
    assert_eq!(index.entries, left.entries);
}

#[test]
fn a_list_left_by_another_profile_is_not_used() {
    let dir = tempdir().unwrap();
    let (config, share_id) = owner_config(dir.path());
    let mut index = ShareIndex::load_for(dir.path(), &config).unwrap();
    index.capture_local(&config, 100);
    index.save(dir.path()).unwrap();
    assert!(
        ShareIndex::load_for(dir.path(), &config)
            .unwrap()
            .entry(share_id)
            .is_some()
    );

    // The device signs in to another profile; the old profile's folders are
    // still in its config, as they are after logging out.
    let other_dir = tempdir().unwrap();
    let other = Profile::create(other_dir.path(), Some("Other".into())).unwrap();
    let switched = AppConfig {
        profile: Some(other.state),
        shared_folders: config.shared_folders.clone(),
        ..AppConfig::default()
    };
    let mut index = ShareIndex::load_for(dir.path(), &switched).unwrap();
    assert_eq!(index.entries.len(), 0);
    // They are not this profile's folders: its list stays empty.
    assert!(!index.capture_local(&switched, 200));
    assert_eq!(index.entries.len(), 0);
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
    let recipients = [phone.public_key().to_hex()];
    let sign = || {
        build_share_index_event(
            &laptop,
            "scope",
            0,
            index.entries.values(),
            &recipients,
            1_000,
        )
        .unwrap()
    };

    let event = sign();
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
    // The same list signed again uses a new key: the two do not look alike.
    assert_ne!(sign().content, event.content);
}

#[test]
fn a_long_list_is_split_over_events_relays_accept_and_a_reader_gets_every_folder() {
    let laptop = Keys::generate();
    let phone = Keys::generate();
    let recipients = [phone.public_key().to_hex()];
    let entries = (0..1_000)
        .map(|n| listed(&laptop, &format!("Folder {n}"), 100))
        .collect::<Vec<_>>();

    let all = entries.iter().collect::<Vec<_>>();
    let parts = split_into_parts(&all, 1);
    assert!(parts.len() > 1);
    let mut reader = ShareIndex::default();
    for (part, entries) in (0_u32..).zip(parts.clone()) {
        let event =
            build_share_index_event(&laptop, "scope", part, entries, &recipients, 1_000).unwrap();
        // Well under the 64 KB that many relays accept for one event.
        assert!(nostr_sdk::JsonUtil::as_json(&event).len() < 48 * 1024);
        reader.merge(parse_share_index_event(&event, &phone).unwrap());
    }
    assert_eq!(reader.entries.len(), entries.len());

    // A list that grew keeps every folder in the part it was in, or moves it
    // to a new part: no part already sent is left holding a stale copy alone.
    let more = split_into_parts(&all, u32::try_from(parts.len()).unwrap() * 2);
    assert_eq!(more.len(), parts.len() * 2);
    for (part, before) in parts.iter().enumerate() {
        for entry in before {
            assert!(
                more[part].contains(entry) || more[part + parts.len()].contains(entry),
                "a folder left its part for an unrelated one"
            );
        }
    }
    // A short list is one event, at the address it always had.
    assert_eq!(split_into_parts(&all[..5], 1).len(), 1);
}

#[test]
fn leaving_and_joining_are_recorded_beside_the_config() {
    let dir = tempdir().unwrap();
    let (config, share_id) = owner_config(dir.path());
    config.save(config_path_in(dir.path())).unwrap();
    let mut index = ShareIndex::load_for(dir.path(), &config).unwrap();
    index.capture_local(&config, 100);
    index.save(dir.path()).unwrap();

    super::record_removed(dir.path(), &config, share_id, 100).unwrap();
    let left = ShareIndex::load_for(dir.path(), &config).unwrap();
    assert!(left.entry(share_id).is_none());
    assert!(left.entries[&share_id.to_string()].removed);

    super::record_joined(dir.path(), &config, share_id, 100).unwrap();
    let back = ShareIndex::load_for(dir.path(), &config).unwrap();
    assert_eq!(back.entry(share_id).unwrap().generation, 1);
}
