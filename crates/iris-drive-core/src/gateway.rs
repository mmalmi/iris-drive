//! Loopback HTTP gateway for serving hashtree-backed Iris Drive content.
//!
//! Browser-facing origins use `*.localhost` names so stock browsers can
//! treat them as secure contexts without a custom CA or browser fork.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, Bytes};
use axum::extract::State;
use axum::extract::ws::WebSocketUpgrade;
use axum::http::header::{
    ACCEPT_RANGES, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE,
    COOKIE, ETAG, HOST, IF_NONE_MATCH, LOCATION, ORIGIN, RANGE, SET_COOKIE, VARY,
    X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::Response;
use axum::routing::any;
use hashtree_core::{
    Cid, Hash, HashTree, LinkType, NHashData, Store, TreeEntry, from_hex, nhash_decode,
    nhash_encode_full, to_hex,
};
use hashtree_fs::FsBlobStore;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::config::{AppConfig, ConfigError};
use crate::daemon::DaemonError;
use crate::paths::{config_path_in, key_path_in};
use crate::{Daemon, PRIMARY_DRIVE_ID};

mod paths;
mod proxy;
mod response;

pub use self::paths::encode_immutable_host_label;
#[allow(clippy::wildcard_imports)]
use self::paths::*;
#[allow(clippy::wildcard_imports)]
use self::proxy::*;
#[allow(clippy::wildcard_imports)]
use self::response::*;

pub const LOCAL_PORTAL_HOST: &str = "iris.localhost";
pub const LOCAL_NHASH_RESOLVER_HOST: &str = "nhash.iris.localhost";
pub const DEFAULT_GATEWAY_PORT: u16 = 17_321;
pub const IRIS_SITES_PORTAL_NPUB: &str =
    "npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm";
pub const IRIS_SITES_PORTAL_TREE: &str = "sites";
pub const IRIS_SITES_PORTAL_BOOTSTRAP_NHASH: &str = "nhash1qqsgjkehkfml3ak2xld3svmy7862ndqv7ay76jeuq9h28ymyex8q3xg9yz5qqcrc4fq3kfwt3maz4nss7d4qeshcmlkgelvhckszvk76lrzxxfu88jz";
const IMMUTABLE_HOST_SUFFIX: &str = ".sites.iris.localhost";
const HASH_HOST_SUFFIX: &str = ".hash.localhost";
const DRIVE_HOST_SUFFIX: &str = ".drive.iris.localhost";
const IRIS_LOCALHOST_SUFFIX: &str = ".iris.localhost";
const IRIS_LOCAL_SUFFIX: &str = ".iris.local";
const KEY_COOKIE: &str = "iris_htree_key";
const SHARE_ACTION_API_PATH: &str = "/api/iris-drive/share-action";

#[derive(Debug, Error)]
pub enum GatewayError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("config: {0}")]
    Config(#[from] ConfigError),
    #[error("daemon: {0}")]
    Daemon(#[from] DaemonError),
    #[error("hashtree: {0}")]
    Hashtree(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("gateway task failed: {0}")]
    Task(String),
}

/// Bind target for the local HTTP gateway.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GatewayBind {
    pub addr: SocketAddr,
}

impl GatewayBind {
    #[must_use]
    pub fn loopback_v4(port: u16) -> Self {
        Self {
            addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
        }
    }
}

impl Default for GatewayBind {
    fn default() -> Self {
        Self::loopback_v4(DEFAULT_GATEWAY_PORT)
    }
}

/// Running loopback gateway. Drop it to request shutdown.
pub struct GatewayServer {
    local_addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), GatewayError>>>,
}

/// Loopback proxy for WebViews that cannot resolve `*.localhost` names.
///
/// The browser keeps the original Iris-local URL and origin, while the proxy
/// forwards only those hosts to the already-running gateway on loopback.
pub struct GatewayProxyServer {
    local_addr: SocketAddr,
    shutdown_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<(), GatewayError>>>,
}

impl GatewayServer {
    pub async fn bind(
        config_dir: impl Into<PathBuf>,
        bind: GatewayBind,
    ) -> Result<Self, GatewayError> {
        let config_dir = config_dir.into();
        let daemon = Daemon::open(config_dir.clone())?;
        Self::bind_with_tree(config_dir, daemon.tree_handle(), bind).await
    }

    pub async fn bind_with_tree(
        config_dir: impl Into<PathBuf>,
        tree: Arc<HashTree<FsBlobStore>>,
        bind: GatewayBind,
    ) -> Result<Self, GatewayError> {
        Self::bind_inner(config_dir, tree, None, bind).await
    }

    pub async fn bind_with_tree_and_htree_daemon(
        config_dir: impl Into<PathBuf>,
        tree: Arc<HashTree<FsBlobStore>>,
        htree_daemon_addr: impl Into<String>,
        bind: GatewayBind,
    ) -> Result<Self, GatewayError> {
        Self::bind_inner(
            config_dir,
            tree,
            Some(normalize_daemon_addr(&htree_daemon_addr.into())),
            bind,
        )
        .await
    }

    async fn bind_inner(
        config_dir: impl Into<PathBuf>,
        tree: Arc<HashTree<FsBlobStore>>,
        htree_daemon_addr: Option<String>,
        bind: GatewayBind,
    ) -> Result<Self, GatewayError> {
        let listener = TcpListener::bind(bind.addr).await?;
        let local_addr = listener.local_addr()?;
        let state = GatewayState {
            config_dir: Arc::new(config_dir.into()),
            tree,
            htree_daemon_addr: htree_daemon_addr.map(Arc::new),
        };
        let app = Router::new()
            .fallback(any(gateway_handler))
            .with_state(state);
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app)
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .map_err(GatewayError::Io)
        });

        Ok(Self {
            local_addr,
            shutdown_tx: Some(shutdown_tx),
            handle: Some(handle),
        })
    }

    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn shutdown(mut self) -> Result<(), GatewayError> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle
                .await
                .map_err(|e| GatewayError::Task(e.to_string()))?
        } else {
            Ok(())
        }
    }
}

impl Drop for GatewayServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

impl GatewayProxyServer {
    pub async fn bind_for_gateway(gateway_addr: SocketAddr) -> Result<Self, GatewayError> {
        let listener =
            TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0)).await?;
        let local_addr = listener.local_addr()?;
        let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();
        let handle =
            tokio::spawn(
                async move { run_gateway_proxy(listener, gateway_addr, shutdown_rx).await },
            );
        Ok(Self {
            local_addr,
            shutdown_tx: Some(shutdown_tx),
            handle: Some(handle),
        })
    }

    #[must_use]
    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    pub async fn shutdown(mut self) -> Result<(), GatewayError> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle
                .await
                .map_err(|e| GatewayError::Task(e.to_string()))?
        } else {
            Ok(())
        }
    }
}

impl Drop for GatewayProxyServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
    }
}

async fn run_gateway_proxy(
    listener: TcpListener,
    gateway_addr: SocketAddr,
    mut shutdown_rx: oneshot::Receiver<()>,
) -> Result<(), GatewayError> {
    loop {
        tokio::select! {
            accept = listener.accept() => {
                let (stream, _) = accept?;
                tokio::spawn(async move {
                    let _ = handle_gateway_proxy_connection(stream, gateway_addr).await;
                });
            }
            _ = &mut shutdown_rx => return Ok(()),
        }
    }
}

async fn handle_gateway_proxy_connection(
    mut inbound: TcpStream,
    gateway_addr: SocketAddr,
) -> std::io::Result<()> {
    let mut buffer = Vec::with_capacity(4096);
    let header_end = loop {
        let mut chunk = [0u8; 1024];
        let read = inbound.read(&mut chunk).await?;
        if read == 0 {
            return Ok(());
        }
        buffer.extend_from_slice(&chunk[..read]);
        if buffer.len() > 64 * 1024 {
            return write_proxy_error(&mut inbound, "431 Request Header Fields Too Large").await;
        }
        if let Some(index) = find_header_end(&buffer) {
            break index;
        }
    };

    let body_start = header_end + 4;
    let head = &buffer[..body_start];
    let remainder = &buffer[body_start..];
    let Ok(head_text) = std::str::from_utf8(head) else {
        return write_proxy_error(&mut inbound, "400 Bad Request").await;
    };
    let Some(line_end) = head_text.find("\r\n") else {
        return write_proxy_error(&mut inbound, "400 Bad Request").await;
    };
    let request_line = &head_text[..line_end];
    let header_tail = &head_text[line_end + 2..];
    let mut parts = request_line.split_whitespace();
    let Some(method) = parts.next() else {
        return write_proxy_error(&mut inbound, "400 Bad Request").await;
    };
    let Some(target) = parts.next() else {
        return write_proxy_error(&mut inbound, "400 Bad Request").await;
    };
    let Some(version) = parts.next() else {
        return write_proxy_error(&mut inbound, "400 Bad Request").await;
    };

    if method.eq_ignore_ascii_case("CONNECT") {
        let Some((host, port)) = proxy_authority_host_port(target) else {
            return write_proxy_error(&mut inbound, "400 Bad Request").await;
        };
        if !is_gateway_proxy_target(&host, port, gateway_addr.port()) {
            return write_proxy_error(&mut inbound, "403 Forbidden").await;
        }
        let mut upstream = TcpStream::connect(gateway_addr).await?;
        inbound
            .write_all(b"HTTP/1.1 200 Connection Established\r\n\r\n")
            .await?;
        if !remainder.is_empty() {
            upstream.write_all(remainder).await?;
        }
        let _ = tokio::io::copy_bidirectional(&mut inbound, &mut upstream).await;
        return Ok(());
    }

    let Some((host, port, path_and_query)) = proxy_request_target(target, header_tail) else {
        return write_proxy_error(&mut inbound, "400 Bad Request").await;
    };
    if !is_gateway_proxy_target(&host, port, gateway_addr.port()) {
        return write_proxy_error(&mut inbound, "403 Forbidden").await;
    }

    let mut upstream = TcpStream::connect(gateway_addr).await?;
    let rewritten = format!("{method} {path_and_query} {version}\r\n{header_tail}");
    upstream.write_all(rewritten.as_bytes()).await?;
    if !remainder.is_empty() {
        upstream.write_all(remainder).await?;
    }
    let _ = tokio::io::copy_bidirectional(&mut inbound, &mut upstream).await;
    Ok(())
}

fn find_header_end(buffer: &[u8]) -> Option<usize> {
    buffer.windows(4).position(|window| window == b"\r\n\r\n")
}

async fn write_proxy_error(stream: &mut TcpStream, status: &str) -> std::io::Result<()> {
    let response = format!("HTTP/1.1 {status}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    stream.write_all(response.as_bytes()).await
}

fn proxy_request_target(target: &str, headers: &str) -> Option<(String, u16, String)> {
    if target.starts_with("http://") {
        let uri = target.parse::<Uri>().ok()?;
        let authority = uri.authority()?.as_str();
        let (host, port) = proxy_authority_host_port(authority)?;
        let path_and_query = uri
            .path_and_query()
            .map_or("/", |value| value.as_str())
            .to_string();
        return Some((host, port, path_and_query));
    }

    let host_header = proxy_header(headers, "host")?;
    let (host, port) = proxy_authority_host_port(&host_header)?;
    Some((host, port, target.to_string()))
}

fn proxy_header(headers: &str, name: &str) -> Option<String> {
    headers.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        key.eq_ignore_ascii_case(name)
            .then(|| value.trim().to_string())
    })
}

fn proxy_authority_host_port(authority: &str) -> Option<(String, u16)> {
    let trimmed = authority.trim().trim_end_matches('.');
    let port = host_port(trimmed)?;
    Some((normalize_host(trimmed), port))
}

fn is_gateway_proxy_target(host: &str, port: u16, gateway_port: u16) -> bool {
    if port != gateway_port {
        return false;
    }
    host == LOCAL_PORTAL_HOST
        || host == LOCAL_NHASH_RESOLVER_HOST
        || host.ends_with(IRIS_LOCALHOST_SUFFIX)
        || host.ends_with(HASH_HOST_SUFFIX)
}

#[derive(Clone)]
struct GatewayState {
    config_dir: Arc<PathBuf>,
    tree: Arc<HashTree<FsBlobStore>>,
    htree_daemon_addr: Option<Arc<String>>,
}

fn normalize_daemon_addr(value: &str) -> String {
    let trimmed = value
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://")
        .trim_end_matches('/');
    match trimmed.parse::<SocketAddr>() {
        Ok(addr) if addr.ip().is_unspecified() => {
            let ip = if addr.ip().is_ipv4() {
                IpAddr::V4(Ipv4Addr::LOCALHOST)
            } else {
                IpAddr::V6(std::net::Ipv6Addr::LOCALHOST)
            };
            socket_addr_authority(SocketAddr::new(ip, addr.port()))
        }
        Ok(addr) => socket_addr_authority(addr),
        Err(_) => trimmed.to_string(),
    }
}

fn socket_addr_authority(addr: SocketAddr) -> String {
    match addr.ip() {
        IpAddr::V4(ip) => format!("{ip}:{}", addr.port()),
        IpAddr::V6(ip) => format!("[{ip}]:{}", addr.port()),
    }
}

#[derive(Debug, Clone)]
enum GatewayRequest {
    Local(LocalGatewayRequest),
    Drive(DriveGatewayRequest),
    HtreeDaemon(HtreeProxyRequest),
    Redirect(String),
}

#[derive(Debug, Clone)]
struct LocalGatewayRequest {
    root: Cid,
    path_segments: Vec<String>,
    cache_policy: CachePolicy,
    set_key_cookie: Option<String>,
}

#[derive(Debug, Clone)]
struct DriveGatewayRequest {
    drive_id: String,
    path_segments: Vec<String>,
}

#[derive(Debug, Clone)]
enum HtreeProxyRequest {
    Tree {
        root: HtreeProxyRoot,
        path_segments: Vec<String>,
        key_query: Option<String>,
        allow_html: bool,
    },
    Runtime {
        target: String,
    },
}

#[derive(Debug, Clone)]
enum HtreeProxyRoot {
    Nhash(String),
    Mutable { npub: String, tree_name: String },
}

#[derive(Debug, Clone, Copy)]
enum CachePolicy {
    Immutable,
    Mutable,
}

#[derive(Debug)]
enum ResolvedContent {
    Directory {
        cid: Cid,
        display_path: String,
    },
    File {
        cid: Cid,
        size: u64,
        path: String,
        mime_type: String,
    },
}

async fn gateway_handler(
    State(state): State<GatewayState>,
    ws: Option<WebSocketUpgrade>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match handle_gateway_request(state, ws, method, uri, headers, body).await {
        Ok(response) => response,
        Err((status, message)) => text_response(status, &message),
    }
}

async fn handle_gateway_request(
    state: GatewayState,
    ws: Option<WebSocketUpgrade>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    if uri.path() == SHARE_ACTION_API_PATH {
        return handle_share_action_api(&state, &method, &headers, body.as_ref());
    }

    if is_htree_runtime_ws_path(uri.path()) && htree_runtime_host_allowed(&headers) {
        let Some(ws) = ws else {
            return Err((StatusCode::BAD_REQUEST, "websocket upgrade required".into()));
        };
        return proxy_htree_daemon_websocket(&state, ws, &uri);
    }

    if let Some(request) = runtime_htree_daemon_request(&uri, &headers) {
        return proxy_htree_daemon_request(&state, &method, &headers, request, body).await;
    }

    let request = resolve_gateway_request(&uri, &headers)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let request = match request {
        GatewayRequest::Local(request) => request,
        GatewayRequest::Drive(request) => materialize_drive_gateway_request(&state, request)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?,
        GatewayRequest::HtreeDaemon(request) => {
            return proxy_htree_daemon_request(&state, &method, &headers, request, body).await;
        }
        GatewayRequest::Redirect(location) => return Ok(redirect_response(&location)),
    };

    if method != Method::GET && method != Method::HEAD {
        return Err((StatusCode::METHOD_NOT_ALLOWED, "method not allowed".into()));
    }

    let content = resolve_content(&state.tree, &request.root, &request.path_segments)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "not found".into()))?;

    match content {
        ResolvedContent::Directory { cid, display_path } => {
            let entries = list_public_directory(&state.tree, &cid)
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok(directory_response(
                &entries,
                &display_path,
                method == Method::HEAD,
                request.cache_policy,
                request.set_key_cookie.as_deref(),
            ))
        }
        ResolvedContent::File {
            cid,
            size,
            path,
            mime_type,
        } => {
            let options = FileResponseOptions {
                size,
                path: &path,
                mime_type: &mime_type,
                head: method == Method::HEAD,
                cache_policy: request.cache_policy,
                set_key_cookie: request.set_key_cookie.as_deref(),
                headers: &headers,
            };
            serve_file(&state.tree, &cid, options).await
        }
    }
}

fn handle_share_action_api(
    state: &GatewayState,
    method: &Method,
    headers: &HeaderMap,
    body: &[u8],
) -> Result<Response, (StatusCode, String)> {
    let cors_origin = share_action_cors_origin(headers)?;
    if !share_action_host_allowed(headers) {
        return Err((StatusCode::BAD_REQUEST, "invalid share action host".into()));
    }
    if method == Method::OPTIONS {
        return try_finish_response(
            share_action_response_builder(StatusCode::NO_CONTENT, cors_origin.as_ref()),
            Body::empty(),
        );
    }
    if method == Method::GET {
        let result = crate::share_action_state(&state.config_dir)
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e:#}")))?;
        return share_action_json_response(&result, cors_origin.as_ref());
    }
    if method != Method::POST {
        return Err((StatusCode::METHOD_NOT_ALLOWED, "method not allowed".into()));
    }
    let action = serde_json::from_slice::<crate::ShareAction>(body)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("share action json: {e}")))?;
    let result = crate::dispatch_share_action(&state.config_dir, action, gateway_now_seconds())
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("{e:#}")))?;
    share_action_json_response(&result, cors_origin.as_ref())
}

fn share_action_json_response(
    result: &crate::ShareActionResult,
    cors_origin: Option<&HeaderValue>,
) -> Result<Response, (StatusCode, String)> {
    let body = serde_json::to_vec(result)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    try_finish_response(
        share_action_response_builder(StatusCode::OK, cors_origin)
            .header(CONTENT_TYPE, "application/json")
            .header(CACHE_CONTROL, "no-store"),
        Body::from(body),
    )
}

fn share_action_response_builder(
    status: StatusCode,
    cors_origin: Option<&HeaderValue>,
) -> http::response::Builder {
    let mut builder = response_builder(status, false)
        .header(ACCESS_CONTROL_ALLOW_METHODS, "GET, POST, OPTIONS")
        .header(ACCESS_CONTROL_ALLOW_HEADERS, "content-type")
        .header(VARY, "origin");
    if let Some(origin) = cors_origin {
        builder = builder.header(ACCESS_CONTROL_ALLOW_ORIGIN, origin);
    }
    builder
}

fn share_action_host_allowed(headers: &HeaderMap) -> bool {
    let Some(host) = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .map(normalize_host)
    else {
        return false;
    };
    is_loopback_host(&host)
        || host == LOCAL_PORTAL_HOST
        || host.ends_with(DRIVE_HOST_SUFFIX)
        || host.ends_with(IRIS_LOCALHOST_SUFFIX)
        || host.ends_with(IRIS_LOCAL_SUFFIX)
}

fn share_action_cors_origin(
    headers: &HeaderMap,
) -> Result<Option<HeaderValue>, (StatusCode, String)> {
    let Some(origin) = headers.get(ORIGIN) else {
        return Ok(None);
    };
    let origin_str = origin
        .to_str()
        .map_err(|_| (StatusCode::FORBIDDEN, "invalid origin".to_string()))?;
    let Some(host) = origin_host(origin_str) else {
        return Err((StatusCode::FORBIDDEN, "invalid origin".into()));
    };
    if share_action_origin_host_allowed(&host) {
        return Ok(Some(origin.clone()));
    }
    Err((StatusCode::FORBIDDEN, "origin is not allowed".into()))
}

fn share_action_origin_host_allowed(host: &str) -> bool {
    let host = normalize_host(host);
    is_loopback_host(&host)
        || host == "drive.iris.to"
        || host == LOCAL_PORTAL_HOST
        || host.ends_with(DRIVE_HOST_SUFFIX)
        || host.ends_with(IRIS_LOCALHOST_SUFFIX)
        || host.ends_with(IRIS_LOCAL_SUFFIX)
        || host.ends_with(".htree.localhost")
}

fn origin_host(origin: &str) -> Option<String> {
    let rest = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))?;
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.starts_with('[') {
        return authority.split(']').next().map(|value| format!("{value}]"));
    }
    authority
        .split(':')
        .next()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn gateway_now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

fn resolve_gateway_request(uri: &Uri, headers: &HeaderMap) -> Result<GatewayRequest, GatewayError> {
    let host_header = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| GatewayError::InvalidRequest("missing Host header".into()))?;
    let host = normalize_host(host_header);
    if host.is_empty() {
        return Err(GatewayError::InvalidRequest("empty Host header".into()));
    }

    let (path_segments, route_from_path) = parse_gateway_path(uri.path())?;
    if let Some(route) = route_from_path {
        return request_from_path_route(uri, headers, route, path_segments);
    }

    if host == LOCAL_NHASH_RESOLVER_HOST {
        return nhash_resolver_host_request(uri, headers, path_segments);
    }

    if host == LOCAL_PORTAL_HOST {
        if path_segments
            .first()
            .is_some_and(|segment| segment.starts_with("npub1"))
        {
            return portal_npub_path_request(path_segments);
        }
        return Ok(portal_host_redirect(uri, host_header));
    }

    if let Some((npub, tree_name)) = mutable_site_host(&host) {
        return Ok(mutable_htree_request(npub, tree_name, path_segments, true));
    }

    if let Some(label) = host.strip_suffix(IMMUTABLE_HOST_SUFFIX) {
        return immutable_host_request(label, uri, headers, path_segments);
    }

    if let Some(label) = host.strip_suffix(HASH_HOST_SUFFIX) {
        return immutable_host_request(label, uri, headers, path_segments);
    }

    if let Some(drive_id) = host.strip_suffix(DRIVE_HOST_SUFFIX) {
        return drive_host_request(drive_id, path_segments);
    }

    if let Some(nhash) = nhash_from_split_host(&host, IRIS_LOCALHOST_SUFFIX)
        .or_else(|| nhash_from_split_host(&host, IRIS_LOCAL_SUFFIX))
    {
        return nhash_request(&nhash, uri, headers, path_segments, true);
    }

    if let Some(drive_id) = host.strip_suffix(IRIS_LOCALHOST_SUFFIX)
        && drive_id == PRIMARY_DRIVE_ID
    {
        return drive_host_request(drive_id, path_segments);
    }

    if is_loopback_host(&host) {
        return Err(GatewayError::InvalidRequest(
            "loopback host requires /drive/<id>/... or /nhash/<value>/...".into(),
        ));
    }

    Err(GatewayError::InvalidRequest(format!(
        "host {host} is not an Iris Drive gateway host"
    )))
}

enum PathRoute {
    Drive(String),
    Nhash(String),
}

fn request_from_path_route(
    uri: &Uri,
    headers: &HeaderMap,
    route: PathRoute,
    path_segments: Vec<String>,
) -> Result<GatewayRequest, GatewayError> {
    match route {
        PathRoute::Drive(drive_id) => drive_host_request(&drive_id, path_segments),
        PathRoute::Nhash(nhash) => nhash_request(&nhash, uri, headers, path_segments, false),
    }
}

fn nhash_resolver_host_request(
    uri: &Uri,
    headers: &HeaderMap,
    mut path_segments: Vec<String>,
) -> Result<GatewayRequest, GatewayError> {
    if path_segments.is_empty() {
        return Err(GatewayError::InvalidRequest(
            "nhash resolver host requires /<nhash>/...".into(),
        ));
    }
    let nhash = path_segments.remove(0);
    nhash_request(&nhash, uri, headers, path_segments, false)
}

fn nhash_request(
    nhash: &str,
    uri: &Uri,
    headers: &HeaderMap,
    path_segments: Vec<String>,
    allow_html: bool,
) -> Result<GatewayRequest, GatewayError> {
    let _ = cid_from_nhash(nhash)?;
    let key_query = query_param(uri.query(), "k")
        .or_else(|| cookie_value(headers, KEY_COOKIE))
        .map(|key| {
            from_hex(&key)
                .map(|parsed| to_hex(&parsed))
                .map_err(|_| GatewayError::InvalidRequest("invalid key".into()))
        })
        .transpose()?;
    Ok(GatewayRequest::HtreeDaemon(HtreeProxyRequest::Tree {
        root: HtreeProxyRoot::Nhash(nhash.to_string()),
        path_segments,
        key_query,
        allow_html,
    }))
}

fn portal_npub_path_request(
    mut path_segments: Vec<String>,
) -> Result<GatewayRequest, GatewayError> {
    if path_segments.len() < 2 {
        return Err(GatewayError::InvalidRequest(
            "iris.localhost npub paths require /<npub>/<tree>/...".into(),
        ));
    }
    let npub = path_segments.remove(0);
    let tree_name = path_segments.remove(0);
    if !npub.starts_with("npub1") || tree_name.is_empty() {
        return Err(GatewayError::InvalidRequest(
            "iris.localhost npub paths require /<npub>/<tree>/...".into(),
        ));
    }
    Ok(mutable_htree_request(npub, tree_name, path_segments, false))
}

fn mutable_htree_request(
    npub: String,
    tree_name: String,
    mut path_segments: Vec<String>,
    allow_html: bool,
) -> GatewayRequest {
    if path_segments.is_empty() {
        path_segments.push("index.html".to_owned());
    }
    GatewayRequest::HtreeDaemon(HtreeProxyRequest::Tree {
        root: HtreeProxyRoot::Mutable { npub, tree_name },
        path_segments,
        key_query: None,
        allow_html,
    })
}

fn portal_host_redirect(uri: &Uri, host_header: &str) -> GatewayRequest {
    let mut location = match host_port(host_header) {
        Some(port) => {
            local_mutable_site_origin(Some(port), IRIS_SITES_PORTAL_NPUB, IRIS_SITES_PORTAL_TREE)
        }
        None => local_mutable_site_origin(None, IRIS_SITES_PORTAL_NPUB, IRIS_SITES_PORTAL_TREE),
    };
    location.push_str(uri.path_and_query().map_or("/", |value| value.as_str()));
    GatewayRequest::Redirect(location)
}

fn host_port(host_header: &str) -> Option<u16> {
    let trimmed = host_header.trim().trim_end_matches('.');
    if trimmed.starts_with('[') {
        return trimmed
            .split_once("]:")
            .and_then(|(_, port)| port.parse().ok());
    }
    trimmed
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse().ok())
}

fn mutable_site_host(host: &str) -> Option<(String, String)> {
    let name = host.strip_suffix(IRIS_LOCALHOST_SUFFIX)?;
    let (tree_name, npub) = name.split_once('.')?;
    if tree_name.contains('.') || !npub.starts_with("npub1") {
        return None;
    }
    if !is_dns_site_label(tree_name) || !is_dns_site_label(npub) {
        return None;
    }
    Some((npub.to_owned(), tree_name.to_owned()))
}

pub(crate) fn is_dns_site_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 63
        && label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-')
}

fn immutable_host_request(
    label: &str,
    uri: &Uri,
    headers: &HeaderMap,
    path_segments: Vec<String>,
) -> Result<GatewayRequest, GatewayError> {
    if label.is_empty() || label.contains('.') {
        return Err(GatewayError::InvalidRequest(
            "immutable content host must use a single base32 hash label".into(),
        ));
    }
    let hash = decode_base32_hash(label)
        .ok_or_else(|| GatewayError::InvalidRequest("invalid immutable hash label".into()))?;
    let (cid, set_key_cookie) = cid_with_request_key(Cid { hash, key: None }, uri, headers)?;
    Ok(GatewayRequest::Local(LocalGatewayRequest {
        root: cid,
        path_segments,
        cache_policy: CachePolicy::Immutable,
        set_key_cookie,
    }))
}

fn drive_host_request(
    drive_id: &str,
    path_segments: Vec<String>,
) -> Result<GatewayRequest, GatewayError> {
    if !is_safe_drive_id(drive_id) {
        return Err(GatewayError::InvalidRequest("invalid drive id".into()));
    }
    Ok(GatewayRequest::Drive(DriveGatewayRequest {
        drive_id: drive_id.to_string(),
        path_segments,
    }))
}

async fn materialize_drive_gateway_request(
    state: &GatewayState,
    request: DriveGatewayRequest,
) -> Result<LocalGatewayRequest, GatewayError> {
    let root = current_drive_root(state, &request.drive_id).await?;
    Ok(LocalGatewayRequest {
        root,
        path_segments: request.path_segments,
        cache_policy: CachePolicy::Mutable,
        set_key_cookie: None,
    })
}

async fn current_drive_root(state: &GatewayState, drive_id: &str) -> Result<Cid, GatewayError> {
    let config_dir = state.config_dir.as_ref();
    if !key_path_in(config_dir).exists() {
        return Err(GatewayError::InvalidRequest(
            "iris-drive is not initialized".into(),
        ));
    }
    let config_path = config_path_in(config_dir);
    let mut config = AppConfig::load_or_default(&config_path)?;
    if crate::repair_missing_share_shortcuts(&mut config)
        .map_err(|e| GatewayError::InvalidRequest(e.to_string()))?
    {
        config.save(&config_path)?;
    }
    if drive_id == PRIMARY_DRIVE_ID {
        let merged = crate::primary_merged_root(state.tree.as_ref(), &config)
            .await
            .map_err(|e| GatewayError::Hashtree(e.to_string()))?;
        return Ok(merged.root_cid);
    }
    let drive = config
        .drive(drive_id)
        .ok_or_else(|| GatewayError::InvalidRequest(format!("drive {drive_id} not found")))?;
    let root_cid = config
        .profile
        .as_ref()
        .and_then(|account| drive.app_key_roots.get(&account.app_key_pubkey))
        .map(|root| root.root_cid.as_str())
        .or(drive.last_root_cid.as_deref())
        .ok_or_else(|| GatewayError::InvalidRequest(format!("drive {drive_id} has no root")))?;
    Cid::parse(root_cid).map_err(|e| GatewayError::InvalidRequest(e.to_string()))
}

async fn resolve_content<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    segments: &[String],
) -> Result<Option<ResolvedContent>, GatewayError> {
    for segment in segments {
        if segment == ".hashtree" {
            return Err(GatewayError::InvalidRequest(
                "internal metadata is not served".into(),
            ));
        }
    }

    if let Some(file) = resolve_root_file(tree, root, segments).await? {
        return Ok(Some(file));
    }

    let mut current = root.clone();
    if segments.is_empty() {
        return resolve_directory_or_index(tree, current, "").await;
    }

    for (index, segment) in segments.iter().enumerate() {
        let Some(entry) = find_entry(tree, &current, segment).await? else {
            return Ok(None);
        };
        let cid = entry_cid(&entry);
        if index + 1 == segments.len() {
            return match entry.link_type {
                LinkType::Dir | LinkType::Fanout => {
                    resolve_directory_or_index(tree, cid, &segments.join("/")).await
                }
                LinkType::Blob | LinkType::File => Ok(Some(ResolvedContent::File {
                    cid,
                    size: entry.size,
                    path: segments.join("/"),
                    mime_type: mime_type_for_path(&segments.join("/"), entry.meta.as_ref()),
                })),
            };
        }
        if !matches!(entry.link_type, LinkType::Dir | LinkType::Fanout) {
            return Ok(None);
        }
        current = cid;
    }

    Ok(None)
}

async fn resolve_root_file<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    segments: &[String],
) -> Result<Option<ResolvedContent>, GatewayError> {
    let is_dir = tree
        .is_dir(root)
        .await
        .map_err(|e| GatewayError::Hashtree(e.to_string()))?;
    if is_dir {
        return Ok(None);
    }

    let Some(size) = file_size_for_cid(tree, root).await? else {
        return Ok(None);
    };
    let path = file_hint_path(segments);
    let mime_type = mime_type_for_path(&path, None);
    Ok(Some(ResolvedContent::File {
        cid: root.clone(),
        size,
        path,
        mime_type,
    }))
}

async fn file_size_for_cid<S: Store>(
    tree: &HashTree<S>,
    cid: &Cid,
) -> Result<Option<u64>, GatewayError> {
    if let Some(node) = tree
        .get_node(cid)
        .await
        .map_err(|e| GatewayError::Hashtree(e.to_string()))?
    {
        return match node.node_type {
            LinkType::File => Ok(Some(node.links.iter().map(|link| link.size).sum())),
            LinkType::Dir | LinkType::Fanout | LinkType::Blob => Ok(None),
        };
    }

    let Some(bytes) = tree
        .read_file_range_cid(cid, 0, None)
        .await
        .map_err(|e| GatewayError::Hashtree(e.to_string()))?
    else {
        return Ok(None);
    };
    Ok(Some(u64::try_from(bytes.len()).unwrap_or(u64::MAX)))
}

fn file_hint_path(segments: &[String]) -> String {
    segments
        .last()
        .cloned()
        .unwrap_or_else(|| "download".to_string())
}

async fn resolve_directory_or_index<S: Store>(
    tree: &HashTree<S>,
    dir: Cid,
    display_path: &str,
) -> Result<Option<ResolvedContent>, GatewayError> {
    if let Some(index) = find_entry(tree, &dir, "index.html").await?
        && (index.link_type == LinkType::Blob || index.link_type == LinkType::File)
    {
        return Ok(Some(ResolvedContent::File {
            cid: entry_cid(&index),
            size: index.size,
            path: append_path(display_path, "index.html"),
            mime_type: mime_type_for_path("index.html", index.meta.as_ref()),
        }));
    }
    Ok(Some(ResolvedContent::Directory {
        cid: dir,
        display_path: display_path.to_string(),
    }))
}

async fn find_entry<S: Store>(
    tree: &HashTree<S>,
    dir: &Cid,
    name: &str,
) -> Result<Option<TreeEntry>, GatewayError> {
    let entries = tree
        .list_directory(dir)
        .await
        .map_err(|e| GatewayError::Hashtree(e.to_string()))?;
    Ok(entries.into_iter().find(|entry| entry.name == name))
}

async fn list_public_directory<S: Store>(
    tree: &HashTree<S>,
    dir: &Cid,
) -> Result<Vec<TreeEntry>, GatewayError> {
    let mut entries = tree
        .list_directory(dir)
        .await
        .map_err(|e| GatewayError::Hashtree(e.to_string()))?;
    entries.retain(|entry| entry.name != ".hashtree");
    entries.sort_by(
        |a, b| match (a.link_type == LinkType::Dir, b.link_type == LinkType::Dir) {
            (true, false) => std::cmp::Ordering::Less,
            (false, true) => std::cmp::Ordering::Greater,
            _ => a.name.cmp(&b.name),
        },
    );
    Ok(entries)
}

struct FileResponseOptions<'a> {
    size: u64,
    path: &'a str,
    mime_type: &'a str,
    head: bool,
    cache_policy: CachePolicy,
    set_key_cookie: Option<&'a str>,
    headers: &'a HeaderMap,
}

async fn serve_file<S: Store>(
    tree: &HashTree<S>,
    cid: &Cid,
    options: FileResponseOptions<'_>,
) -> Result<Response, (StatusCode, String)> {
    let etag = etag_for(cid);
    if options
        .headers
        .get(IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|part| part.trim() == etag))
    {
        return try_finish_response(
            response_builder(StatusCode::NOT_MODIFIED, options.head).header(ETAG, etag),
            Body::empty(),
        );
    }

    let range = if options.size == 0 {
        None
    } else {
        match options
            .headers
            .get(RANGE)
            .and_then(|value| value.to_str().ok())
        {
            Some(value) => Some(parse_byte_range(value, options.size).map_err(|message| {
                (
                    StatusCode::RANGE_NOT_SATISFIABLE,
                    format!("invalid range: {message}"),
                )
            })?),
            None => None,
        }
    };

    let (start, end_exclusive, status) = if let Some((start, end)) = range {
        (start, Some(end), StatusCode::PARTIAL_CONTENT)
    } else {
        (0, None, StatusCode::OK)
    };
    let bytes = tree
        .read_file_range_cid(cid, start, end_exclusive)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("file not found: {}", options.path),
            )
        })?;

    let body_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let mut builder = response_builder(status, options.head)
        .header(CONTENT_TYPE, options.mime_type)
        .header(CONTENT_LENGTH, body_len.to_string())
        .header(ACCEPT_RANGES, "bytes")
        .header(ETAG, etag)
        .header(CACHE_CONTROL, cache_control(options.cache_policy))
        .header(X_CONTENT_TYPE_OPTIONS, "nosniff");
    if let Some(key) = options.set_key_cookie
        && let Some(cookie) = key_cookie_value(key)
    {
        builder = builder.header(SET_COOKIE, cookie);
    }
    if status == StatusCode::PARTIAL_CONTENT {
        let end = start.saturating_add(body_len).saturating_sub(1);
        builder = builder.header(
            CONTENT_RANGE,
            format!(
                "bytes {start}-{end}/{}",
                options.size.max(end.saturating_add(1))
            ),
        );
    }
    try_finish_response(
        builder,
        if options.head {
            Body::empty()
        } else {
            Body::from(bytes)
        },
    )
}

#[must_use]
pub fn local_drive_url(port: u16, drive_id: &str) -> String {
    format!("http://{drive_id}.drive.iris.localhost:{port}/")
}

#[must_use]
pub fn local_portal_url(port: u16) -> String {
    local_mutable_site_url(port, IRIS_SITES_PORTAL_NPUB, IRIS_SITES_PORTAL_TREE)
}

#[must_use]
pub fn local_mutable_site_url(port: u16, npub: &str, tree_name: &str) -> String {
    let mut url = local_mutable_site_origin(Some(port), npub, tree_name);
    url.push('/');
    url
}

#[must_use]
pub fn local_iris_url(port: u16) -> String {
    format!("http://{LOCAL_PORTAL_HOST}:{port}/")
}

#[must_use]
pub fn local_portal_npub_path_url(
    port: u16,
    npub: &str,
    tree_name: &str,
    path_segments: &[String],
) -> String {
    let mut url = format!(
        "http://{LOCAL_PORTAL_HOST}:{port}/{}/{}",
        percent_encode_path_segment(npub),
        percent_encode_path_segment(tree_name)
    );
    for segment in path_segments {
        url.push('/');
        url.push_str(&percent_encode_path_segment(segment));
    }
    url
}

fn local_mutable_site_origin(port: Option<u16>, npub: &str, tree_name: &str) -> String {
    match port {
        Some(port) => format!("http://{tree_name}.{npub}.iris.localhost:{port}"),
        None => format!("http://{tree_name}.{npub}.iris.localhost"),
    }
}

#[must_use]
pub fn local_immutable_url(port: u16, cid: &Cid) -> String {
    let nhash = nhash_encode_full(&NHashData {
        hash: cid.hash,
        decrypt_key: cid.key,
    })
    .ok();
    let label = encode_immutable_host_label(&cid.hash);
    let key = cid
        .key
        .map(|key| format!("?k={}", to_hex(&key)))
        .unwrap_or_default();
    match nhash {
        Some(_) => format!("http://{label}.sites.iris.localhost:{port}/{key}"),
        None => format!("http://{label}.sites.iris.localhost:{port}/"),
    }
}

#[must_use]
pub fn local_nhash_url(port: u16, nhash: &str, filename_hint: Option<&str>) -> String {
    let mut path = format!(
        "/{}",
        percent_encode_path_segment(&nhash.to_ascii_lowercase())
    );
    if let Some(hint) = filename_hint.filter(|hint| !hint.is_empty()) {
        path.push('/');
        path.push_str(&percent_encode_path_segment(hint));
    }
    format!("http://{LOCAL_NHASH_RESOLVER_HOST}:{port}{path}")
}

#[cfg(test)]
mod tests;
