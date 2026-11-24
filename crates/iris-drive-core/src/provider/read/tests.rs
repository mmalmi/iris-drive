use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::Response;
use axum::routing::any;
use hashtree_core::{Cid, MemoryStore, Store, from_hex};
use hashtree_provider::{HashTreeProviderFs, ProviderFs};
use tempfile::TempDir;

use super::read_provider_file;
use crate::config::{AppConfig, Drive};
use crate::paths::config_path_in;
use crate::{Daemon, Profile};

const NAME: &str = "ios-joiner-fixture.txt";
const CONTENT: &[u8] = b"physical iOS joiner post-link provider write fixture";

/// A Blossom file service that serves `blocks` and counts requests.
struct FileService {
    url: String,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for FileService {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn file_service(blocks: MemoryStore) -> FileService {
    async fn get(
        State((blocks, requests)): State<(MemoryStore, Arc<AtomicUsize>)>,
        uri: Uri,
    ) -> Response {
        requests.fetch_add(1, Ordering::SeqCst);
        let name = uri.path().trim_start_matches('/');
        let hash = from_hex(name.strip_suffix(".bin").unwrap_or(name)).ok();
        let bytes = match hash {
            Some(hash) => blocks.get(&hash).await.unwrap(),
            None => None,
        };
        match bytes {
            Some(bytes) => Response::new(Body::from(bytes)),
            None => Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(Body::empty())
                .unwrap(),
        }
    }
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let requests = Arc::new(AtomicUsize::new(0));
    let app = Router::new()
        .fallback(any(get))
        .with_state((blocks, requests.clone()));
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    FileService {
        url,
        requests,
        task,
    }
}

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

fn use_file_service(dir: &TempDir, url: &str) {
    let mut config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    config.blossom_servers = vec![url.to_owned()];
    config.save(config_path_in(dir.path())).unwrap();
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
    assert!(service.requests.load(Ordering::SeqCst) > 0);
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
    let requests = service.requests.load(Ordering::SeqCst);

    let (_, bytes) = read_provider_file(dir.path(), &provider, NAME)
        .await
        .unwrap();

    assert_eq!(bytes, CONTENT);
    assert_eq!(service.requests.load(Ordering::SeqCst), requests);
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
