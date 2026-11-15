use std::collections::BTreeMap;
use std::sync::Arc;

use hashtree_core::{Cid, DirEntry, HashTree, HashTreeConfig, LinkType, MemoryStore, Store};
use nostr_sdk::{Event, JsonUtil, Keys, SecretKey};
use serde_json::{Value, json};

use super::measure::{HoldingCache, measure_holding};
use super::view::{
    SyncState, folder_lines, holders_sentence, holding_label, space_label, summary, sync_state,
};
use super::*;
use crate::sealed_content::SealedContentError;

mod flow;

/// Built by Iris Drive web's `buildDeviceStorageEvent`
/// (`src/drive/deviceStorageReport.ts`) with fixed keys.
const WEB_EVENT: &str = include_str!("tests/web_event.json");

fn fixture() -> Value {
    serde_json::from_str(WEB_EVENT).unwrap()
}

fn keys(secret: &Value) -> Keys {
    Keys::new(SecretKey::from_hex(secret.as_str().unwrap()).unwrap())
}

fn report(overrides: &Value) -> DeviceStorageReport {
    let mut value = json!({
        "v": 1, "measured_at": 1_700_000_000_u64, "app": "web", "root": "ab".repeat(32),
        "files": 3, "file_bytes": 3000, "held_files": 1, "held_bytes": 1000,
        "stored_bytes": 5000, "capacity_bytes": 10_000,
    });
    for (name, field) in overrides.as_object().unwrap() {
        value[name] = field.clone();
    }
    read_device_storage_report(&value).unwrap()
}

#[test]
fn a_report_the_web_built_opens_here() {
    let fixture = fixture();
    let event = Event::from_json(fixture["event"].to_string()).unwrap();
    let profile = fixture["profile_id"].as_str().unwrap();
    assert_eq!(
        device_storage_d_tag(profile),
        fixture["d_tag"].as_str().unwrap()
    );
    assert!(is_device_storage_event_for(&event, profile));
    assert!(looks_like_device_storage_event(&event));

    let opened =
        open_device_storage_event(&event, profile, &keys(&fixture["native_secret"])).unwrap();
    let expected = read_device_storage_report(&fixture["report"]).unwrap();
    assert_eq!(opened, expected);
    assert_eq!(
        opened.folders.as_ref().unwrap()[1].path,
        ["Photos", "Ünïcödé 2026"]
    );
    assert_eq!(serde_json::to_value(&opened).unwrap(), fixture["report"]);

    assert_eq!(
        open_device_storage_event(&event, profile, &Keys::generate()),
        Err(SealedContentError::KeyUnavailable)
    );
    assert!(
        open_device_storage_event(&event, "another-profile", &keys(&fixture["native_secret"]))
            .is_err()
    );
}

#[test]
fn a_report_built_here_has_the_web_shape_and_opens_for_the_web_device() {
    let fixture = fixture();
    let profile = fixture["profile_id"].as_str().unwrap();
    let native = keys(&fixture["native_secret"]);
    let web = keys(&fixture["web_secret"]);
    let sent = report(&json!({
        "app": "macos",
        "seen": { web.public_key().to_hex(): 3 },
        "loose": { "files": 1, "bytes": 10, "held": 10 },
        "folders": [{ "path": ["Docs"], "bytes": 2000, "held": 0 }],
    }));
    let event = build_device_storage_event(
        &native,
        profile,
        &sent,
        &[web.public_key().to_hex()],
        1_763_200_010,
    )
    .unwrap();

    // Relays can tell neither the profile nor what the event is.
    let tags = event
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect::<Vec<_>>();
    assert_eq!(
        tags,
        vec![vec!["d".to_owned(), device_storage_d_tag(profile)]]
    );
    let json = event.as_json();
    assert!(!json.contains(profile) && !json.contains(DEVICE_STORAGE_LABEL));
    assert!(!event.content.contains("Docs") && !event.content.contains("macos"));
    let content: Value = serde_json::from_str(&event.content).unwrap();
    assert_eq!(content["v"], 1);
    assert_eq!(content["key_wraps"].as_object().unwrap().len(), 2);

    assert_eq!(
        open_device_storage_event(&event, profile, &web).unwrap(),
        sent
    );
    assert_eq!(
        open_device_storage_event(&event, profile, &native).unwrap(),
        sent
    );

    // For the web side of the check: `NATIVE_EVENT_OUT=<file>` writes the
    // event, which Iris Drive web's `openDeviceStorageEvent` must open.
    if let Ok(path) = std::env::var("NATIVE_EVENT_OUT") {
        let out = json!({
            "profile_id": profile,
            "web_secret": fixture["web_secret"],
            "report": serde_json::to_value(&sent).unwrap(),
            "event": serde_json::from_str::<Value>(&json).unwrap(),
        });
        std::fs::write(path, serde_json::to_vec_pretty(&out).unwrap()).unwrap();
    }
}

#[test]
fn a_changed_report_is_refused() {
    let fixture = fixture();
    let mut event: Value = fixture["event"].clone();
    event["created_at"] = json!(event["created_at"].as_u64().unwrap() + 1);
    let event = Event::from_json(event.to_string()).unwrap();
    assert!(
        open_device_storage_event(
            &event,
            fixture["profile_id"].as_str().unwrap(),
            &keys(&fixture["native_secret"])
        )
        .is_err()
    );
}

#[test]
fn only_well_formed_fields_are_kept_and_never_more_held_than_there_is() {
    let mut base = serde_json::to_value(report(&json!({}))).unwrap();
    base["files"] = json!(-1);
    assert!(read_device_storage_report(&base).is_none());
    let mut base = serde_json::to_value(report(&json!({}))).unwrap();
    base["root"] = json!("nope");
    assert!(read_device_storage_report(&base).is_none());

    let read = report(&json!({
        "held_bytes": 9999,
        "seen": { "cd".repeat(32): 4, "bad": 2, "ef".repeat(32): -1 },
        "loose": { "files": 1, "bytes": 10, "held": 20 },
        "folders": [
            { "path": ["Photos"], "bytes": 100, "held": 500 },
            { "path": [], "bytes": 1, "held": 1 },
            { "name": "old", "bytes": 1, "held": 1 },
        ],
    }));
    assert_eq!(read.held_bytes, 3000);
    assert_eq!(read.seen, Some(BTreeMap::from([("cd".repeat(32), 4)])));
    assert_eq!(
        read.loose,
        Some(DeviceStorageFiles {
            files: 1,
            bytes: 10,
            held: 10
        })
    );
    assert_eq!(
        read.folders,
        Some(vec![DeviceStorageFolder {
            path: vec!["Photos".into()],
            bytes: 100,
            held: 100
        }])
    );
}

#[test]
fn a_report_is_sent_again_only_when_it_moved() {
    let last = report(&json!({ "file_bytes": 100_000_000, "held_bytes": 50_000_000 }));
    assert!(!report_moved(
        &last,
        &report(&json!({ "file_bytes": 100_000_000, "held_bytes": 50_500_000 }))
    ));
    assert!(report_moved(
        &last,
        &report(&json!({ "file_bytes": 100_000_000, "held_bytes": 52_000_000 }))
    ));
    assert!(report_moved(
        &last,
        &report(&json!({ "file_bytes": 100_000_000, "held_bytes": 50_000_000, "files": 4 }))
    ));
    assert!(report_moved(
        &last,
        &report(
            &json!({ "file_bytes": 100_000_000, "held_bytes": 50_000_000, "seen": { "aa".repeat(32): 1 } })
        )
    ));
}

const BIG: usize = 2 * 1024 * 1024 + 1000;

struct Drive {
    tree: HashTree<MemoryStore>,
    store: Arc<MemoryStore>,
    root: Cid,
    large: Cid,
    photos: Cid,
}

async fn drive() -> Drive {
    let store = Arc::new(MemoryStore::new());
    let tree = HashTree::new(HashTreeConfig::new(store.clone()));
    let file = |name: &str, (cid, size): (Cid, u64)| {
        DirEntry::from_cid(name, &cid)
            .with_size(size)
            .with_link_type(LinkType::Blob)
    };
    let dir = |name: &str, cid: &Cid| DirEntry::from_cid(name, cid).with_link_type(LinkType::Dir);
    let small = tree.put_file(b"hello").await.unwrap();
    let bytes = (0..BIG)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect::<Vec<_>>();
    let large = tree.put_file(&bytes).await.unwrap();
    let trip = tree.put_file(b"a photo of a trip").await.unwrap();
    let year = tree
        .put_directory(vec![file("trip.jpg", trip)])
        .await
        .unwrap();
    let photos = tree
        .put_directory(vec![file("large.bin", large.clone()), dir("2024", &year)])
        .await
        .unwrap();
    let meta = tree
        .put_directory(vec![file("note", small.clone())])
        .await
        .unwrap();
    let root = tree
        .put_directory(vec![
            file("hello.txt", small),
            dir("Photos", &photos),
            dir(".hashtree", &meta),
            file(".DS_Store", tree.put_file(b"x").await.unwrap()),
        ])
        .await
        .unwrap();
    Drive {
        tree,
        store,
        root,
        large: large.0,
        photos,
    }
}

#[tokio::test]
async fn every_file_counts_and_only_parts_stored_here_are_held() {
    let drive = drive().await;
    let node = drive.tree.get_node(&drive.large).await.unwrap().unwrap();
    let small_piece = node.links.iter().find(|link| link.size == 1000).unwrap();
    drive.store.delete(&small_piece.hash).await.unwrap();

    let holding = measure_holding(
        &drive.tree,
        &drive.root,
        &mut HoldingCache::default(),
        None,
        1000,
    )
    .await;
    assert_eq!(holding.total.files, 3);
    assert_eq!(holding.total.bytes, 5 + BIG as u64 + 17);
    assert_eq!(holding.total.held_bytes, 5 + 2 * 1024 * 1024 + 17);
    assert_eq!(holding.total.held_files, 2);
    assert_eq!(holding.loose.files, 1);
    assert_eq!(holding.loose.held_bytes, 5);
    // 2024 holds 17 bytes of over 2 MB: too small to list.
    assert_eq!(
        holding.folders,
        vec![DeviceStorageFolder {
            path: vec!["Photos".into()],
            bytes: BIG as u64 + 17,
            held: 2 * 1024 * 1024 + 17,
        }]
    );
}

#[tokio::test]
async fn a_folder_counted_before_is_used_again_until_a_recount() {
    let drive = drive().await;
    let mut cache = HoldingCache::default();
    measure_holding(&drive.tree, &drive.root, &mut cache, None, 1000).await;
    // Content removed since: only a recount sees it.
    drive.store.delete(&drive.large.hash).await.unwrap();
    let again = measure_holding(&drive.tree, &drive.root, &mut cache, None, 2000).await;
    assert_eq!(again.total.held_bytes, 5 + BIG as u64 + 17);
    let recount = measure_holding(&drive.tree, &drive.root, &mut cache, Some(0), 3000).await;
    assert_eq!(recount.total.held_bytes, 5 + 17);
    assert_eq!(recount.total.held_files, 2);
}

#[tokio::test]
async fn a_folder_that_cannot_be_read_is_skipped_and_said() {
    let drive = drive().await;
    drive.store.delete(&drive.photos.hash).await.unwrap();
    let holding = measure_holding(
        &drive.tree,
        &drive.root,
        &mut HoldingCache::default(),
        None,
        1000,
    )
    .await;
    assert_eq!(holding.unread_folders, 1);
    assert_eq!(holding.total.files, 1);
    assert_eq!(holding.folders.len(), 0);
}

#[test]
fn the_storage_page_says_how_much_each_device_holds() {
    assert_eq!(
        holding_label(&report(&json!({ "held_files": 3, "held_bytes": 3000 }))),
        "All of it"
    );
    assert_eq!(holding_label(&report(&json!({}))), "1000 B · 33%");
    assert_eq!(
        holding_label(&report(&json!({ "held_bytes": 0, "held_files": 0 }))),
        "None of it"
    );
    assert_eq!(
        holding_label(&report(&json!({ "held_bytes": 1, "file_bytes": 1000 }))),
        "1 B · under 1%"
    );
    assert_eq!(
        holding_label(&report(
            &json!({ "held_bytes": 3_221_225_472_u64, "file_bytes": 6_442_450_944_u64 })
        )),
        "3.0 GB · 50%"
    );
    assert_eq!(
        space_label(&report(&json!({}))),
        "Uses 4.9 KB of 9.8 KB allowed"
    );
    let mut no_room = report(&json!({}));
    no_room.capacity_bytes = None;
    assert_eq!(space_label(&no_room), "Uses 4.9 KB");
}

fn row(name: &str, current: bool, report: Option<DeviceStorageReport>) -> view::DeviceRowInput {
    view::DeviceRowInput {
        pubkey: name.to_owned(),
        name: name.to_owned(),
        current,
        connected: false,
        report,
        heard_at: None,
    }
}

#[test]
fn the_storage_page_names_the_devices_that_hold_everything() {
    let all = report(&json!({ "held_files": 3, "held_bytes": 3000 }));
    assert_eq!(
        holders_sentence(&[row("Laptop", false, Some(all.clone()))]),
        "All of it is on Laptop."
    );
    assert_eq!(
        holders_sentence(&[row("Laptop", true, Some(all.clone()))]),
        "All of it is on this device."
    );
    assert_eq!(
        holders_sentence(&[row("Laptop", false, Some(report(&json!({}))))]),
        "No device holds all of it."
    );
    assert_eq!(
        summary(&[row("Mac", true, Some(all)), row("Phone", false, None)]),
        "Your Drive is 2.9 KB in 3 files. All of it is on this device."
    );
    assert_eq!(
        summary(&[row("Phone", false, None)]),
        "Measuring how much of your Drive is here…"
    );
}

#[test]
fn the_storage_page_lists_folders_loose_files_and_the_rest() {
    let lines = folder_lines(&report(&json!({
        "folders": [{ "path": ["Photos"], "bytes": 2000, "held": 500 }, { "path": ["Photos", "2024"], "bytes": 1500, "held": 0 }],
        "loose": { "files": 2, "bytes": 600, "held": 500 },
    })));
    let shown = lines
        .iter()
        .map(|line| (line.label.as_str(), line.depth, line.bytes, line.held))
        .collect::<Vec<_>>();
    assert_eq!(
        shown,
        vec![
            ("Photos", 0, 2000, 500),
            ("2024", 1, 1500, 0),
            ("2 files not in a folder", 0, 600, 500),
            ("Other folders", 0, 400, 0),
        ]
    );
    assert_eq!(lines[0].detail(), "500 B of 2.0 KB");
    assert_eq!(lines[2].detail(), "500 B of 600 B");
}

#[test]
fn the_storage_page_tells_whether_a_device_has_the_same_versions() {
    let [me, laptop, phone] = ["a", "b", "c"].map(|letter| letter.repeat(64));
    let held = BTreeMap::from([(me.clone(), 5), (laptop.clone(), 3), (phone.clone(), 7)]);
    let devices = [me.clone(), laptop.clone(), phone.clone()];
    let seen = |a: u64, c: u64| report(&json!({ "seen": { me.clone(): a, phone.clone(): c } }));
    assert_eq!(
        sync_state(&laptop, Some(&seen(5, 7)), &held, &devices),
        Some(SyncState::InSync)
    );
    assert_eq!(
        sync_state(&laptop, Some(&seen(4, 7)), &held, &devices),
        Some(SyncState::Behind)
    );
    assert_eq!(
        sync_state(&laptop, Some(&seen(5, 8)), &held, &devices),
        Some(SyncState::Ahead)
    );
    assert_eq!(
        sync_state(&laptop, Some(&seen(4, 8)), &held, &devices),
        Some(SyncState::Apart)
    );
    assert_eq!(
        sync_state(&laptop, Some(&report(&json!({}))), &held, &devices),
        None
    );
}
