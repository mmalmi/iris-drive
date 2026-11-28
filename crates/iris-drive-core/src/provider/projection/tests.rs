use std::collections::{BTreeMap, HashSet};

use hashtree_core::{Cid, MemoryStore, Store};
use tempfile::TempDir;

use super::provider_projection;
use crate::config::{AppConfig, AppKeyRootRef, Drive};
use crate::daemon::PRIMARY_DRIVE_ID;
use crate::merge::walk_app_key_tree;
use crate::paths::config_path_in;
use crate::provider::test_file_service::{file_service, use_file_service};
use crate::root_meta::DriveRootMeta;
use crate::{Daemon, Identity, Profile};

const PEER_FOLDER: &str = "From Pixel";
const PEER_FILE: &str = "From Pixel/notes.txt";

/// This device's Drive after sync recorded a linked device's new root whose
/// blocks have not arrived: the root's own blocks are only on the file
/// service. Returns the blocks the service holds and the peer file's hash.
async fn drive_with_a_peer_root_not_stored_here() -> (TempDir, MemoryStore, [u8; 32]) {
    let dir = tempfile::tempdir().unwrap();
    let mut profile = Profile::create(dir.path(), Some("iPhone".to_owned())).unwrap();
    let peer = Identity::generate(dir.path().join("peer")).pubkey_hex();
    profile
        .approve_app_key(&peer, Some("Pixel".to_owned()))
        .unwrap();
    let mut config = AppConfig {
        profile: Some(profile.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(profile.state.root_scope_id()));
    config.blossom_servers.clear();
    config.save(config_path_in(dir.path())).unwrap();

    let local = tempfile::tempdir().unwrap();
    std::fs::write(local.path().join("mine.txt"), b"saved on this iPhone").unwrap();
    let mut daemon = Daemon::open(dir.path()).unwrap();
    let imported = daemon.import_source_dir(local.path()).await.unwrap();
    let local_root = Cid::parse(&imported.root_cid).unwrap();

    let files = tempfile::tempdir().unwrap();
    std::fs::create_dir(files.path().join(PEER_FOLDER)).unwrap();
    std::fs::write(files.path().join(PEER_FILE), b"written on the Pixel").unwrap();
    let meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.into(),
        app_key_pubkey: peer.clone(),
        app_key_seq: 1,
        dck_generation: 1,
        local_only: false,
        parents: vec![],
        observed: BTreeMap::new(),
        created_at: 101,
    };
    let peer_root = crate::indexer::index_dir_with_history_and_meta(
        daemon.tree(),
        files.path(),
        None,
        101,
        Some(&meta),
    )
    .await
    .unwrap();
    let (peer_files, _) = walk_app_key_tree(daemon.tree(), &peer_root).await.unwrap();
    let peer_file = peer_files
        .iter()
        .find(|f| f.path == PEER_FILE)
        .unwrap()
        .hash;

    let mut config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    config.drives[0].app_key_roots.insert(
        peer,
        AppKeyRootRef::from_meta(peer_root.to_string(), 101, &meta),
    );
    config.save(config_path_in(dir.path())).unwrap();

    let tree = daemon.tree();
    let kept: HashSet<_> = hashtree_core::diff::collect_hashes(tree, &local_root, 4)
        .await
        .unwrap()
        .into_iter()
        .collect();
    let store = tree.get_store();
    let blocks = MemoryStore::new();
    for hash in hashtree_core::diff::collect_hashes(tree, &peer_root, 4)
        .await
        .unwrap()
    {
        if !kept.contains(&hash) {
            blocks
                .put(hash, store.get(&hash).await.unwrap().unwrap())
                .await
                .unwrap();
            assert!(store.delete(&hash).await.unwrap());
        }
    }
    let missing = Daemon::open(dir.path())
        .unwrap()
        .provider_projection()
        .await
        .unwrap_err();
    assert!(
        missing.to_string().contains("Missing chunk"),
        "the fixture must apply a root whose blocks are not here: {missing}"
    );
    (dir, blocks, peer_file)
}

fn names_missing_chunk(message: &str) -> bool {
    // The share sheet keeps waiting for folders only on this exact shape.
    message
        .match_indices("Missing chunk: ")
        .any(|(at, prefix)| {
            let hash = &message[at + prefix.len()..];
            hash.len() >= 64 && hash[..64].bytes().all(|b| b.is_ascii_hexdigit())
        })
}

#[tokio::test]
async fn listing_downloads_the_folders_of_a_peer_root_not_stored_here() {
    let (dir, blocks, peer_file) = drive_with_a_peer_root_not_stored_here().await;
    let service = file_service(blocks).await;
    use_file_service(&dir, &service.url);
    let daemon = Daemon::open(dir.path()).unwrap();

    let (view, _, cached) = provider_projection(&daemon).await.unwrap();

    assert!(!cached);
    let mut paths: Vec<_> = view.view.files.iter().map(|f| f.path.as_str()).collect();
    paths.sort_unstable();
    assert_eq!(paths, ["From Pixel/notes.txt", "mine.txt"]);
    assert!(service.requests() > 0);
    assert!(
        !daemon.tree().get_store().has(&peer_file).await.unwrap(),
        "listing fetches folders, not the bytes of the files in them"
    );
    drop(service);
    let (view, _, cached) = Daemon::open(dir.path())
        .unwrap()
        .provider_projection()
        .await
        .unwrap();
    assert!(!cached, "downloaded folders must stay on the device");
    assert_eq!(view.file_count(), 2);
}

#[tokio::test]
async fn a_listing_that_cannot_download_keeps_the_missing_chunk_and_says_why() {
    let (dir, _blocks, _) = drive_with_a_peer_root_not_stored_here().await;
    let without_service = provider_projection(&Daemon::open(dir.path()).unwrap())
        .await
        .unwrap_err();
    let message = format!("{without_service:#}");
    assert!(names_missing_chunk(&message), "{message}");
    assert!(
        message.contains("no blossom servers configured"),
        "{message}"
    );

    let lacking = file_service(MemoryStore::new()).await;
    use_file_service(&dir, &lacking.url);
    let explicit_miss = provider_projection(&Daemon::open(dir.path()).unwrap())
        .await
        .unwrap_err();
    let message = format!("{explicit_miss:#}");
    assert!(names_missing_chunk(&message), "{message}");
    assert!(message.contains("404 Not Found"), "{message}");

    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    use_file_service(&dir, &url);
    let unreachable = provider_projection(&Daemon::open(dir.path()).unwrap())
        .await
        .unwrap_err();
    let message = format!("{unreachable:#}");
    assert!(names_missing_chunk(&message), "{message}");
    assert!(
        message.contains("error sending request for url")
            && message.contains(&url)
            && !message.contains("404 Not Found"),
        "an unreachable service must not read as an explicit miss: {message}"
    );
}
