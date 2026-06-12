#[allow(clippy::wildcard_imports)]
use super::*;

use axum::extract::ws::{Message as AxumWebSocketMessage, WebSocket, WebSocketUpgrade};
use axum::http::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, IF_NONE_MATCH, RANGE, SET_COOKIE};
use futures::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as TungsteniteMessage;

pub(super) async fn proxy_htree_daemon_request(
    state: &GatewayState,
    method: &Method,
    headers: &HeaderMap,
    request: HtreeProxyRequest,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    let target = htree_daemon_target(&request);
    let Some(htree_daemon_addr) = state.htree_daemon_addr.as_deref() else {
        return Err((
            StatusCode::BAD_GATEWAY,
            "hashtree daemon upstream is not configured".into(),
        ));
    };
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
                    .send(TungsteniteMessage::Text(text.into()))
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
                    .send(AxumWebSocketMessage::Text(text.to_string()))
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
