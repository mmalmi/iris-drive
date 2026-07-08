#[allow(clippy::wildcard_imports)]
use super::*;

use axum::extract::ws::{Message as AxumWebSocketMessage, WebSocket, WebSocketUpgrade};
use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, IF_NONE_MATCH, RANGE, SET_COOKIE};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;

pub(super) async fn proxy_htree_daemon_request(
    state: &GatewayState,
    method: &Method,
    headers: &HeaderMap,
    request: HtreeProxyRequest,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    let Some(htree_daemon_addr) = state.htree_daemon_addr.as_deref() else {
        return Err((
            StatusCode::BAD_GATEWAY,
            "hashtree daemon upstream is not configured".into(),
        ));
    };
    if let Some(response) =
        proxy_public_mutable_tree_request(state, method, headers, &request, htree_daemon_addr)
            .await?
    {
        return Ok(response);
    }
    let target = htree_daemon_target(&request);
    let url = format!("http://{htree_daemon_addr}{target}");
    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let reqwest_method = reqwest::Method::from_bytes(method.as_str().as_bytes())
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut upstream = client.request(reqwest_method, url);
    for header in [RANGE, IF_NONE_MATCH, ACCEPT, CONTENT_TYPE, AUTHORIZATION] {
        if let Some(value) = headers.get(&header) {
            upstream = upstream.header(header.as_str(), value.as_bytes());
        }
    }
    if method != Method::GET && method != Method::HEAD {
        upstream = upstream.body(body.to_vec());
    }

    let upstream = upstream
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("hashtree daemon: {e}")))?;
    let status = StatusCode::from_u16(upstream.status().as_u16())
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    let is_html = upstream
        .headers()
        .get(CONTENT_TYPE)
        .is_some_and(is_html_content_type);
    if status.is_success() && is_html && !request_allows_html(&request) {
        return Err((
            StatusCode::FORBIDDEN,
            "HTML htree apps require an isolated iris.localhost origin".into(),
        ));
    }
    let mut builder = response_builder(status, method == Method::HEAD);
    for (name, value) in upstream.headers() {
        if is_proxy_response_header(name.as_str()) {
            builder = builder.header(name.as_str(), value.as_bytes());
        }
    }
    if let Some(key) = request_key_query(&request)
        && let Some(cookie) = key_cookie_value(key)
    {
        builder = builder.header(SET_COOKIE, cookie);
    }
    let bytes = upstream
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("hashtree daemon: {e}")))?;
    try_finish_response(
        builder,
        if method == Method::HEAD {
            Body::empty()
        } else {
            Body::from(bytes)
        },
    )
}

#[derive(Deserialize)]
struct ResolveResponse {
    hash: Option<String>,
    error: Option<String>,
}

async fn proxy_public_mutable_tree_request(
    state: &GatewayState,
    method: &Method,
    headers: &HeaderMap,
    request: &HtreeProxyRequest,
    htree_daemon_addr: &str,
) -> Result<Option<Response>, (StatusCode, String)> {
    if !matches!(*method, Method::GET | Method::HEAD) {
        return Ok(None);
    }
    let HtreeProxyRequest::Tree {
        root: HtreeProxyRoot::Mutable { npub, tree_name },
        path_segments,
        key_query: None,
        allow_html,
    } = request
    else {
        return Ok(None);
    };

    let client = reqwest::Client::builder()
        .no_proxy()
        .build()
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let resolve_url = format!(
        "http://{htree_daemon_addr}/api/resolve/{}/{}",
        percent_encode_path_segment(npub),
        percent_encode_path_segment(tree_name)
    );
    let resolved = client.get(resolve_url).send().await.map_err(|e| {
        (
            StatusCode::BAD_GATEWAY,
            format!("hashtree daemon resolve: {e}"),
        )
    })?;
    if resolved.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resolved.status().is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("hashtree daemon resolve status {}", resolved.status()),
        ));
    }
    let Ok(resolved) = resolved.json::<ResolveResponse>().await else {
        return Ok(None);
    };
    if let Some(error) = resolved.error {
        return Err((StatusCode::BAD_GATEWAY, error));
    }
    let hash = resolved
        .hash
        .ok_or_else(|| {
            (
                StatusCode::BAD_GATEWAY,
                "hashtree resolve missing hash".to_string(),
            )
        })
        .and_then(|hash| {
            from_hex(&hash).map_err(|_| {
                (
                    StatusCode::BAD_GATEWAY,
                    "hashtree resolve returned invalid hash".to_string(),
                )
            })
        })?;

    let servers = gateway_blossom_read_servers(state);
    if servers.is_empty() {
        return Ok(None);
    }
    let response = serve_public_blossom_tree_path(
        &client,
        &servers,
        method,
        headers,
        Cid { hash, key: None },
        path_segments,
        *allow_html,
    )
    .await?;
    Ok(Some(response))
}

async fn serve_public_blossom_tree_path(
    client: &reqwest::Client,
    servers: &[String],
    method: &Method,
    headers: &HeaderMap,
    root: Cid,
    path_segments: &[String],
    allow_html: bool,
) -> Result<Response, (StatusCode, String)> {
    let store = MemoryStore::new();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(store.clone())).public());
    let path = if path_segments.is_empty() {
        "index.html".to_string()
    } else {
        path_segments.join("/")
    };
    let cid = resolve_public_blossom_path(client, servers, store.clone(), &tree, &root, &path)
        .await?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "not found".to_string()))?;
    let bytes = read_public_blossom_file(client, servers, store, &tree, &cid).await?;
    let mime_type = mime_guess::from_path(&path)
        .first_or_octet_stream()
        .essence_str()
        .to_string();
    if mime_type.eq_ignore_ascii_case("text/html") && !allow_html {
        return Err((
            StatusCode::FORBIDDEN,
            "HTML htree apps require an isolated iris.localhost origin".into(),
        ));
    }
    let etag = etag_for(&cid);
    if headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|part| part.trim() == etag))
    {
        return try_finish_response(
            response_builder(StatusCode::NOT_MODIFIED, method == Method::HEAD).header(ETAG, etag),
            Body::empty(),
        );
    }
    try_finish_response(
        response_builder(StatusCode::OK, method == Method::HEAD)
            .header(CONTENT_TYPE, mime_type)
            .header(CONTENT_LENGTH, bytes.len().to_string())
            .header(CACHE_CONTROL, cache_control(CachePolicy::Mutable))
            .header(ETAG, etag)
            .header(X_CONTENT_TYPE_OPTIONS, "nosniff"),
        if method == Method::HEAD {
            Body::empty()
        } else {
            Body::from(bytes)
        },
    )
}

async fn resolve_public_blossom_path(
    client: &reqwest::Client,
    servers: &[String],
    store: MemoryStore,
    tree: &HashTree<MemoryStore>,
    root: &Cid,
    path: &str,
) -> Result<Option<Cid>, (StatusCode, String)> {
    let mut current = root.clone();
    let parts = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    for part in parts {
        let entries =
            list_public_blossom_directory(client, servers, store.clone(), tree, &current).await?;
        let Some(entry) = entries.into_iter().find(|entry| entry.name == part) else {
            return Ok(None);
        };
        current = Cid {
            hash: entry.hash,
            key: entry.key,
        };
    }
    Ok(Some(current))
}

async fn list_public_blossom_directory(
    client: &reqwest::Client,
    servers: &[String],
    store: MemoryStore,
    tree: &HashTree<MemoryStore>,
    cid: &Cid,
) -> Result<Vec<TreeEntry>, (StatusCode, String)> {
    let mut seen_missing = std::collections::HashSet::new();
    loop {
        fetch_public_blossom_blob(client, servers, &store, &cid.hash).await?;
        match tree.list_directory(cid).await {
            Ok(entries) => return Ok(entries),
            Err(HashTreeError::MissingChunk(hash)) => {
                if !seen_missing.insert(hash.clone()) {
                    return Err((StatusCode::BAD_GATEWAY, "cycle in missing chunks".into()));
                }
                let hash = from_hex(&hash).map_err(|_| {
                    (
                        StatusCode::BAD_GATEWAY,
                        "invalid missing chunk hash".to_string(),
                    )
                })?;
                fetch_public_blossom_blob(client, servers, &store, &hash).await?;
            }
            Err(error) => return Err((StatusCode::BAD_GATEWAY, error.to_string())),
        }
    }
}

async fn read_public_blossom_file(
    client: &reqwest::Client,
    servers: &[String],
    store: MemoryStore,
    tree: &HashTree<MemoryStore>,
    cid: &Cid,
) -> Result<Vec<u8>, (StatusCode, String)> {
    let mut seen_missing = std::collections::HashSet::new();
    loop {
        fetch_public_blossom_blob(client, servers, &store, &cid.hash).await?;
        match tree.read_file_range_cid(cid, 0, None).await {
            Ok(Some(bytes)) => return Ok(bytes),
            Ok(None) => return Err((StatusCode::NOT_FOUND, "file not found".into())),
            Err(HashTreeError::MissingChunk(hash)) => {
                if !seen_missing.insert(hash.clone()) {
                    return Err((StatusCode::BAD_GATEWAY, "cycle in missing chunks".into()));
                }
                let hash = from_hex(&hash).map_err(|_| {
                    (
                        StatusCode::BAD_GATEWAY,
                        "invalid missing chunk hash".to_string(),
                    )
                })?;
                fetch_public_blossom_blob(client, servers, &store, &hash).await?;
            }
            Err(error) => return Err((StatusCode::BAD_GATEWAY, error.to_string())),
        }
    }
}

async fn fetch_public_blossom_blob(
    client: &reqwest::Client,
    servers: &[String],
    store: &MemoryStore,
    hash: &Hash,
) -> Result<(), (StatusCode, String)> {
    if store
        .has(hash)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    {
        return Ok(());
    }
    let hash_hex = to_hex(hash);
    let mut last_error = String::new();
    for server in servers {
        let url = format!("{}/{}.bin", server.trim_end_matches('/'), hash_hex);
        match client.get(&url).send().await {
            Ok(response) if response.status().is_success() => {
                let bytes = response
                    .bytes()
                    .await
                    .map_err(|e| (StatusCode::BAD_GATEWAY, format!("blossom read: {e}")))?
                    .to_vec();
                if sha256(&bytes) != *hash {
                    return Err((StatusCode::BAD_GATEWAY, "blossom hash mismatch".into()));
                }
                store
                    .put(*hash, bytes)
                    .await
                    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
                return Ok(());
            }
            Ok(response) => {
                last_error = format!("{} returned {}", url, response.status());
            }
            Err(error) => {
                last_error = format!("{url}: {error}");
            }
        }
    }
    Err((
        StatusCode::BAD_GATEWAY,
        format!("blob {hash_hex} unavailable from Blossom: {last_error}"),
    ))
}

fn gateway_blossom_read_servers(state: &GatewayState) -> Vec<String> {
    AppConfig::load_or_default(config_path_in(state.config_dir.as_ref()))
        .map(|config| config.blossom_servers)
        .unwrap_or_default()
        .into_iter()
        .map(|server| server.trim().trim_end_matches('/').to_string())
        .filter(|server| !server.is_empty())
        .collect()
}

fn is_proxy_response_header(name: &str) -> bool {
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}

fn request_key_query(request: &HtreeProxyRequest) -> Option<&str> {
    match request {
        HtreeProxyRequest::Tree { key_query, .. } => key_query.as_deref(),
        HtreeProxyRequest::Runtime { .. } => None,
    }
}

fn request_allows_html(request: &HtreeProxyRequest) -> bool {
    match request {
        HtreeProxyRequest::Tree { allow_html, .. } => *allow_html,
        HtreeProxyRequest::Runtime { .. } => true,
    }
}

fn is_html_content_type(value: &HeaderValue) -> bool {
    value.to_str().ok().is_some_and(|value| {
        value
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case("text/html")
    })
}

fn htree_daemon_target(request: &HtreeProxyRequest) -> String {
    let HtreeProxyRequest::Tree {
        root,
        path_segments,
        key_query,
        allow_html: _,
    } = request
    else {
        let HtreeProxyRequest::Runtime { target } = request else {
            unreachable!();
        };
        return target.clone();
    };
    let mut target = match root {
        HtreeProxyRoot::Nhash(nhash) => {
            format!("/htree/{}", percent_encode_path_segment(nhash))
        }
        HtreeProxyRoot::Mutable { npub, tree_name } => format!(
            "/htree/{}/{}",
            percent_encode_path_segment(npub),
            percent_encode_path_segment(tree_name)
        ),
    };
    for segment in path_segments {
        target.push('/');
        target.push_str(&percent_encode_path_segment(segment));
    }
    if let Some(key) = key_query.as_deref() {
        target.push_str("?k=");
        target.push_str(&percent_encode_path_segment(key));
    }
    target
}

pub(super) fn runtime_htree_daemon_request(
    uri: &Uri,
    headers: &HeaderMap,
) -> Option<HtreeProxyRequest> {
    if !htree_runtime_host_allowed(headers) || is_htree_runtime_ws_path(uri.path()) {
        return None;
    }
    if !is_htree_runtime_http_path(uri.path()) {
        return None;
    }
    Some(HtreeProxyRequest::Runtime {
        target: uri
            .path_and_query()
            .map_or_else(|| uri.path().to_owned(), |value| value.as_str().to_owned()),
    })
}

pub(super) fn htree_runtime_host_allowed(headers: &HeaderMap) -> bool {
    share_action_host_allowed(headers)
}

pub(super) fn is_htree_runtime_ws_path(path: &str) -> bool {
    path == "/ws" || path == "/ws/"
}

fn is_htree_runtime_http_path(path: &str) -> bool {
    path == "/htree/test"
        || path.starts_with("/htree/")
        || path.starts_with("/__iris/store/")
        || path == "/api/stats"
        || path == "/api/status"
        || path == "/api/cache-tree-root"
        || path == "/api/clear-tree-root-cache"
        || path.starts_with("/api/resolve/")
        || path.starts_with("/api/nostr/")
        || path.starts_with("/api/trees/")
        || path == "/upload"
        || path == "/upload/batch"
        || path == "/upload/check"
        || path.starts_with("/list/")
        || is_top_level_hash_path(path)
}

fn is_top_level_hash_path(path: &str) -> bool {
    let Some(value) = path.strip_prefix('/') else {
        return false;
    };
    if value.is_empty() || value.contains('/') {
        return false;
    }
    let hash = value.split('.').next().unwrap_or(value);
    hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())
}

pub(super) fn proxy_htree_daemon_websocket(
    state: &GatewayState,
    ws: WebSocketUpgrade,
    uri: &Uri,
) -> Result<Response, (StatusCode, String)> {
    let Some(htree_daemon_addr) = state.htree_daemon_addr.as_deref() else {
        return Err((
            StatusCode::BAD_GATEWAY,
            "hashtree daemon upstream is not configured".into(),
        ));
    };
    let target = uri
        .path_and_query()
        .map_or_else(|| uri.path().to_owned(), |value| value.as_str().to_owned());
    let upstream_url = format!("ws://{htree_daemon_addr}{target}");
    Ok(ws.on_upgrade(move |socket| async move {
        if let Err(error) = bridge_htree_daemon_websocket(socket, upstream_url).await {
            tracing::warn!("hashtree daemon websocket proxy ended: {error}");
        }
    }))
}

async fn bridge_htree_daemon_websocket(
    client: WebSocket,
    upstream_url: String,
) -> Result<(), String> {
    let (upstream, _) = tokio_tungstenite::connect_async(upstream_url)
        .await
        .map_err(|e| e.to_string())?;
    let (mut client_tx, mut client_rx) = client.split();
    let (mut upstream_tx, mut upstream_rx) = upstream.split();

    let client_to_upstream = async {
        while let Some(message) = client_rx.next().await {
            let message = message.map_err(|e| e.to_string())?;
            match message {
                AxumWebSocketMessage::Text(text) => upstream_tx
                    .send(TungsteniteMessage::Text(text))
                    .await
                    .map_err(|e| e.to_string())?,
                AxumWebSocketMessage::Binary(bytes) => upstream_tx
                    .send(TungsteniteMessage::Binary(bytes))
                    .await
                    .map_err(|e| e.to_string())?,
                AxumWebSocketMessage::Ping(bytes) => upstream_tx
                    .send(TungsteniteMessage::Ping(bytes))
                    .await
                    .map_err(|e| e.to_string())?,
                AxumWebSocketMessage::Pong(bytes) => upstream_tx
                    .send(TungsteniteMessage::Pong(bytes))
                    .await
                    .map_err(|e| e.to_string())?,
                AxumWebSocketMessage::Close(_) => break,
            }
        }
        let _ = upstream_tx.close().await;
        Ok::<(), String>(())
    };

    let upstream_to_client = async {
        while let Some(message) = upstream_rx.next().await {
            let message = message.map_err(|e| e.to_string())?;
            match message {
                TungsteniteMessage::Text(text) => client_tx
                    .send(AxumWebSocketMessage::Text(text.clone()))
                    .await
                    .map_err(|e| e.to_string())?,
                TungsteniteMessage::Binary(bytes) => client_tx
                    .send(AxumWebSocketMessage::Binary(bytes))
                    .await
                    .map_err(|e| e.to_string())?,
                TungsteniteMessage::Ping(bytes) => client_tx
                    .send(AxumWebSocketMessage::Ping(bytes))
                    .await
                    .map_err(|e| e.to_string())?,
                TungsteniteMessage::Pong(bytes) => client_tx
                    .send(AxumWebSocketMessage::Pong(bytes))
                    .await
                    .map_err(|e| e.to_string())?,
                TungsteniteMessage::Close(_) => break,
                TungsteniteMessage::Frame(_) => {}
            }
        }
        let _ = client_tx.close().await;
        Ok::<(), String>(())
    };

    tokio::select! {
        result = client_to_upstream => result,
        result = upstream_to_client => result,
    }
}
