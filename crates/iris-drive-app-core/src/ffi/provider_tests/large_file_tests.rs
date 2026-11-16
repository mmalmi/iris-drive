//! Large files reach the Drive whole through the paths the phones use: the
//! share sheet's import and the Files/SAF/photo-backup write.

use super::*;

fn video_bytes() -> Vec<u8> {
    let mut state: u32 = 11;
    (0..6 * 1024 * 1024 + 7)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 16).to_le_bytes()[0]
        })
        .collect()
}

fn read_back(data_dir: &str, path: &str, out: &Path) -> Vec<u8> {
    let read = super::super::native_provider_read_json(data_dir, path, &out.display().to_string());
    assert!(
        read["error"].as_str().unwrap_or_default().is_empty(),
        "read {path}: {read:#}"
    );
    std::fs::read(out).unwrap()
}

#[test]
fn a_large_shared_video_is_saved_whole_by_both_mobile_paths() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    mark_daemon_live(dir.path());
    let data_dir = dir.path().display().to_string();
    let bytes = video_bytes();
    let source = dir.path().join("staged.mov");
    std::fs::write(&source, &bytes).unwrap();
    let source = source.display().to_string();

    let shared =
        super::super::native_provider_import_shared_file_json(&data_dir, "Trip.mov", &source);
    assert!(
        shared["error"].as_str().unwrap_or_default().is_empty(),
        "{shared:#}"
    );
    let written = super::super::native_provider_write_json(&data_dir, "Videos/Trip.mov", &source);
    assert!(
        written["error"].as_str().unwrap_or_default().is_empty(),
        "{written:#}"
    );
    let again =
        super::super::native_provider_import_shared_file_json(&data_dir, "Trip.mov", &source);
    assert!(
        again["error"].as_str().unwrap_or_default().is_empty(),
        "{again:#}"
    );

    let list = super::super::native_provider_list_json(&data_dir);
    let sizes = list["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == "file")
        .map(|entry| {
            (
                entry["path"].as_str().unwrap().to_owned(),
                entry["size"].as_u64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    let len = bytes.len() as u64;
    assert_eq!(
        sizes,
        vec![
            ("Trip (2).mov".to_owned(), len),
            ("Trip.mov".to_owned(), len),
            ("Videos/Trip.mov".to_owned(), len),
        ]
    );
    for path in ["Trip.mov", "Trip (2).mov", "Videos/Trip.mov"] {
        assert!(
            read_back(&data_dir, path, &dir.path().join("out")) == bytes,
            "{path} differs"
        );
    }
}

#[test]
fn share_sheet_upload_survives_offline_extension_exit_then_opens_on_a_fresh_receiver() {
    use crate::ffi::tests::sharing_network::{LocalBlossomServer, LocalNostrRelay};
    use crate::native_provider::{publish_shared_files_json, stage_shared_file_json};
    use hashtree_core::{Cid, HashTree, HashTreeConfig, MemoryStore};

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let relay = runtime.block_on(LocalNostrRelay::spawn());
    // Multiple sequential block uploads exceed the former ten-second share-sheet timeout.
    let blossom = runtime.block_on(LocalBlossomServer::spawn_with_upload_delay(
        std::time::Duration::from_secs(2),
    ));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_str().unwrap();
    let mut owner = iris_drive_core::Profile::create(dir.path(), Some("Sender".into())).unwrap();
    let receiver = nostr_sdk::Keys::generate();
    owner
        .approve_app_key(&receiver.public_key().to_hex(), Some("Receiver".into()))
        .unwrap();
    let mut config = AppConfig {
        profile: Some(owner.state.clone()),
        relays: vec![relay.url.clone()],
        blossom_servers: vec![blossom.url.clone()],
        ..AppConfig::default()
    };
    config.upsert_drive(iris_drive_core::Drive::primary(owner.state.root_scope_id()));
    config.save(config_path_in(dir.path())).unwrap();
    // A playable single-frame QuickTime movie with a multi-chunk free-space atom.
    let mut movie = include_bytes!("one-frame.mov").to_vec();
    let padding = video_bytes();
    movie.extend_from_slice(&u32::try_from(padding.len() + 8).unwrap().to_be_bytes());
    movie.extend_from_slice(b"free");
    movie.extend(padding);
    let movie_file = dir.path().join("incoming.mov");
    std::fs::write(&movie_file, &movie).unwrap();

    // No app runtime or foreground daemon is running. The extension saves into a folder.
    blossom.set_uploads_offline(true);
    let staged = stage_shared_file_json(path, "Videos", "Trip.mov", movie_file.to_str().unwrap());
    assert_eq!(staged["path"], "Videos/Trip.mov", "{staged}");
    std::fs::remove_file(movie_file).unwrap();
    assert_eq!(
        publish_shared_files_json(path, true, || false)["pending"],
        true
    );
    assert_eq!(runtime.block_on(relay.events()).len(), 0);
    // A fresh background callback can finish even after interruption or an offline save.
    assert_eq!(
        publish_shared_files_json(path, true, || true)["pending"],
        true
    );
    blossom.set_uploads_offline(false);
    config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    config.sync_enabled = false;
    config.save(config_path_in(dir.path())).unwrap();
    assert_eq!(
        publish_shared_files_json(path, true, || false)["paused"],
        true
    );
    assert_eq!(runtime.block_on(blossom.blob_count()), 0);
    config.sync_enabled = true;
    config.save(config_path_in(dir.path())).unwrap();
    let published = publish_shared_files_json(path, true, || false);
    assert_eq!(published["ok"], true, "{published}");
    assert_eq!(
        publish_shared_files_json(path, false, || false)["pending"],
        false
    );

    runtime.block_on(async {
        let events = relay.events().await;
        assert_eq!(events.len(), 1, "the signed root reached the relay");
        let event = Event::from_json(events[0].to_string()).unwrap();
        let event =
            iris_drive_core::nostr_events::parse_drive_root_event_for_device(&event, &receiver)
                .expect("recipient can decrypt the published root");
        let root = Cid::parse(&event.3.root_cid).unwrap();
        let store = Arc::new(MemoryStore::new());
        let client = iris_drive_core::blossom_sync_client(receiver, &config.blossom_servers);
        iris_drive_core::blossom_sync::download_tree(store.clone(), &root, client)
            .await
            .unwrap();
        let tree = HashTree::new(HashTreeConfig::new(store));
        let cid = tree
            .resolve_path(&root, "Videos/Trip.mov")
            .await
            .unwrap()
            .unwrap();
        let downloaded_bytes = tree
            .read_file_range_cid(&cid, 0, None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(downloaded_bytes.len(), movie.len());
        assert_eq!(
            hashtree_core::sha256(&downloaded_bytes),
            hashtree_core::sha256(&movie)
        );
    });
}
