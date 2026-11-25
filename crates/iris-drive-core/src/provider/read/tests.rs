use hashtree_core::{Cid, MemoryStore, Store};
use hashtree_provider::{HashTreeProviderFs, ProviderFs};
use tempfile::TempDir;

use super::read_provider_file;
use crate::config::{AppConfig, Drive};
use crate::paths::config_path_in;
use crate::provider::test_file_service::{file_service, use_file_service};
use crate::{Daemon, Profile};

const NAME: &str = "ios-joiner-fixture.txt";
const CONTENT: &[u8] = b"physical iOS joiner post-link provider write fixture";

/// A Drive whose listing has `NAME` but none of that file's own blocks, as
/// after a sync that stored the directory and was stopped before the bytes.
/// Returns the blocks the file needs.
async fn drive_listing_a_file_without_its_bytes() -> (TempDir, MemoryStore) {
    let dir = tempfile::tempdir().unwrap();
    let profile = Profile::create(dir.path(), Some("Pixel".to_owned())).unwrap();
    let mut config = AppConfig {
        profile: Some(profile.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(profile.state.root_scope_id()));
    config.blossom_servers.clear();
    config.save(config_path_in(dir.path())).unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join(NAME), CONTENT).unwrap();
    let mut daemon = Daemon::open(dir.path()).unwrap();
    daemon.import_source_dir(source.path()).await.unwrap();

    let provider = provider_in(&daemon).await;
    let item = provider.item(&NAME.to_owned()).await.unwrap();
    let file = Cid::parse(&item.version).unwrap();
    let local = daemon.tree().get_store().clone();
    let blocks = MemoryStore::new();
    for hash in hashtree_core::diff::collect_hashes(daemon.tree(), &file, 4)
        .await
        .unwrap()
    {
        let block = local.get(&hash).await.unwrap().unwrap();
        blocks.put(hash, block).await.unwrap();
        assert!(local.delete(&hash).await.unwrap());
    }
    assert!(
        provider.read(&item.id, 0, item.size).await.is_err(),
        "the fixture must list the file without its bytes"
    );
    (dir, blocks)
}

async fn provider_in(daemon: &Daemon) -> HashTreeProviderFs<hashtree_fs::FsBlobStore> {
    let (_, visible, _) = daemon.provider_projection().await.unwrap();
    HashTreeProviderFs::open(daemon.tree_handle(), visible.root_cid)
        .await
        .unwrap()
}

#[tokio::test]
async fn opening_a_listed_file_downloads_its_missing_bytes() {
    let (dir, blocks) = drive_listing_a_file_without_its_bytes().await;
    let service = file_service(blocks).await;
    use_file_service(&dir, &service.url);
    let daemon = Daemon::open(dir.path()).unwrap();
    let provider = provider_in(&daemon).await;

    let (item, bytes) = read_provider_file(dir.path(), &provider, NAME)
        .await
        .unwrap();

    assert_eq!(bytes, CONTENT);
    assert!(service.requests() > 0);
    drop(service);
    assert_eq!(
        provider.read(&item.id, 0, item.size).await.unwrap(),
        CONTENT,
        "downloaded bytes must stay on the device"
    );
}

#[tokio::test]
async fn a_stored_file_is_read_without_the_file_service() {
    let (dir, blocks) = drive_listing_a_file_without_its_bytes().await;
    let service = file_service(blocks).await;
    use_file_service(&dir, &service.url);
    let daemon = Daemon::open(dir.path()).unwrap();
    let provider = provider_in(&daemon).await;
    read_provider_file(dir.path(), &provider, NAME)
        .await
        .unwrap();
    let requests = service.requests();

    let (_, bytes) = read_provider_file(dir.path(), &provider, NAME)
        .await
        .unwrap();

    assert_eq!(bytes, CONTENT);
    assert_eq!(service.requests(), requests);
}

#[tokio::test]
async fn an_unreachable_file_says_it_is_not_stored_and_why() {
    let (dir, _blocks) = drive_listing_a_file_without_its_bytes().await;
    let daemon = Daemon::open(dir.path()).unwrap();
    let provider = provider_in(&daemon).await;

    let error = read_provider_file(dir.path(), &provider, NAME)
        .await
        .unwrap_err();

    let message = format!("{error:#}");
    assert!(
        message.contains(&format!("{NAME} is not stored on this device"))
            && message.contains("no blossom servers configured"),
        "{message}"
    );
}
