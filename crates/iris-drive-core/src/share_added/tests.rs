use nostr_sdk::nips::nip44;
use nostr_sdk::{Event, EventBuilder, JsonUtil, Keys, Kind, Tag};
use serde::Deserialize;

use super::{
    SHARE_ADDED_LABEL, ShareAddedNotice, build_share_added_notices, device_note_d_tag,
    read_share_added_notice, share_added_filter,
};
use crate::nostr_events::KIND_DRIVE_ROOT;
use crate::sharing::{ShareRole, create_shared_folder};

/// A notice written by Iris Drive web's `buildShareAddedNotices`, with the
/// keys and the JSON it was made from.
#[derive(Deserialize)]
struct WebFixture {
    admin_secret_key: String,
    device_secret_key: String,
    plaintext: String,
    event: serde_json::Value,
}

fn web_fixture() -> (WebFixture, Event) {
    let fixture: WebFixture = serde_json::from_str(include_str!(
        "../../tests/fixtures/share_added_web_notice.json"
    ))
    .unwrap();
    let event = Event::from_json(fixture.event.to_string()).unwrap();
    (fixture, event)
}

fn notice() -> ShareAddedNotice {
    ShareAddedNotice {
        v: 1,
        share_id: "123e4567-e89b-42d3-a456-426614174011".parse().unwrap(),
        owner_profile_id: "123e4567-e89b-42d3-a456-426614174022".parse().unwrap(),
        name: "Tax papers 2026".into(),
        role: ShareRole::Editor,
    }
}

fn tags(event: &Event) -> Vec<Vec<String>> {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect()
}

#[test]
fn a_notice_is_built_exactly_as_iris_drive_web_builds_it() {
    let (fixture, web) = web_fixture();
    let admin = Keys::parse(&fixture.admin_secret_key).unwrap();
    let device = Keys::parse(&fixture.device_secret_key).unwrap();
    let device_pubkey = device.public_key().to_hex();

    let built = build_share_added_notices(
        &admin,
        &notice(),
        // A device named twice is told once.
        &[device_pubkey.clone(), device_pubkey.clone()],
        1000,
    )
    .unwrap();
    assert_eq!(built.len(), 1);
    let built = &built[0];

    assert_eq!(built.kind, Kind::from(KIND_DRIVE_ROOT));
    assert_eq!(built.kind, web.kind);
    assert_eq!(built.pubkey, web.pubkey);
    assert_eq!(built.created_at, web.created_at);
    assert_eq!(tags(built), tags(&web));
    assert_eq!(
        tags(built),
        [
            vec![
                "d".to_owned(),
                device_note_d_tag(
                    SHARE_ADDED_LABEL,
                    &["123e4567-e89b-42d3-a456-426614174011"],
                    &device_pubkey,
                ),
            ],
            vec!["l".to_owned(), "iris-drive/share-added".to_owned()],
            vec!["p".to_owned(), device_pubkey.clone()],
        ]
    );
    // The encryption is salted, so the words inside are compared.
    let open =
        |event: &Event| nip44::decrypt(device.secret_key(), &event.pubkey, &event.content).unwrap();
    assert_eq!(open(built), fixture.plaintext);
    assert_eq!(open(&web), fixture.plaintext);
    built.verify().unwrap();
}

#[test]
fn a_notice_from_iris_drive_web_is_read_only_by_the_device_it_is_for() {
    let (fixture, web) = web_fixture();
    let device = Keys::parse(&fixture.device_secret_key).unwrap();

    assert_eq!(read_share_added_notice(&web, &device), Some(notice()));
    assert_eq!(read_share_added_notice(&web, &Keys::generate()), None);

    let admin = Keys::parse(&fixture.admin_secret_key).unwrap();
    let other = Keys::generate();
    let built = build_share_added_notices(
        &admin,
        &notice(),
        &[device.public_key().to_hex(), other.public_key().to_hex()],
        1000,
    )
    .unwrap();
    assert_eq!(built.len(), 2);
    assert_eq!(read_share_added_notice(&built[0], &device), Some(notice()));
    assert_eq!(read_share_added_notice(&built[1], &other), Some(notice()));
    assert_eq!(read_share_added_notice(&built[0], &other), None);
    // Relays see a label and the device, not the folder, its name or owner.
    let published = built[0].as_json();
    assert!(!published.contains("123e4567"));
    assert!(!published.contains("Tax papers"));
}

#[test]
fn a_notice_that_is_plain_mislabelled_or_about_another_folder_is_ignored() {
    let admin = Keys::generate();
    let device = Keys::generate();
    let device_pubkey = device.public_key().to_hex();
    let good = build_share_added_notices(
        &admin,
        &notice(),
        std::slice::from_ref(&device_pubkey),
        1000,
    )
    .unwrap()
    .remove(0);
    let resigned = |content: String, tags: Vec<Tag>| {
        EventBuilder::new(Kind::from(KIND_DRIVE_ROOT), content)
            .tags(tags)
            .sign_with_keys(&admin)
            .unwrap()
    };
    let good_tags = || good.tags.iter().cloned().collect::<Vec<_>>();
    assert!(
        read_share_added_notice(&resigned(good.content.clone(), good_tags()), &device).is_some()
    );

    let plain = resigned(serde_json::to_string(&notice()).unwrap(), good_tags());
    assert_eq!(read_share_added_notice(&plain, &device), None);

    let unlabelled = good_tags()
        .into_iter()
        .filter(|tag| tag.as_slice()[0] != "l")
        .collect();
    assert_eq!(
        read_share_added_notice(&resigned(good.content.clone(), unlabelled), &device),
        None
    );

    // The name on the outside must be the folder named inside.
    let other_folder = ShareAddedNotice {
        share_id: "123e4567-e89b-42d3-a456-426614174033".parse().unwrap(),
        ..notice()
    };
    let swapped = build_share_added_notices(&admin, &other_folder, &[device_pubkey], 1000)
        .unwrap()
        .remove(0);
    assert_eq!(
        read_share_added_notice(&resigned(swapped.content.clone(), good_tags()), &device),
        None
    );
}

#[test]
fn a_notice_names_no_role_above_editor() {
    let admin = Keys::generate();
    let folder = create_shared_folder(
        &admin,
        "123e4567-e89b-42d3-a456-426614174022".parse().unwrap(),
        "Projects/Alpha",
        "Alpha",
        None,
        Vec::new(),
        10,
    )
    .unwrap();
    // Iris Drive web reads only editor and reader; the access list a device
    // then waits for gives it the role it really has.
    let told = ShareAddedNotice::new(&folder, ShareRole::Admin);
    assert_eq!(told.role, ShareRole::Editor);
    assert_eq!(told.name, "Alpha");
    assert_eq!(told.share_id, folder.share_id);
    assert_eq!(told.owner_profile_id, folder.owner_profile_id);
    assert_eq!(
        ShareAddedNotice::new(&folder, ShareRole::Reader).role,
        ShareRole::Reader
    );

    let device = Keys::generate();
    let as_admin = ShareAddedNotice {
        role: ShareRole::Admin,
        ..told
    };
    let event = build_share_added_notices(&admin, &as_admin, &[device.public_key().to_hex()], 10)
        .unwrap()
        .remove(0);
    assert_eq!(read_share_added_notice(&event, &device), None);
}

#[test]
fn a_device_asks_relays_for_its_notices_the_way_iris_drive_web_does() {
    let device = Keys::generate().public_key().to_hex();
    let filter = serde_json::to_value(share_added_filter(&device).unwrap()).unwrap();
    assert_eq!(
        filter,
        serde_json::json!({
            "kinds": [30078],
            "#l": ["iris-drive/share-added"],
            "#p": [device],
        })
    );
    assert!(share_added_filter("not a key").is_none());
}

#[tokio::test]
async fn devices_stay_owed_until_drive_reaches_a_relay() {
    use super::outbox::{ShareAddedOutbox, owe_share_added_notices, publish_share_added_notices};
    let dir = tempfile::tempdir().unwrap();
    let share_id = notice().share_id;
    let device = Keys::generate().public_key().to_hex();

    // Letting nobody in owes nothing and writes nothing.
    owe_share_added_notices(dir.path(), &[(share_id, Vec::new())]).unwrap();
    assert!(!dir.path().join(super::outbox::SHARE_ADDED_FILE).exists());

    owe_share_added_notices(dir.path(), &[(share_id, vec![device.clone()])]).unwrap();
    owe_share_added_notices(dir.path(), &[(share_id, vec![device.clone()])]).unwrap();
    let owed = ShareAddedOutbox::load(dir.path()).unwrap();
    assert_eq!(
        owed.untold.get(&share_id.to_string()),
        Some(&std::collections::BTreeSet::from([device]))
    );

    // No relay to send through: the device is still owed its note.
    let offline = nostr_sdk::Client::default();
    assert_eq!(
        publish_share_added_notices(&offline, dir.path())
            .await
            .unwrap(),
        0
    );
    assert_eq!(ShareAddedOutbox::load(dir.path()).unwrap(), owed);
}
