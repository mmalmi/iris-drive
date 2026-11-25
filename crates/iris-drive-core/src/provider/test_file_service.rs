//! A loopback Blossom file service for provider tests.

use std::net::Ipv4Addr;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, Uri};
use axum::response::Response;
use axum::routing::any;
use hashtree_core::{MemoryStore, Store, from_hex};
use tempfile::TempDir;

use crate::config::AppConfig;
use crate::paths::config_path_in;

/// Serves `blocks` and counts requests; a block it lacks is a 404.
pub(crate) struct FileService {
    pub(crate) url: String,
    requests: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl FileService {
    pub(crate) fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }
}

impl Drop for FileService {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) async fn file_service(blocks: MemoryStore) -> FileService {
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

pub(crate) fn use_file_service(dir: &TempDir, url: &str) {
    let mut config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    config.blossom_servers = vec![url.to_owned()];
    config.save(config_path_in(dir.path())).unwrap();
}
