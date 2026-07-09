#[allow(clippy::wildcard_imports)]
use super::*;
use crate::config::Drive;
use crate::paths::config_path_in;
use crate::profile::Profile;
use tempfile::tempdir;

struct FakeServer {
    addr: String,
    shutdown_tx: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

impl FakeServer {
    async fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        let _ = self.handle.await;
    }
}

fn init_account_config(dir: &std::path::Path) {
    let account = Profile::create(dir, Some("gateway-test".into())).unwrap();
    let mut cfg = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(account.state.root_scope_id()));
    cfg.save(config_path_in(dir)).unwrap();
}

async fn fake_resolving_htree_daemon(root: &Cid) -> FakeServer {
    let root_hash = to_hex(&root.hash);

    async fn handler(State(root_hash): State<Arc<String>>, method: Method, uri: Uri) -> Response {
        if method != Method::GET || !uri.path().starts_with("/api/resolve/") {
            return text_response(StatusCode::NOT_FOUND, "unexpected path");
        }
        response_builder(StatusCode::OK, false)
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(
                serde_json::json!({
                    "hash": root_hash.as_str(),
                    "cid": root_hash.as_str(),
                    "source": "test"
                })
                .to_string(),
            ))
            .expect("response")
    }

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new()
        .fallback(any(handler))
        .with_state(Arc::new(root_hash));
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    FakeServer {
        addr: addr.to_string(),
        shutdown_tx,
        handle,
    }
}

async fn fake_resolving_and_serving_htree_daemon(root: &Cid, expected_path: &str) -> FakeServer {
    #[derive(Clone)]
    struct FakeState {
        root_hash: String,
        expected_path: Arc<String>,
    }

    async fn handler(State(state): State<FakeState>, method: Method, uri: Uri) -> Response {
        if method == Method::GET && uri.path().starts_with("/api/resolve/") {
            return response_builder(StatusCode::OK, false)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "hash": state.root_hash.as_str(),
                        "cid": state.root_hash.as_str(),
                        "source": "test",
                    })
                    .to_string(),
                ))
                .expect("response");
        }
        if uri.path() != state.expected_path.as_str() {
            return text_response(StatusCode::NOT_FOUND, "unexpected path");
        }
        response_builder(StatusCode::OK, method == Method::HEAD)
            .header(CONTENT_TYPE, "text/html; charset=utf-8")
            .body(if method == Method::HEAD {
                Body::empty()
            } else {
                Body::from("<!doctype html><title>Iris Apps</title>")
            })
            .expect("response")
    }

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().fallback(any(handler)).with_state(FakeState {
        root_hash: to_hex(&root.hash),
        expected_path: Arc::new(expected_path.to_string()),
    });
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    FakeServer {
        addr: addr.to_string(),
        shutdown_tx,
        handle,
    }
}

async fn fake_blossom_server(store: MemoryStore) -> FakeServer {
    async fn handler(State(store): State<MemoryStore>, method: Method, uri: Uri) -> Response {
        if method != Method::GET {
            return text_response(StatusCode::METHOD_NOT_ALLOWED, "method not allowed");
        }
        let Some(hash) = uri
            .path()
            .trim_start_matches('/')
            .strip_suffix(".bin")
            .and_then(|value| from_hex(value).ok())
        else {
            return text_response(StatusCode::NOT_FOUND, "not found");
        };
        let Some(bytes) = store.get(&hash).await.unwrap() else {
            return text_response(StatusCode::NOT_FOUND, "not found");
        };
        response_builder(StatusCode::OK, false)
            .header(CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(bytes))
            .expect("response")
    }

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().fallback(any(handler)).with_state(store);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    FakeServer {
        addr: addr.to_string(),
        shutdown_tx,
        handle,
    }
}

async fn http_get(addr: SocketAddr, host: &str, path: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await.unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[tokio::test]
async fn gateway_falls_back_to_htree_daemon_when_public_blossom_site_path_misses() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let missing_root = Cid {
        hash: [42u8; 32],
        key: None,
    };
    let blossom = fake_blossom_server(MemoryStore::new()).await;

    let mut cfg = AppConfig::load_or_default(config_path_in(cfg_dir.path())).unwrap();
    cfg.blossom_servers = vec![format!("http://{}", blossom.addr)];
    cfg.save(config_path_in(cfg_dir.path())).unwrap();

    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let htree = fake_resolving_and_serving_htree_daemon(
        &missing_root,
        &format!("/htree/{IRIS_SITES_PORTAL_NPUB}/sites/index.html"),
    )
    .await;
    let server = GatewayServer::bind_with_tree_and_htree_daemon(
        cfg_dir.path(),
        daemon.tree_handle(),
        htree.addr.clone(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();

    let host = format!("sites.{IRIS_SITES_PORTAL_NPUB}.iris.localhost");
    let response = http_get(server.local_addr(), &host, "/").await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("Iris Apps"), "{response}");

    server.shutdown().await.unwrap();
    htree.shutdown().await;
    blossom.shutdown().await;
}

#[tokio::test]
async fn gateway_serves_public_mutable_site_paths_from_resolved_blossom_root() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());

    let source_store = MemoryStore::new();
    let source_tree = HashTree::new(HashTreeConfig::new(Arc::new(source_store.clone())).public());
    let (index_cid, _) = source_tree.put(b"hello public site").await.unwrap();
    let root = source_tree
        .put_directory(vec![
            hashtree_core::DirEntry::from_cid("index.html", &index_cid)
                .with_link_type(LinkType::File),
        ])
        .await
        .unwrap();
    let blossom = fake_blossom_server(source_store.clone()).await;

    let mut cfg = AppConfig::load_or_default(config_path_in(cfg_dir.path())).unwrap();
    cfg.blossom_servers = vec![format!("http://{}", blossom.addr)];
    cfg.save(config_path_in(cfg_dir.path())).unwrap();

    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let htree = fake_resolving_htree_daemon(&root).await;
    let server = GatewayServer::bind_with_tree_and_htree_daemon(
        cfg_dir.path(),
        daemon.tree_handle(),
        htree.addr.clone(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();

    let host = format!("sites.{IRIS_SITES_PORTAL_NPUB}.iris.localhost");
    let response = http_get(server.local_addr(), &host, "/").await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("content-type: text/html"), "{response}");
    assert!(response.contains("hello public site"), "{response}");

    server.shutdown().await.unwrap();
    htree.shutdown().await;
    blossom.shutdown().await;
}
