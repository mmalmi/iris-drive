#[allow(clippy::wildcard_imports)]
use super::*;

#[tokio::test]
async fn gateway_path_routes_cannot_escape_the_request_host() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let work = tempdir().unwrap();
    std::fs::write(work.path().join("secret.txt"), b"private drive bytes").unwrap();
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    daemon.import_source_dir(work.path()).await.unwrap();
    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();

    for host in [
        "rebound.example",
        "untrusted.npub1attacker.iris.localhost",
        "main.drive.iris.localhost",
    ] {
        let response = http_get(server.local_addr(), host, "/drive/main/secret.txt").await;
        assert!(
            response.starts_with("HTTP/1.1 400 Bad Request"),
            "{host}: {response}"
        );
        assert!(!response.contains("private drive bytes"), "{response}");
    }
    let response = http_get(server.local_addr(), "localhost", "/drive/main/secret.txt").await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(response.contains("private drive bytes"), "{response}");
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_private_apis_reject_untrusted_content_hosts_and_origins() {
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
    let action = br#"{"type":"create_share","source_path":"Private","display_name":"Private"}"#;
    for host in [
        "untrusted.npub1attacker.iris.localhost",
        "main.drive.iris.localhost",
    ] {
        let share = http_request(
            server.local_addr(),
            "POST",
            host,
            SHARE_ACTION_API_PATH,
            &[("content-type", "application/json")],
            action,
        )
        .await;
        assert!(share.starts_with("HTTP/1.1 400 Bad Request"), "{share}");
        let calendar = http_get(server.local_addr(), host, "/caldav/calendar.ics").await;
        assert!(
            calendar.starts_with("HTTP/1.1 400 Bad Request"),
            "{calendar}"
        );
    }
    for origin in [
        "http://untrusted.npub1attacker.iris.localhost",
        "http://main.drive.iris.localhost",
        "http://drive.iris.to",
        "https://drive.iris.to:444",
        "https://localhost:80@attacker.example",
        "null",
    ] {
        let share = http_request(
            server.local_addr(),
            "POST",
            "localhost",
            SHARE_ACTION_API_PATH,
            &[("origin", origin), ("content-type", "application/json")],
            action,
        )
        .await;
        assert!(
            share.starts_with("HTTP/1.1 403 Forbidden"),
            "{origin}: {share}"
        );
        let calendar = http_request(
            server.local_addr(),
            "PUT",
            "localhost",
            "/caldav/calendars/iris/calendar/untrusted.ics",
            &[("origin", origin)],
            b"BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n",
        )
        .await;
        assert!(calendar.starts_with("HTTP/1.1 403 Forbidden"), "{calendar}");
    }
    let config = AppConfig::load_or_default(config_path_in(cfg_dir.path())).unwrap();
    assert_eq!(config.shared_folders.len(), 0);
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_runtime_http_rejects_cross_origin_requests() {
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
    let path = "/__iris/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    for origin in [
        "https://attacker.example",
        "null",
        "http://localhost",
        "http://video.npub1attacker.iris.localhost",
        &format!("https://{host}"),
        &format!("http://{host}:444"),
    ] {
        let response = http_request(
            server.local_addr(),
            "PUT",
            &host,
            path,
            &[("origin", origin)],
            b"untrusted",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 403 Forbidden"),
            "{origin}: {response}"
        );
    }
    let origin = format!("http://{host}");
    let response = http_request(
        server.local_addr(),
        "PUT",
        &host,
        path,
        &[("origin", &origin)],
        b"trusted",
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 201 Created"), "{response}");
    server.shutdown().await.unwrap();
    htree.shutdown().await;
}

#[tokio::test]
async fn gateway_runtime_websocket_rejects_cross_origin_upgrade() {
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
    let mut request = format!("ws://{}/ws", server.local_addr())
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert(HOST, HeaderValue::from_static("iris.localhost"));
    request
        .headers_mut()
        .insert(ORIGIN, HeaderValue::from_static("https://attacker.example"));
    let result = tokio_tungstenite::connect_async(request).await;
    assert!(
        matches!(result,
        Err(tokio_tungstenite::tungstenite::Error::Http(ref response))
            if response.status() == StatusCode::FORBIDDEN),
        "foreign-origin WebSocket must be rejected"
    );
    let mut request = format!("ws://{}/ws", server.local_addr())
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        ORIGIN,
        HeaderValue::from_str(&format!("http://{}", server.local_addr())).unwrap(),
    );
    let (mut socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            "trusted".into(),
        ))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        tokio_tungstenite::tungstenite::Message::Text("upstream:trusted".into())
    );
    socket.close(None).await.unwrap();
    server.shutdown().await.unwrap();
    htree.shutdown().await;
}

#[tokio::test]
async fn gateway_sandboxes_drive_documents_on_the_privileged_loopback_origin() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let work = tempdir().unwrap();
    std::fs::write(
        work.path().join("index.html"),
        b"<script>fetch('/api/iris-drive/share-action')</script>",
    )
    .unwrap();
    let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
    daemon.import_source_dir(work.path()).await.unwrap();
    let server = GatewayServer::bind_with_tree(
        cfg_dir.path(),
        daemon.tree_handle(),
        GatewayBind::loopback_v4(0),
    )
    .await
    .unwrap();
    let response = http_get(server.local_addr(), "localhost", "/drive/main/").await;
    assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
    assert!(
        response.contains("content-security-policy: sandbox\r\n"),
        "{response}"
    );
    server.shutdown().await.unwrap();
}

#[tokio::test]
async fn gateway_runtime_content_cannot_execute_on_a_shared_origin() {
    let cfg_dir = tempdir().unwrap();
    init_account_config(cfg_dir.path());
    let daemon = Daemon::open(cfg_dir.path()).unwrap();
    for content_type in ["text/html", "application/xhtml+xml", "image/svg+xml"] {
        let htree = fake_htree_daemon_with_content_type(
            "/htree/untrusted/index.html",
            "untrusted active content",
            content_type,
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
            "/htree/untrusted/index.html",
        )
        .await;
        assert!(
            response.starts_with("HTTP/1.1 403 Forbidden"),
            "{content_type}: {response}"
        );
        assert!(!response.contains("untrusted active content"), "{response}");
        server.shutdown().await.unwrap();
        htree.shutdown().await;
    }
}
