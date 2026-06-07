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
use axum::http::header::{
    ACCEPT_RANGES, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE,
    COOKIE, ETAG, HOST, IF_NONE_MATCH, ORIGIN, RANGE, SET_COOKIE, VARY, X_CONTENT_TYPE_OPTIONS,
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
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::config::{AppConfig, ConfigError};
use crate::daemon::DaemonError;
use crate::paths::{config_path_in, key_path_in};
use crate::{Daemon, PRIMARY_DRIVE_ID};

mod paths;
mod response;

pub use self::paths::encode_immutable_host_label;
#[allow(clippy::wildcard_imports)]
use self::paths::*;
#[allow(clippy::wildcard_imports)]
use self::response::*;

const LOCAL_PORTAL_HOST: &str = "sites.iris.localhost";
pub const LOCAL_NHASH_RESOLVER_HOST: &str = "nhash.iris.localhost";
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
        Self::loopback_v4(17_321)
    }
}

/// Running loopback gateway. Drop it to request shutdown.
pub struct GatewayServer {
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
struct HtreeProxyRequest {
    nhash: String,
    path_segments: Vec<String>,
    key_query: Option<String>,
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
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    match handle_gateway_request(state, method, uri, headers, body).await {
        Ok(response) => response,
        Err((status, message)) => text_response(status, &message),
    }
}

async fn handle_gateway_request(
    state: GatewayState,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    if uri.path() == SHARE_ACTION_API_PATH {
        return handle_share_action_api(&state, &method, &headers, body.as_ref());
    }

    let request = resolve_gateway_request(&uri, &headers)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let request = match request {
        GatewayRequest::Local(request) => request,
        GatewayRequest::Drive(request) => materialize_drive_gateway_request(&state, request)
            .await
            .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?,
        GatewayRequest::HtreeDaemon(request) => {
            return proxy_htree_daemon_request(&state, &method, &headers, request).await;
        }
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
        return Ok(
            share_action_response_builder(StatusCode::NO_CONTENT, cors_origin.as_ref())
                .body(Body::empty())
                .expect("response"),
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
    Ok(share_action_response_builder(StatusCode::OK, cors_origin)
        .header(CONTENT_TYPE, "application/json")
        .header(CACHE_CONTROL, "no-store")
        .body(Body::from(body))
        .expect("response"))
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

async fn proxy_htree_daemon_request(
    state: &GatewayState,
    method: &Method,
    headers: &HeaderMap,
    request: HtreeProxyRequest,
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
    for header in [RANGE, IF_NONE_MATCH, axum::http::header::ACCEPT] {
        if let Some(value) = headers.get(&header) {
            upstream = upstream.header(header.as_str(), value.as_bytes());
        }
    }

    let upstream = upstream
        .send()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("hashtree daemon: {e}")))?;
    let status = StatusCode::from_u16(upstream.status().as_u16())
        .map_err(|e| (StatusCode::BAD_GATEWAY, e.to_string()))?;
    let mut builder = response_builder(status, method == Method::HEAD);
    for (name, value) in upstream.headers() {
        if is_proxy_response_header(name.as_str()) {
            builder = builder.header(name.as_str(), value.as_bytes());
        }
    }
    if let Some(key) = request.key_query.as_deref() {
        builder = builder.header(SET_COOKIE, key_cookie_value(key));
    }
    let bytes = upstream
        .bytes()
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("hashtree daemon: {e}")))?;
    Ok(builder
        .body(if method == Method::HEAD {
            Body::empty()
        } else {
            Body::from(bytes)
        })
        .expect("response"))
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

fn htree_daemon_target(request: &HtreeProxyRequest) -> String {
    let mut target = format!("/htree/{}", percent_encode_path_segment(&request.nhash));
    for segment in &request.path_segments {
        target.push('/');
        target.push_str(&percent_encode_path_segment(segment));
    }
    if let Some(key) = request.key_query.as_deref() {
        target.push_str("?k=");
        target.push_str(&percent_encode_path_segment(key));
    }
    target
}

fn resolve_gateway_request(uri: &Uri, headers: &HeaderMap) -> Result<GatewayRequest, GatewayError> {
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| GatewayError::InvalidRequest("missing Host header".into()))?;
    let host = normalize_host(host);
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
        return Err(GatewayError::InvalidRequest(
            "use a content host such as main.drive.iris.localhost".into(),
        ));
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
        return nhash_request(&nhash, uri, headers, path_segments);
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
        PathRoute::Nhash(nhash) => nhash_request(&nhash, uri, headers, path_segments),
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
    nhash_request(&nhash, uri, headers, path_segments)
}

fn nhash_request(
    nhash: &str,
    uri: &Uri,
    headers: &HeaderMap,
    path_segments: Vec<String>,
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
    Ok(GatewayRequest::HtreeDaemon(HtreeProxyRequest {
        nhash: nhash.to_string(),
        path_segments,
        key_query,
    }))
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
                LinkType::Dir => resolve_directory_or_index(tree, cid, &segments.join("/")).await,
                LinkType::Blob | LinkType::File => Ok(Some(ResolvedContent::File {
                    cid,
                    size: entry.size,
                    path: segments.join("/"),
                    mime_type: mime_type_for_path(&segments.join("/"), entry.meta.as_ref()),
                })),
            };
        }
        if entry.link_type != LinkType::Dir {
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
            LinkType::Dir | LinkType::Blob => Ok(None),
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
        return Ok(response_builder(StatusCode::NOT_MODIFIED, options.head)
            .header(ETAG, etag)
            .body(Body::empty())
            .expect("response"));
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
    if let Some(key) = options.set_key_cookie {
        builder = builder.header(SET_COOKIE, key_cookie_value(key));
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
    Ok(builder
        .body(if options.head {
            Body::empty()
        } else {
            Body::from(bytes)
        })
        .expect("response"))
}

#[must_use]
pub fn local_drive_url(port: u16, drive_id: &str) -> String {
    format!("http://{drive_id}.drive.iris.localhost:{port}/")
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
