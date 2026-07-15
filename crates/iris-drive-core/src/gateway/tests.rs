#[allow(clippy::wildcard_imports)]
use super::*;
use axum::extract::ws::{Message as AxumWebSocketMessage, WebSocket, WebSocketUpgrade};
use futures::{SinkExt, StreamExt};
use nostr_sdk::ToBech32;
use std::path::Path;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;

use crate::config::Drive;
use crate::paths::config_path_in;
use crate::profile::Profile;
use tempfile::tempdir;

fn init_account_config(dir: &Path) {
    let account = Profile::create(dir, Some("gateway-test".into())).unwrap();
    let mut cfg = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(account.state.root_scope_id()));
    cfg.save(config_path_in(dir)).unwrap();
}

fn test_nhash() -> String {
    nhash_encode_full(&NHashData {
        hash: [8u8; 32],
        decrypt_key: Some([9u8; 32]),
    })
    .unwrap()
}

struct FakeHtreeDaemon {
    addr: String,
    shutdown_tx: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

impl FakeHtreeDaemon {
    async fn shutdown(self) {
        let _ = self.shutdown_tx.send(());
        let _ = self.handle.await;
    }
}

async fn fake_htree_daemon(expected_path: &str, body: &str) -> FakeHtreeDaemon {
    fake_htree_daemon_with_content_type(expected_path, body, "image/webp").await
}

async fn fake_htree_daemon_with_content_type(
    expected_path: &str,
    body: &str,
    content_type: &str,
) -> FakeHtreeDaemon {
    #[derive(Clone)]
    struct FakeState {
        expected_path: Arc<String>,
        body: Arc<String>,
        content_type: Arc<String>,
    }

    async fn handler(
        State(state): State<FakeState>,
        method: Method,
        uri: Uri,
        headers: HeaderMap,
    ) -> Response {
        if uri.path() != state.expected_path.as_str() {
            return text_response(
                StatusCode::NOT_FOUND,
                &format!("unexpected path: {}", uri.path()),
            );
        }
        if headers.get(RANGE).is_some_and(|value| value != "bytes=0-3") {
            return text_response(StatusCode::BAD_REQUEST, "unexpected range");
        }
        response_builder(StatusCode::OK, method == Method::HEAD)
            .header(CONTENT_TYPE, state.content_type.as_str())
            .body(if method == Method::HEAD {
                Body::empty()
            } else {
                Body::from(state.body.as_str().to_string())
            })
            .expect("response")
    }

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let state = FakeState {
        expected_path: Arc::new(expected_path.to_string()),
        body: Arc::new(body.to_string()),
        content_type: Arc::new(content_type.to_string()),
    };
    let app = Router::new().fallback(any(handler)).with_state(state);
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    FakeHtreeDaemon {
        addr: addr.to_string(),
        shutdown_tx,
        handle,
    }
}

async fn fake_echo_path_htree_daemon() -> FakeHtreeDaemon {
    async fn handler(method: Method, uri: Uri) -> Response {
        response_builder(StatusCode::OK, method == Method::HEAD)
            .header(CONTENT_TYPE, "text/plain")
            .body(if method == Method::HEAD {
                Body::empty()
            } else {
                Body::from(uri.path().to_string())
            })
            .expect("response")
    }

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().fallback(any(handler));
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    FakeHtreeDaemon {
        addr: addr.to_string(),
        shutdown_tx,
        handle,
    }
}

async fn fake_runtime_htree_daemon() -> FakeHtreeDaemon {
    async fn ws_echo(mut socket: WebSocket) {
        while let Some(Ok(message)) = socket.next().await {
            match message {
                AxumWebSocketMessage::Text(text) => {
                    let _ = socket
                        .send(AxumWebSocketMessage::Text(format!("upstream:{text}")))
                        .await;
                }
                AxumWebSocketMessage::Ping(bytes) => {
                    let _ = socket.send(AxumWebSocketMessage::Pong(bytes)).await;
                }
                AxumWebSocketMessage::Close(_) => break,
                _ => {}
            }
        }
    }

    async fn handler(
        ws: Option<WebSocketUpgrade>,
        method: Method,
        uri: Uri,
        body: Bytes,
    ) -> Response {
        if uri.path() == "/ws" {
            return match ws {
                Some(ws) => ws.on_upgrade(ws_echo),
                None => text_response(StatusCode::BAD_REQUEST, "missing websocket upgrade"),
            };
        }
        if method == Method::GET && uri.path() == "/api/stats" {
            return response_builder(StatusCode::OK, false)
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"total_dags":7,"total_bytes":42}"#))
                .expect("response");
        }
        if method == Method::PUT && uri.path().starts_with("/__iris/store/") {
            return response_builder(StatusCode::CREATED, false)
                .header(CONTENT_TYPE, "text/plain")
                .body(Body::from(format!(
                    "stored:{}",
                    String::from_utf8_lossy(&body)
                )))
                .expect("response");
        }
        text_response(
            StatusCode::NOT_FOUND,
            &format!("unexpected runtime path: {}", uri.path()),
        )
    }

    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = Router::new().fallback(any(handler));
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    FakeHtreeDaemon {
        addr: addr.to_string(),
        shutdown_tx,
        handle,
    }
}

#[tokio::test]
async fn base32_host_label_round_trips_hash() {
    let hash = [7u8; 32];
    let label = encode_immutable_host_label(&hash);
    assert_eq!(label.len(), 52);
    assert_eq!(decode_base32_hash(&label), Some(hash));
}

#[tokio::test]
async fn gateway_serves_current_primary_drive_root() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let work = tempdir().unwrap();
    std::fs::write(work.path().join("index.html"), b"hello gateway").unwrap();

    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    daemon.import_source_dir(work.path()).await.unwrap();

    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let response = http_get(server.local_addr(), "main.drive.iris.localhost", "/").await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("hello gateway"), "{response}");
    server.shutdown().await.unwrap();
}

async fn proxy_http_get(proxy_addr: SocketAddr, host: &str, port: u16, path: &str) -> String {
    let mut stream = TcpStream::connect(proxy_addr).await.unwrap();
    stream
        .write_all(
            format!(
                "GET http://{host}:{port}{path} HTTP/1.1\r\n\
                 Host: {host}:{port}\r\n\
                 Connection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response
}

#[tokio::test]
async fn gateway_proxy_forwards_absolute_form_iris_host() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    let work = tempdir().unwrap();
    std::fs::write(work.path().join("index.html"), b"hello proxied gateway").unwrap();
    daemon.import_source_dir(work.path()).await.unwrap();
    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let proxy = GatewayProxyServer::bind_for_gateway(server.local_addr())
        .await
        .unwrap();
    let mut stream = TcpStream::connect(proxy.local_addr()).await.unwrap();
    let gateway_port = server.local_addr().port();
    stream
        .write_all(
            format!(
                "GET http://main.drive.iris.localhost:{gateway_port}/ HTTP/1.1\r\n\
                 Host: main.drive.iris.localhost:{gateway_port}\r\n\
                 Connection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();

    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("hello proxied gateway"), "{response}");
    proxy.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_proxy_preserves_distinct_mutable_site_origins() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let htree = fake_echo_path_htree_daemon().await;
    let server = GatewayServer::bind_with_tree_and_htree_daemon(
        cfg_dir.path(),
        daemon.tree_handle(),
        htree.addr.clone(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let proxy = GatewayProxyServer::bind_for_gateway(server.local_addr())
        .await
        .unwrap();
    let port = server.local_addr().port();
    let maps_host = format!("maps.{IRIS_SITES_PORTAL_NPUB}.iris.localhost");
    let notes_host = format!("notes.{IRIS_SITES_PORTAL_NPUB}.iris.localhost");

    let maps_response = proxy_http_get(proxy.local_addr(), &maps_host, port, "/index.html").await;
    let notes_response = proxy_http_get(proxy.local_addr(), &notes_host, port, "/index.html").await;

    assert!(
        maps_response.starts_with("HTTP/1.1 200 OK"),
        "{maps_response}"
    );
    assert!(
        notes_response.starts_with("HTTP/1.1 200 OK"),
        "{notes_response}"
    );
    assert!(
        maps_response.contains(&format!("/htree/{IRIS_SITES_PORTAL_NPUB}/maps/index.html")),
        "{maps_response}"
    );
    assert!(
        notes_response.contains(&format!("/htree/{IRIS_SITES_PORTAL_NPUB}/notes/index.html")),
        "{notes_response}"
    );
    assert_ne!(maps_response, notes_response);

    proxy.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
    htree.shutdown().await;
}

#[tokio::test]
async fn gateway_proxy_tunnels_connect_iris_host() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    let work = tempdir().unwrap();
    std::fs::write(work.path().join("index.html"), b"hello tunnel").unwrap();
    daemon.import_source_dir(work.path()).await.unwrap();
    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let proxy = GatewayProxyServer::bind_for_gateway(server.local_addr())
        .await
        .unwrap();
    let mut stream = TcpStream::connect(proxy.local_addr()).await.unwrap();
    let gateway_port = server.local_addr().port();
    stream
        .write_all(
            format!(
                "CONNECT main.drive.iris.localhost:{gateway_port} HTTP/1.1\r\n\
                 Host: main.drive.iris.localhost:{gateway_port}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut connect_response = vec![0u8; 128];
    let read = stream.read(&mut connect_response).await.unwrap();
    let connect_response = String::from_utf8_lossy(&connect_response[..read]);
    assert!(
        connect_response.starts_with("HTTP/1.1 200 Connection Established"),
        "{connect_response}"
    );

    stream
        .write_all(
            format!(
                "GET / HTTP/1.1\r\n\
                 Host: main.drive.iris.localhost:{gateway_port}\r\n\
                 Connection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();

    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("hello tunnel"), "{response}");
    proxy.shutdown().await.unwrap();
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_serves_primary_share_shortcut_projection() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let work = tempdir().unwrap();
    std::fs::create_dir_all(work.path().join("Shared").join("keke")).unwrap();
    std::fs::write(
        work.path().join("Shared").join("keke").join("note.txt"),
        b"shared through gateway",
    )
    .unwrap();

    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    daemon.import_source_dir(work.path()).await.unwrap();
    crate::dispatch_share_action(
        cfg_dir.path(),
        crate::ShareAction::CreateShare {
            source_path: "Shared/keke".to_owned(),
            display_name: Some("keke".to_owned()),
        },
        2,
    )
    .unwrap();

    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let host = format!("main{DRIVE_HOST_SUFFIX}");
    let response = http_get(server.local_addr(), &host, "/keke/note.txt").await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("shared through gateway"), "{response}");
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_serves_immutable_hash_host() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let work = tempdir().unwrap();
    std::fs::write(work.path().join("app.js"), b"console.log('iris')").unwrap();

    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    let report = daemon.import_source_dir(work.path()).await.unwrap();
    let root = Cid::parse(&report.root_cid).unwrap();
    let host = format!(
        "{}.sites.iris.localhost",
        encode_immutable_host_label(&root.hash)
    );
    let path = format!("/app.js?k={}", to_hex(&root.key.unwrap()));

    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let response = http_get(server.local_addr(), &host, &path).await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(
        response.contains("content-type: text/javascript"),
        "{response}"
    );
    assert!(response.contains("console.log('iris')"), "{response}");
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_treats_nhash_file_path_as_filename_hint() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let nhash = test_nhash();
    let path = format!("/nhash/{nhash}/Aragorn.webp");
    let htree = fake_htree_daemon(&format!("/htree/{nhash}/Aragorn.webp"), "webp-bytes").await;

    let server = GatewayServer::bind_with_tree_and_htree_daemon(
        cfg_dir.path(),
        daemon.tree_handle(),
        htree.addr.clone(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let response = http_get(server.local_addr(), "localhost", &path).await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("content-type: image/webp"), "{response}");
    assert!(response.contains("webp-bytes"), "{response}");
    server.shutdown().await.unwrap();
    htree.shutdown().await;
}

#[tokio::test]
async fn gateway_accepts_split_nhash_hostname() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let nhash = test_nhash();
    let host = format!("{}.iris.localhost", split_dns_labels(&nhash));
    let htree = fake_htree_daemon(&format!("/htree/{nhash}/Aragorn.webp"), "webp-bytes").await;

    let server = GatewayServer::bind_with_tree_and_htree_daemon(
        cfg_dir.path(),
        daemon.tree_handle(),
        htree.addr.clone(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let response = http_get(server.local_addr(), &host, "/Aragorn.webp").await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("content-type: image/webp"), "{response}");
    assert!(response.contains("webp-bytes"), "{response}");
    server.shutdown().await.unwrap();
    htree.shutdown().await;
}

#[tokio::test]
async fn gateway_accepts_nhash_resolver_hostname() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let nhash = test_nhash();
    let htree = fake_htree_daemon(&format!("/htree/{nhash}/Aragorn.webp"), "webp-bytes").await;

    let server = GatewayServer::bind_with_tree_and_htree_daemon(
        cfg_dir.path(),
        daemon.tree_handle(),
        htree.addr.clone(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let response = http_get(
        server.local_addr(),
        LOCAL_NHASH_RESOLVER_HOST,
        &format!("/{nhash}/Aragorn.webp"),
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("content-type: image/webp"), "{response}");
    assert!(response.contains("webp-bytes"), "{response}");
    server.shutdown().await.unwrap();
    htree.shutdown().await;
}

#[tokio::test]
async fn gateway_proxies_nhash_to_hashtree_daemon() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let nhash = test_nhash();
    let host = format!("{}.iris.localhost", split_dns_labels(&nhash));
    let htree = fake_htree_daemon(&format!("/htree/{nhash}/Aragorn.webp"), "external webp").await;

    let server = GatewayServer::bind_with_tree_and_htree_daemon(
        cfg_dir.path(),
        daemon.tree_handle(),
        htree.addr.clone(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let response = http_get(server.local_addr(), &host, "/Aragorn.webp").await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("content-type: image/webp"), "{response}");
    assert!(response.contains("external webp"), "{response}");
    server.shutdown().await.unwrap();
    htree.shutdown().await;
}

#[tokio::test]
async fn gateway_root_host_redirects_to_mutable_portal_host() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();

    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let port = server.local_addr().port();
    let host = format!("{LOCAL_PORTAL_HOST}:{port}");

    let response = http_get(server.local_addr(), &host, "/").await;

    assert!(
        response.starts_with("HTTP/1.1 307 Temporary Redirect"),
        "{response}"
    );
    assert!(
        response.contains(&format!("location: {}", local_portal_url(port))),
        "{response}"
    );
    assert!(response.contains("cache-control: no-store"), "{response}");
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_proxies_mutable_site_host_to_hashtree_daemon() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let tree_name = "hashtree-cc-site";
    let htree = fake_htree_daemon(
        &format!("/htree/{IRIS_SITES_PORTAL_NPUB}/{tree_name}/app.js"),
        "mutable app",
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
    let host = format!("{tree_name}.{IRIS_SITES_PORTAL_NPUB}.iris.localhost");
    let response = http_get(server.local_addr(), &host, "/app.js").await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("mutable app"), "{response}");
    server.shutdown().await.unwrap();
    htree.shutdown().await;
}

#[tokio::test]
async fn gateway_proxies_portal_npub_path_to_hashtree_daemon() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let npub = nostr_sdk::Keys::generate()
        .public_key()
        .to_bech32()
        .unwrap();
    let htree = fake_htree_daemon(
        &format!("/htree/{npub}/releases%2Fhashtree/latest/install.sh"),
        "install script",
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
    let response = http_get(
        server.local_addr(),
        "iris.localhost",
        &format!("/{npub}/releases%2Fhashtree/latest/install.sh"),
    )
    .await;

    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("install script"), "{response}");
    server.shutdown().await.unwrap();
    htree.shutdown().await;
}

#[tokio::test]
async fn gateway_blocks_html_on_portal_npub_path() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let npub = nostr_sdk::Keys::generate()
        .public_key()
        .to_bech32()
        .unwrap();
    let htree = fake_htree_daemon_with_content_type(
        &format!("/htree/{npub}/releases%2Fhashtree/index.html"),
        "<!doctype html><script>localStorage.pwned = true;</script>",
        "text/html; charset=utf-8",
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
    let response = http_get(
        server.local_addr(),
        "iris.localhost",
        &format!("/{npub}/releases%2Fhashtree/index.html"),
    )
    .await;

    assert!(response.starts_with("HTTP/1.1 403 Forbidden"), "{response}");
    assert!(
        response.contains("HTML htree apps require an isolated iris.localhost origin"),
        "{response}"
    );
    assert!(!response.contains("localStorage.pwned"), "{response}");
    server.shutdown().await.unwrap();
    htree.shutdown().await;
}

#[tokio::test]
async fn gateway_proxies_same_origin_runtime_http_to_hashtree_daemon() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let htree = fake_runtime_htree_daemon().await;

    let server = GatewayServer::bind_with_tree_and_htree_daemon(
        cfg_dir.path(),
        daemon.tree_handle(),
        htree.addr.clone(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let host = format!("video.{IRIS_SITES_PORTAL_NPUB}.iris.localhost");

    let stats = http_get(server.local_addr(), &host, "/api/stats").await;
    assert!(stats.starts_with("HTTP/1.1 200 OK"), "{stats}");
    assert!(stats.contains(r#""total_dags":7"#), "{stats}");

    let put = http_request(
        server.local_addr(),
        "PUT",
        &host,
        "/__iris/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        &[("content-type", "application/octet-stream")],
        b"blob",
    )
    .await;
    assert!(put.starts_with("HTTP/1.1 201 Created"), "{put}");
    assert!(put.contains("stored:blob"), "{put}");

    server.shutdown().await.unwrap();
    htree.shutdown().await;
}

#[tokio::test]
async fn gateway_proxies_same_origin_runtime_websocket_to_hashtree_daemon() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let htree = fake_runtime_htree_daemon().await;

    let server = GatewayServer::bind_with_tree_and_htree_daemon(
        cfg_dir.path(),
        daemon.tree_handle(),
        htree.addr.clone(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let host = format!("video.{IRIS_SITES_PORTAL_NPUB}.iris.localhost");
    let mut request = format!("ws://{}/ws", server.local_addr())
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert(HOST, HeaderValue::from_str(&host).unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            "hello".into(),
        ))
        .await
        .unwrap();

    let message = socket.next().await.unwrap().unwrap();
    assert_eq!(
        message,
        tokio_tungstenite::tungstenite::Message::Text("upstream:hello".into())
    );

    server.shutdown().await.unwrap();
    htree.shutdown().await;
}

#[tokio::test]
async fn gateway_leaves_mutable_site_html_without_runtime_injection() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let tree_name = "video";
    let html = r#"<!doctype html><html><head><title>video</title></head><body><script type="module" src="/assets/app.js"></script></body></html>"#;
    let htree = fake_htree_daemon_with_content_type(
        &format!("/htree/{IRIS_SITES_PORTAL_NPUB}/{tree_name}/index.html"),
        html,
        "text/html; charset=utf-8",
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
    let host = format!("{tree_name}.{IRIS_SITES_PORTAL_NPUB}.iris.localhost");
    let response = http_get(server.local_addr(), &host, "/").await;

    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(
        !response.contains("window.__HTREE_SERVER_URL__"),
        "{response}"
    );
    assert!(response.contains(html), "{response}");

    server.shutdown().await.unwrap();
    htree.shutdown().await;
}

#[tokio::test]
async fn gateway_does_not_keep_sites_portal_alias() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();

    let response = http_get(server.local_addr(), "sites.iris.localhost", "/").await;

    assert!(
        response.starts_with("HTTP/1.1 400 Bad Request"),
        "{response}"
    );
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_without_htree_upstream_does_not_use_global_daemon() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let nhash = test_nhash();
    let host = format!("{}.iris.localhost", split_dns_labels(&nhash));

    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let response = http_get(server.local_addr(), &host, "/Aragorn.webp").await;
    assert!(
        response.starts_with("HTTP/1.1 502 Bad Gateway"),
        "{response}"
    );
    assert!(
        response.contains("hashtree daemon upstream is not configured"),
        "{response}"
    );
    server.shutdown().await.unwrap();
}

#[test]
fn local_nhash_url_uses_nhash_resolver_host() {
    let nhash = "nhash1qqsvmfqp5hk00w9nerl4x5009ce5z7gj480g0z4zhq2pkvxl0vezprs9yr0u7t0w95k937aldt699ax2u29lpev8y50ewpsllp5e5kv5ta6vk26rfge";
    let url = local_nhash_url(17_321, nhash, Some("Aragorn.webp"));
    assert_eq!(
        url,
        "http://nhash.iris.localhost:17321/nhash1qqsvmfqp5hk00w9nerl4x5009ce5z7gj480g0z4zhq2pkvxl0vezprs9yr0u7t0w95k937aldt699ax2u29lpev8y50ewpsllp5e5kv5ta6vk26rfge/Aragorn.webp"
    );
}

#[tokio::test]
async fn gateway_rejects_unknown_hosts() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let response = http_get(server.local_addr(), "example.com", "/").await;
    assert!(
        response.starts_with("HTTP/1.1 400 Bad Request"),
        "{response}"
    );
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_share_action_creates_share_with_core_projection() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();

    let body = serde_json::json!({
        "type": "create_share",
        "source_path": "Projects/Alpha",
        "display_name": "Alpha"
    })
    .to_string();
    let response = http_request(
        server.local_addr(),
        "POST",
        "localhost",
        "/api/iris-drive/share-action",
        &[("content-type", "application/json")],
        body.as_bytes(),
    )
    .await;

    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(
        response.contains("\"source_path\":\"Projects/Alpha\""),
        "{response}"
    );
    assert!(
        response.contains("\"display_name\":\"Alpha\""),
        "{response}"
    );
    assert!(response.contains("\"local_role\":\"admin\""), "{response}");

    let saved = AppConfig::load_or_default(config_path_in(cfg_dir.path())).unwrap();
    assert_eq!(saved.shared_folders.len(), 1);
    assert_eq!(saved.shared_folders[0].source_path, "Projects/Alpha");

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_share_action_get_returns_current_core_projection() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();

    let body = serde_json::json!({
        "type": "create_share",
        "source_path": "Projects/Alpha",
        "display_name": "Alpha"
    })
    .to_string();
    let created = http_request(
        server.local_addr(),
        "POST",
        "localhost",
        "/api/iris-drive/share-action",
        &[("content-type", "application/json")],
        body.as_bytes(),
    )
    .await;
    assert!(created.starts_with("HTTP/1.1 200 OK"), "{created}");

    let response = http_request(
        server.local_addr(),
        "GET",
        "localhost",
        "/api/iris-drive/share-action",
        &[],
        b"",
    )
    .await;

    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(
        response.contains("\"source_path\":\"Projects/Alpha\""),
        "{response}"
    );
    assert!(response.contains("\"shares\":[{"), "{response}");

    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_share_action_allows_drive_web_preflight_to_loopback() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();

    let response = http_request(
        server.local_addr(),
        "OPTIONS",
        "127.0.0.1",
        "/api/iris-drive/share-action",
        &[
            ("origin", "https://drive.iris.to"),
            ("access-control-request-method", "POST"),
        ],
        b"",
    )
    .await;

    assert!(
        response.starts_with("HTTP/1.1 204 No Content"),
        "{response}"
    );
    assert!(
        response.contains("access-control-allow-origin: https://drive.iris.to"),
        "{response}"
    );
    assert!(
        response.contains("access-control-allow-methods: GET, POST, OPTIONS"),
        "{response}"
    );

    server.shutdown().await.unwrap();
}

async fn http_get(addr: SocketAddr, host: &str, path: &str) -> String {
    http_request(addr, "GET", host, path, &[], b"").await
}

async fn http_request(
    addr: SocketAddr,
    method: &str,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8_lossy(&response).into_owned()
}
