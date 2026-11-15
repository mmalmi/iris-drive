//! Devices a native admin lets into a folder are told the way Iris Drive web
//! listens, end to end over a real relay: a note to each device, then the
//! admin's signed access list that includes it.
use super::devices_tests::{
    act, block_on, join_link, link_device, new_profile, only_share_id, sync,
};
use super::*;
use iris_drive_core::share_added::{
    ShareAddedNotice, ShareAddedOutbox, read_share_added_notice, share_added_filter,
};
use iris_drive_core::share_join::{ShareAnchor, shared_folder_from_access_event};
use iris_drive_core::{NostrIdentityId, ShareAction, ShareRole};
use std::time::Duration;
use tempfile::tempdir;

fn device_keys(dir: &Path) -> nostr_sdk::Keys {
    iris_drive_core::AppKey::load(iris_drive_core::paths::key_path_in(dir))
        .unwrap()
        .keys()
        .clone()
}

fn profile_id(dir: &Path) -> NostrIdentityId {
    AppConfig::load_or_default(config_path_in(dir))
        .unwrap()
        .profile
        .unwrap()
        .profile_id
}

/// A folder a device was told of: what the note says, who sent it, and the
/// role the sender's signed access list gives the device.
#[derive(Debug, PartialEq)]
struct Told {
    notice: ShareAddedNotice,
    admin: String,
    role_in_access_list: Option<ShareRole>,
}

/// What Iris Drive web does when it opens: ask the relay for the notes to
/// this device, read each, and look for the access list its sender signed.
fn told(relay_of: &Path, device: &nostr_sdk::Keys) -> Vec<Told> {
    let timeout = Duration::from_secs(5);
    let pubkey = device.public_key().to_hex();
    block_on(async {
        let config = AppConfig::load_or_default(config_path_in(relay_of)).unwrap();
        let client = iris_drive_core::relay_sync::connect(&config.relays)
            .await
            .unwrap();
        let notes = client
            .fetch_events(share_added_filter(&pubkey).unwrap(), timeout)
            .await
            .unwrap();
        let mut told = Vec::new();
        for note in notes {
            let notice = read_share_added_notice(&note, device).unwrap();
            let anchor = ShareAnchor {
                share_id: notice.share_id,
                owner_profile_id: notice.owner_profile_id,
                admin: note.pubkey.to_hex(),
                name: notice.name.clone(),
            };
            let lists = iris_drive_core::relay_sync::fetch_share_access_snapshots(
                &client,
                notice.share_id,
                timeout,
            )
            .await
            .unwrap();
            told.push(Told {
                role_in_access_list: lists
                    .iter()
                    .find_map(|list| shared_folder_from_access_event(&anchor, list, &pubkey))
                    .map(|folder| folder.local_role),
                admin: anchor.admin,
                notice,
            });
        }
        iris_drive_core::relay_sync::shutdown_client(&client).await;
        told
    })
}

#[test]
fn devices_a_profile_links_later_are_each_told_of_its_shared_folder() {
    let services = super::sharing_test_network::SharingTestServices::new();
    let laptop = tempdir().unwrap();
    new_profile(laptop.path(), "Laptop");
    services.prepare(laptop.path());
    cmd_shares_create(laptop.path(), "Projects/Alpha", None).unwrap();
    let share_id = only_share_id(laptop.path());
    let laptop_keys = device_keys(laptop.path());

    // Two devices are linked; neither syncs, as a closed browser tab would not.
    let browser = tempdir().unwrap();
    link_device(laptop.path(), browser.path(), "Browser");
    let tablet = tempdir().unwrap();
    link_device(laptop.path(), tablet.path(), "Tablet");
    // A device that is not in the folder was once owed a note.
    let stranger = nostr_sdk::Keys::generate();
    let mut outbox = ShareAddedOutbox::default();
    outbox
        .untold
        .entry(share_id.to_string())
        .or_default()
        .insert(stranger.public_key().to_hex());
    outbox.save(laptop.path()).unwrap();

    sync(laptop.path());

    // The laptop holds the profile's folder, so its devices manage it; the
    // note says editor, the most it can say, and the access list says admin.
    let expected = Told {
        notice: ShareAddedNotice {
            v: 1,
            share_id,
            owner_profile_id: profile_id(laptop.path()),
            name: "Alpha".into(),
            role: ShareRole::Editor,
        },
        admin: laptop_keys.public_key().to_hex(),
        role_in_access_list: Some(ShareRole::Admin),
    };
    for device in [&browser, &tablet] {
        assert_eq!(
            told(laptop.path(), &device_keys(device.path())),
            [Told {
                notice: expected.notice.clone(),
                admin: expected.admin.clone(),
                role_in_access_list: expected.role_in_access_list,
            }]
        );
    }
    assert_eq!(told(laptop.path(), &laptop_keys), []);
    assert_eq!(told(laptop.path(), &stranger), []);
    assert_eq!(
        ShareAddedOutbox::load(laptop.path()).unwrap(),
        ShareAddedOutbox::default()
    );

    // Nothing new to say: the relay still holds one note for each device.
    sync(laptop.path());
    assert_eq!(told(laptop.path(), &device_keys(browser.path())).len(), 1);
    assert_eq!(told(laptop.path(), &device_keys(tablet.path())).len(), 1);
}

#[test]
fn a_device_let_in_by_link_is_told_of_the_folder() {
    let services = super::sharing_test_network::SharingTestServices::new();
    let owner = tempdir().unwrap();
    new_profile(owner.path(), "Owner");
    services.prepare(owner.path());
    cmd_shares_create(owner.path(), "Projects/Alpha", None).unwrap();
    let share_id = only_share_id(owner.path());
    let link = join_link(owner.path(), share_id, ShareRole::Reader);

    let guest = tempdir().unwrap();
    new_profile(guest.path(), "Guest");
    services.prepare(guest.path());
    let guest_keys = device_keys(guest.path());
    act(
        guest.path(),
        ShareAction::AcceptShareInvite { invite: link },
    );
    assert_eq!(told(owner.path(), &guest_keys), []);

    // The owner's Drive sees the request, lets the guest in and says so.
    sync(owner.path());
    let expected = [Told {
        notice: ShareAddedNotice {
            v: 1,
            share_id,
            owner_profile_id: profile_id(owner.path()),
            name: "Alpha".into(),
            role: ShareRole::Reader,
        },
        admin: device_keys(owner.path()).public_key().to_hex(),
        role_in_access_list: Some(ShareRole::Reader),
    }];
    assert_eq!(told(owner.path(), &guest_keys), expected);
    assert_eq!(
        ShareAddedOutbox::load(owner.path()).unwrap(),
        ShareAddedOutbox::default()
    );

    // The request stays on the relay; seeing it again tells nobody again.
    sync(owner.path());
    assert_eq!(told(owner.path(), &guest_keys), expected);
}
