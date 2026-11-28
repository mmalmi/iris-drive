//! Repeatable Blossom upload/download tests and scaling probe.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::extract::{Path as AxumPath, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Json, Router};
use base64::Engine;
use hashtree_core::{HashTree, HashTreeConfig, MemoryStore, Store, sha256, to_hex};
use hashtree_fs::FsBlobStore;
use nostr_sdk::Keys;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use super::*;
use crate::indexer::index_dir_with_history_and_meta;

#[derive(Clone)]
struct MockBlossomState {
    blobs: Arc<Mutex<HashMap<String, Vec<u8>>>>,
    delay: Duration,
    supports_upload_check: bool,
    puts: Arc<AtomicUsize>,
    heads: Arc<AtomicUsize>,
    gets: Arc<AtomicUsize>,
    checks: Arc<AtomicUsize>,
    put_bytes: Arc<AtomicUsize>,
}

struct MockBlossom {
    url: String,
    state: MockBlossomState,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for MockBlossom {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Deserialize)]
struct CheckReq {
    hashes: Vec<String>,
}

#[derive(Serialize)]
struct CheckResp {
    count: usize,
    present: String,
}

impl MockBlossom {
    async fn spawn(delay: Duration, supports_upload_check: bool) -> Self {
        let state = MockBlossomState {
            blobs: Arc::new(Mutex::new(HashMap::new())),
            delay,
            supports_upload_check,
            puts: Arc::default(),
            heads: Arc::default(),
            gets: Arc::default(),
            checks: Arc::default(),
            put_bytes: Arc::default(),
        };
        let app = Router::new()
            .route(
                "/upload",
                put(handle_put).layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024)),
            )
            .route("/upload/check", post(handle_check))
            .route("/:name", get(handle_get).head(handle_head))
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url: format!("http://{addr}"),
            state,
            task,
        }
    }

    fn reset_counts(&self) {
        self.state.puts.store(0, Ordering::SeqCst);
        self.state.heads.store(0, Ordering::SeqCst);
        self.state.gets.store(0, Ordering::SeqCst);
        self.state.checks.store(0, Ordering::SeqCst);
        self.state.put_bytes.store(0, Ordering::SeqCst);
    }

    fn counts(&self) -> (usize, usize, usize, usize, usize) {
        (
            self.state.checks.load(Ordering::SeqCst),
            self.state.heads.load(Ordering::SeqCst),
            self.state.puts.load(Ordering::SeqCst),
            self.state.gets.load(Ordering::SeqCst),
            self.state.put_bytes.load(Ordering::SeqCst),
        )
    }
}

async fn handle_put(State(state): State<MockBlossomState>, body: Bytes) -> Response {
    if !state.delay.is_zero() {
        tokio::time::sleep(state.delay).await;
    }
    state.puts.fetch_add(1, Ordering::SeqCst);
    state.put_bytes.fetch_add(body.len(), Ordering::SeqCst);
    let hash = to_hex(&sha256(&body));
    let existed = state
        .blobs
        .lock()
        .await
        .insert(hash, body.to_vec())
        .is_some();
    if existed {
        StatusCode::OK.into_response()
    } else {
        StatusCode::CREATED.into_response()
    }
}

async fn handle_check(
    State(state): State<MockBlossomState>,
    Json(req): Json<CheckReq>,
) -> Response {
    if !state.delay.is_zero() {
        tokio::time::sleep(state.delay).await;
    }
    if !state.supports_upload_check {
        return StatusCode::NOT_FOUND.into_response();
    }
    state.checks.fetch_add(1, Ordering::SeqCst);
    let blobs = state.blobs.lock().await;
    let mut bits = vec![0u8; req.hashes.len().div_ceil(8)];
    for (idx, hash) in req.hashes.iter().enumerate() {
        if blobs.contains_key(hash) {
            bits[idx / 8] |= 1 << (idx % 8);
        }
    }
    let resp = CheckResp {
        count: req.hashes.len(),
        present: base64::engine::general_purpose::STANDARD.encode(bits),
    };
    (StatusCode::OK, Json(resp)).into_response()
}

async fn handle_head(
    State(state): State<MockBlossomState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    if !state.delay.is_zero() {
        tokio::time::sleep(state.delay).await;
    }
    state.heads.fetch_add(1, Ordering::SeqCst);
    let hash = name.strip_suffix(".bin").unwrap_or(&name);
    let blobs = state.blobs.lock().await;
    if let Some(bytes) = blobs.get(hash) {
        let mut headers = HeaderMap::new();
        headers.insert("content-type", "application/octet-stream".parse().unwrap());
        headers.insert("content-length", bytes.len().to_string().parse().unwrap());
        (StatusCode::OK, headers).into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

async fn handle_get(
    State(state): State<MockBlossomState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    if !state.delay.is_zero() {
        tokio::time::sleep(state.delay).await;
    }
    state.gets.fetch_add(1, Ordering::SeqCst);
    let hash = name.strip_suffix(".bin").unwrap_or(&name);
    let blobs = state.blobs.lock().await;
    if let Some(bytes) = blobs.get(hash) {
        (StatusCode::OK, bytes.clone()).into_response()
    } else {
        StatusCode::NOT_FOUND.into_response()
    }
}

#[tokio::test]
#[ignore = "manual scaling probe; run with --ignored --nocapture"]
async fn blossom_tree_sync_scaling_probe() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FsBlobStore::new(dir.path().join("blocks")).unwrap());
    let tree = HashTree::new(HashTreeConfig::new(store));

    let src = tempfile::tempdir().unwrap();
    for d in 0..10 {
        std::fs::create_dir(src.path().join(format!("dir_{d:02}"))).unwrap();
    }
    for i in 0..100 {
        let content = vec![u8::try_from(i % 251).unwrap(); 4096];
        std::fs::write(
            src.path().join(format!("dir_{:02}/f_{i:03}.bin", i % 10)),
            content,
        )
        .unwrap();
    }
    // 32 MiB video (16 x 2 MiB chunks) with distinct chunks
    let mut video = vec![0u8; 32 * 1024 * 1024];
    let mut prng: u32 = 17;
    for chunk in video.as_chunks_mut::<4>().0 {
        prng = prng.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        *chunk = prng.to_le_bytes();
    }
    std::fs::write(src.path().join("video.mov"), &video).unwrap();

    let root1 = index_dir_with_history_and_meta(&tree, src.path(), None, 100, None)
        .await
        .unwrap();
    std::fs::write(src.path().join("dir_00/f_000.bin"), b"edited single file").unwrap();
    let root2 = index_dir_with_history_and_meta(&tree, src.path(), Some(&root1), 101, None)
        .await
        .unwrap();

    let keys = Keys::generate();
    let delay = Duration::from_millis(20);

    for (label, supports_check) in [("standard_blossom", false), ("upload_check_blossom", true)] {
        let server = MockBlossom::spawn(delay, supports_check).await;
        let client = crate::blossom_sync_client(keys.clone(), std::slice::from_ref(&server.url));

        let t0 = Instant::now();
        let initial = upload_tree(&tree, &root1, &client).await.unwrap();
        let initial_ms = t0.elapsed().as_millis();
        let (checks, heads, puts, _, put_bytes) = server.counts();
        println!(
            "{label} initial_upload: ms={initial_ms} total={} uploaded={} already={} checks={checks} heads={heads} puts={puts} put_bytes={put_bytes}",
            initial.total_hashes, initial.uploaded, initial.already_present
        );

        server.reset_counts();
        let t0 = Instant::now();
        let incr = upload_tree(&tree, &root2, &client).await.unwrap();
        let incr_ms = t0.elapsed().as_millis();
        let (checks, heads, puts, _, put_bytes) = server.counts();
        println!(
            "{label} incremental_edit: ms={incr_ms} total={} uploaded={} already={} checks={checks} heads={heads} puts={puts} put_bytes={put_bytes}",
            incr.total_hashes, incr.uploaded, incr.already_present
        );

        if !supports_check {
            server.reset_counts();
            let dl_dir = tempfile::tempdir().unwrap();
            let dl_store = Arc::new(FsBlobStore::new(dl_dir.path().join("blocks")).unwrap());
            let t0 = Instant::now();
            let dl = download_tree(dl_store, &root2, client.clone())
                .await
                .unwrap();
            let dl_ms = t0.elapsed().as_millis();
            let (_, _, _, gets, _) = server.counts();
            println!(
                "{label} download_tree: ms={dl_ms} total={} fetched={} already={} gets={gets}",
                dl.total_hashes, dl.fetched, dl.already_local
            );
        }
    }

    // Also measure local-only hash collection with MemoryStore read counter.
    let _ = MemoryStore::new();
}

#[tokio::test]
async fn upload_tree_renews_existing_blocks_and_reports_new_content() {
    let store = Arc::new(MemoryStore::new());
    let tree = HashTree::new(HashTreeConfig::new(store.clone()));

    let src = tempfile::tempdir().unwrap();
    for i in 0..10 {
        std::fs::write(
            src.path().join(format!("note_{i:02}.txt")),
            format!("content-{i:02}"),
        )
        .unwrap();
    }
    let root1 = index_dir_with_history_and_meta(&tree, src.path(), None, 10, None)
        .await
        .unwrap();
    std::fs::write(src.path().join("note_00.txt"), b"updated-content").unwrap();
    let root2 = index_dir_with_history_and_meta(&tree, src.path(), Some(&root1), 11, None)
        .await
        .unwrap();

    let server = MockBlossom::spawn(Duration::ZERO, true).await;
    let client = crate::blossom_sync_client(Keys::generate(), std::slice::from_ref(&server.url));

    let initial = upload_tree(&tree, &root1, &client).await.unwrap();
    assert!(initial.total_hashes >= 11);
    assert_eq!(initial.uploaded, initial.total_hashes);
    assert_eq!(initial.already_present, 0);

    server.reset_counts();
    let incr = upload_tree(&tree, &root2, &client).await.unwrap();
    let (checks, heads, puts, _, _) = server.counts();
    assert_eq!(checks, 0, "an existence check cannot establish ownership");
    assert_eq!(heads, 0);
    assert_eq!(
        puts, incr.total_hashes,
        "all live blocks must be submitted with the backup identity"
    );
    assert!(
        incr.uploaded < 6,
        "expected only changed blocks, got {}",
        incr.uploaded
    );
    assert!(incr.already_present >= 9);

    // A remote copy must not hide a missing local source block.
    let entries = tree.list_directory(&root2).await.unwrap();
    let leaf = entries
        .iter()
        .find(|entry| entry.name == "note_01.txt")
        .unwrap();
    assert!(store.delete(&leaf.hash).await.unwrap());
    let err = upload_tree(&tree, &root2, &client).await.unwrap_err();
    assert!(
        matches!(err, BlossomSyncError::MissingLocally(_)),
        "expected MissingLocally, got {err:?}"
    );
}

#[tokio::test]
async fn upload_and_download_tree_round_trips_fanout_directory() {
    let store = Arc::new(MemoryStore::new());
    let tree = HashTree::new(HashTreeConfig::new(store));

    // >174 files in a single directory triggers LinkType::Fanout pages.
    let src = tempfile::tempdir().unwrap();
    let photos = src.path().join("Photos");
    std::fs::create_dir(&photos).unwrap();
    for i in 0..200 {
        std::fs::write(
            photos.join(format!("img_{i:04}.jpg")),
            format!("jpeg-body-{i:04}"),
        )
        .unwrap();
    }
    let root = index_dir_with_history_and_meta(&tree, src.path(), None, 20, None)
        .await
        .unwrap();

    let server = MockBlossom::spawn(Duration::ZERO, true).await;
    let client = crate::blossom_sync_client(Keys::generate(), std::slice::from_ref(&server.url));

    let uploaded = upload_tree(&tree, &root, &client).await.unwrap();
    assert!(uploaded.total_hashes >= 204);
    assert_eq!(uploaded.uploaded, uploaded.total_hashes);

    let dest_store = Arc::new(MemoryStore::new());
    let downloaded = download_tree(dest_store.clone(), &root, client)
        .await
        .unwrap();
    assert_eq!(downloaded.fetched, uploaded.total_hashes);
    assert_eq!(downloaded.already_local, 0);

    let dest_tree = HashTree::new(HashTreeConfig::new(dest_store));
    let root_entries = dest_tree.list_directory(&root).await.unwrap();
    let photos_entry = root_entries.iter().find(|e| e.name == "Photos").unwrap();
    let photo_entries = dest_tree
        .list_directory(&Cid {
            hash: photos_entry.hash,
            key: photos_entry.key,
        })
        .await
        .unwrap();
    assert_eq!(photo_entries.len(), 200);
    let sample = photo_entries
        .iter()
        .find(|e| e.name == "img_0199.jpg")
        .unwrap();
    let bytes = dest_tree
        .get(
            &Cid {
                hash: sample.hash,
                key: sample.key,
            },
            None,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(bytes, b"jpeg-body-0199");
}

#[tokio::test]
async fn upload_tree_succeeds_without_upload_check_endpoint() {
    let store = Arc::new(MemoryStore::new());
    let tree = HashTree::new(HashTreeConfig::new(store));

    let src = tempfile::tempdir().unwrap();
    for i in 0..12 {
        std::fs::write(
            src.path().join(format!("f_{i:02}.txt")),
            format!("v-{i:02}"),
        )
        .unwrap();
    }
    let root = index_dir_with_history_and_meta(&tree, src.path(), None, 30, None)
        .await
        .unwrap();

    let server = MockBlossom::spawn(Duration::ZERO, false).await;
    let client = crate::blossom_sync_client(Keys::generate(), std::slice::from_ref(&server.url));

    let report = upload_tree(&tree, &root, &client).await.unwrap();
    assert_eq!(report.uploaded, report.total_hashes);
    let (checks, _, puts, _, _) = server.counts();
    assert_eq!(checks, 0);
    assert_eq!(puts, report.total_hashes);

    let dest_store = Arc::new(MemoryStore::new());
    let dl = download_tree(dest_store, &root, client).await.unwrap();
    assert_eq!(dl.fetched, report.total_hashes);
}

#[tokio::test]
async fn backup_claims_cached_blocks_before_eviction_and_restores_every_file() {
    use hashtree_cli::{HashtreeServer, HashtreeStore};
    use hashtree_config::StorageBackend;
    use hashtree_core::DirEntry;

    let keys = Keys::generate();
    let owner = keys.public_key().to_bytes();
    let dir = tempfile::tempdir().unwrap();
    let server_store = Arc::new(
        HashtreeStore::with_options_and_backend(
            dir.path(),
            None,
            1024 * 1024,
            true,
            &StorageBackend::Fs,
        )
        .unwrap(),
    );
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let server = HashtreeServer::new(server_store.clone(), addr.to_string())
        .with_public_writes(false)
        .with_allowed_pubkeys([keys.public_key().to_hex()].into_iter().collect());
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(server.run_with_listener_until(listener, async {
        let _ = stopped.await;
    }));
    let url = format!("http://{addr}");
    let client = crate::blossom_sync_client(keys, std::slice::from_ref(&url));

    let local = Arc::new(MemoryStore::new());
    let tree = HashTree::new(HashTreeConfig::new(local.clone()));
    let files: Vec<Vec<u8>> = (0u8..10)
        .map(|n| vec![n; if n == 0 { 300 * 1024 } else { 80 }])
        .collect();
    let mut entries = Vec::new();
    for (i, bytes) in files.iter().enumerate() {
        let (cid, size) = tree.put(bytes).await.unwrap();
        let stored = local.get(&cid.hash).await.unwrap().unwrap();
        server_store.put_cached_blob(&stored).unwrap();
        assert!(!server_store.is_blob_owner(&cid.hash, &owner).unwrap());
        entries.push(DirEntry::from_cid(i.to_string(), &cid).with_size(size));
    }
    let root = tree.put_directory(entries).await.unwrap();
    let live = collect_live_sync_hashes(&tree, &root, 4).await.unwrap();
    let control = b"cache-only eviction witness";
    server_store.put_cached_blob(control).unwrap();

    let report = upload_tree(&tree, &root, &client).await.unwrap();
    let owned = live
        .iter()
        .filter(|hash| server_store.is_blob_owner(hash, &owner).unwrap())
        .count();
    assert_eq!(report.total_hashes, live.len());
    // Use the store's quota eviction directly so the test observes completion
    // of one ordinary disposable-cache sweep, without racing repeated inserts.
    let evicted = server_store.make_room_for_cached_blob(1024 * 1024).unwrap();
    assert!(evicted > 0);
    assert!(!server_store.blob_exists(&sha256(control)).unwrap());
    let fresh = Arc::new(MemoryStore::new());
    let restored = download_tree(fresh.clone(), &root, client).await;
    let _ = stop.send(());
    task.await.unwrap().unwrap();
    assert!(
        restored.is_ok(),
        "restoring after eviction with {owned}/{} blocks owned: {restored:?}",
        live.len()
    );
    assert_eq!(owned, live.len());
    assert!(
        live.iter()
            .all(|hash| server_store.is_blob_owner(hash, &owner).unwrap())
    );
    let reader = HashTree::new(HashTreeConfig::new(fresh));
    for (i, expected) in files.iter().enumerate() {
        let cid = reader
            .resolve(&root, &i.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            reader.get(&cid, None).await.unwrap().as_ref(),
            Some(expected)
        );
    }
}
