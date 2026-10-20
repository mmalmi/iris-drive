use super::*;

async fn record_blossom_uploads(
    listener: tokio::net::TcpListener,
    uploads: Arc<Mutex<Vec<String>>>,
) {
    loop {
        let Ok((mut stream, _)) = listener.accept().await else {
            return;
        };
        let uploads = uploads.clone();
        tokio::spawn(async move {
            let mut request = Vec::new();
            let mut chunk = [0_u8; 4096];
            let mut expected_len = None;
            loop {
                let Ok(read) = stream.read(&mut chunk).await else {
                    return;
                };
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..read]);
                if expected_len.is_none()
                    && let Some(headers_end) =
                        request.windows(4).position(|part| part == b"\r\n\r\n")
                {
                    let headers = String::from_utf8_lossy(&request[..headers_end]);
                    let content_len = headers
                        .lines()
                        .find_map(|line| {
                            let (name, value) = line.split_once(':')?;
                            name.eq_ignore_ascii_case("content-length")
                                .then(|| value.trim().parse::<usize>().ok())
                                .flatten()
                        })
                        .unwrap_or(0);
                    expected_len = Some(headers_end + 4 + content_len);
                    if let Some(hash) = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("x-sha-256")
                            .then(|| value.trim().to_owned())
                    }) {
                        uploads
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push(hash);
                    }
                }
                if expected_len.is_some_and(|expected| request.len() >= expected) {
                    break;
                }
            }
            let _ = stream
                .write_all(
                    b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await;
        });
    }
}

fn spawn_rejecting_blossom_server() -> (String, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};

        listener.set_nonblocking(true).unwrap();
        let started = std::time::Instant::now();
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && started.elapsed() < std::time::Duration::from_secs(2) =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("Blossom reject fixture did not receive upload: {error}"),
            }
        };
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(2)))
            .unwrap();
        let mut request = [0_u8; 4096];
        let bytes_read = stream.read(&mut request).unwrap();
        assert!(
            bytes_read > 0,
            "Blossom reject fixture received an empty request"
        );
        stream
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
    });
    (url, server)
}

#[tokio::test]
async fn current_provider_root_uploads_live_blocks_before_relay_handoff() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let blossom_url = format!("http://{}", listener.local_addr().unwrap());
    let uploads = Arc::new(Mutex::new(Vec::new()));
    let server = tokio::spawn(record_blossom_uploads(listener, uploads.clone()));

    let dir = tempfile::tempdir().unwrap();
    let profile = iris_drive_core::Profile::create(dir.path(), Some("iPhone".to_owned())).unwrap();
    let mut initial_config = AppConfig {
        profile: Some(profile.state.clone()),
        ..AppConfig::default()
    };
    initial_config.upsert_drive(iris_drive_core::Drive::primary(
        profile.state.root_scope_id(),
    ));
    initial_config.save(config_path_in(dir.path())).unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(
        source.path().join("web-readable.txt"),
        b"native provider bytes",
    )
    .unwrap();
    let mut daemon = iris_drive_core::Daemon::open(dir.path()).unwrap();
    daemon.import_source_dir(source.path()).await.unwrap();
    drop(daemon);

    let mut config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    config.blossom_servers = vec![blossom_url];
    config.save(config_path_in(dir.path())).unwrap();
    let account = config.profile.as_ref().unwrap();
    let root = config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&account.app_key_pubkey)
        .unwrap()
        .clone();

    let report = crate::native_provider::upload_current_app_key_root_to_blossom(
        dir.path(),
        &config,
        &root.root_cid,
    )
    .await
    .unwrap();

    assert!(report.total_hashes > 0);
    assert_eq!(report.uploaded, report.total_hashes);
    let root_hash = hashtree_core::to_hex(&hashtree_core::Cid::parse(&root.root_cid).unwrap().hash);
    assert!(
        uploads
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .contains(&root_hash),
        "the published root block must be remotely available"
    );
    server.abort();
}

#[test]
fn provider_publish_releases_config_lock_and_keeps_exact_imported_root_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    mark_daemon_live(dir.path());

    let source = dir.path().join("publish-lock-probe.txt");
    std::fs::write(&source, b"exact provider snapshot").unwrap();
    let _probe = crate::native_provider::provider_publish_lock_probe_for_test(dir.path());
    let result = super::super::native_provider_write_json(
        &dir.path().display().to_string(),
        "publish-lock-probe.txt",
        &source.display().to_string(),
    );

    assert!(
        result["error"].as_str().unwrap_or_default().is_empty(),
        "unexpected provider write result: {result:#}"
    );
    assert_eq!(result["path"], "publish-lock-probe.txt");
    assert_eq!(result["publish"]["published_drive_root"], false);
    assert_eq!(
        result["publish"]["error"],
        "provider publish lock probe skipped network"
    );
    assert_eq!(
        result["publish"]["prepared_root_cid"], result["root_cid"],
        "provider response and prepared relay event must describe the same imported root"
    );
}

#[test]
fn provider_write_keeps_local_success_and_nested_publish_error_when_blossom_upload_fails() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    mark_daemon_live(dir.path());

    let (rejecting_blossom_url, rejecting_blossom) = spawn_rejecting_blossom_server();
    let mut config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    config.blossom_servers = vec![rejecting_blossom_url];
    config.save(config_path_in(dir.path())).unwrap();

    let source = dir.path().join("upload-failure.txt");
    std::fs::write(&source, b"local mutation survives upload failure").unwrap();
    let result = super::super::native_provider_write_json(
        &dir.path().display().to_string(),
        "upload-failure.txt",
        &source.display().to_string(),
    );
    rejecting_blossom.join().unwrap();

    assert!(
        result["error"].as_str().unwrap_or_default().is_empty(),
        "upload failure must not turn a persisted provider import into a local failure: {result:#}"
    );
    assert_eq!(result["path"], "upload-failure.txt");
    assert!(result["root_cid"].as_str().is_some());
    assert_eq!(result["file_count"], 1);
    assert_eq!(result["publish"]["published_drive_root"], false);
    assert!(
        result["publish"]["error"]
            .as_str()
            .unwrap_or_default()
            .contains("making provider root blocks available"),
        "unexpected nested publish failure: {result:#}"
    );
    let roundtrip = dir.path().join("upload-failure-roundtrip.txt");
    let read = super::super::native_provider_read_json(
        &dir.path().display().to_string(),
        "upload-failure.txt",
        &roundtrip.display().to_string(),
    );
    assert!(
        read["error"].as_str().unwrap_or_default().is_empty(),
        "persisted provider file was unreadable after upload failure: {read:#}"
    );
    assert_eq!(
        std::fs::read(roundtrip).unwrap(),
        b"local mutation survives upload failure"
    );
    let top_level_keys = result
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(
        top_level_keys,
        vec![
            "file_count",
            "path",
            "publish",
            "root_cid",
            "top_level_entries"
        ]
    );
    let publish_keys = result["publish"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(publish_keys, vec!["error", "published_drive_root"]);
}
