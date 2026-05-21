//! Loopback HTTP gateway for serving hashtree-backed Iris Drive content.
//!
//! Browser-facing origins use `*.localhost` names so stock browsers can
//! treat them as secure contexts without a custom CA or browser fork.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::State;
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE, COOKIE, ETAG, HOST,
    IF_NONE_MATCH, RANGE, SET_COOKIE, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::Response;
use axum::routing::any;
use hashtree_core::{
    Cid, Hash, HashTree, LinkType, NHashData, TreeEntry, from_hex, nhash_decode, nhash_encode_full,
    to_hex,
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

const LOCAL_PORTAL_HOST: &str = "sites.iris.localhost";
const IMMUTABLE_HOST_SUFFIX: &str = ".sites.iris.localhost";
const HASH_HOST_SUFFIX: &str = ".hash.localhost";
const DRIVE_HOST_SUFFIX: &str = ".drive.iris.localhost";
const SHORT_DRIVE_HOST_SUFFIX: &str = ".iris.localhost";
const KEY_COOKIE: &str = "iris_htree_key";

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
        let listener = TcpListener::bind(bind.addr).await?;
        let local_addr = listener.local_addr()?;
        let state = GatewayState {
            config_dir: Arc::new(config_dir.into()),
            tree,
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
}

#[derive(Debug, Clone)]
struct GatewayRequest {
    root: Cid,
    path_segments: Vec<String>,
    cache_policy: CachePolicy,
    set_key_cookie: Option<String>,
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
) -> Response {
    match handle_gateway_request(state, method, uri, headers).await {
        Ok(response) => response,
        Err((status, message)) => text_response(status, &message),
    }
}

async fn handle_gateway_request(
    state: GatewayState,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> Result<Response, (StatusCode, String)> {
    if method != Method::GET && method != Method::HEAD {
        return Err((StatusCode::METHOD_NOT_ALLOWED, "method not allowed".into()));
    }

    let request = resolve_gateway_request(&state, &uri, &headers)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
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

fn resolve_gateway_request(
    state: &GatewayState,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<GatewayRequest, GatewayError> {
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
        return request_from_path_route(state, uri, headers, route, path_segments);
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
        return drive_host_request(state, drive_id, path_segments);
    }

    if let Some(drive_id) = host.strip_suffix(SHORT_DRIVE_HOST_SUFFIX)
        && drive_id == PRIMARY_DRIVE_ID
    {
        return drive_host_request(state, drive_id, path_segments);
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
    state: &GatewayState,
    uri: &Uri,
    headers: &HeaderMap,
    route: PathRoute,
    path_segments: Vec<String>,
) -> Result<GatewayRequest, GatewayError> {
    match route {
        PathRoute::Drive(drive_id) => drive_host_request(state, &drive_id, path_segments),
        PathRoute::Nhash(nhash) => {
            let cid = cid_from_nhash(&nhash)?;
            let (cid, set_key_cookie) = cid_with_request_key(cid, uri, headers)?;
            Ok(GatewayRequest {
                root: cid,
                path_segments,
                cache_policy: CachePolicy::Immutable,
                set_key_cookie,
            })
        }
    }
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
    Ok(GatewayRequest {
        root: cid,
        path_segments,
        cache_policy: CachePolicy::Immutable,
        set_key_cookie,
    })
}

fn drive_host_request(
    state: &GatewayState,
    drive_id: &str,
    path_segments: Vec<String>,
) -> Result<GatewayRequest, GatewayError> {
    if !is_safe_drive_id(drive_id) {
        return Err(GatewayError::InvalidRequest("invalid drive id".into()));
    }
    let root = current_drive_root(&state.config_dir, drive_id)?;
    Ok(GatewayRequest {
        root,
        path_segments,
        cache_policy: CachePolicy::Mutable,
        set_key_cookie: None,
    })
}

fn current_drive_root(config_dir: &Path, drive_id: &str) -> Result<Cid, GatewayError> {
    if !key_path_in(config_dir).exists() {
        return Err(GatewayError::InvalidRequest(
            "iris-drive is not initialized".into(),
        ));
    }
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let drive = config
        .drive(drive_id)
        .ok_or_else(|| GatewayError::InvalidRequest(format!("drive {drive_id} not found")))?;
    let root_cid = config
        .account
        .as_ref()
        .and_then(|account| drive.device_roots.get(&account.device_pubkey))
        .map(|root| root.root_cid.as_str())
        .or(drive.last_root_cid.as_deref())
        .ok_or_else(|| GatewayError::InvalidRequest(format!("drive {drive_id} has no root")))?;
    Cid::parse(root_cid).map_err(|e| GatewayError::InvalidRequest(e.to_string()))
}

async fn resolve_content(
    tree: &HashTree<FsBlobStore>,
    root: &Cid,
    segments: &[String],
) -> Result<Option<ResolvedContent>, GatewayError> {
    let mut current = root.clone();
    if segments.is_empty() {
        return resolve_directory_or_index(tree, current, "").await;
    }

    for (index, segment) in segments.iter().enumerate() {
        if segment == ".hashtree" {
            return Err(GatewayError::InvalidRequest(
                "internal metadata is not served".into(),
            ));
        }
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

async fn resolve_directory_or_index(
    tree: &HashTree<FsBlobStore>,
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

async fn find_entry(
    tree: &HashTree<FsBlobStore>,
    dir: &Cid,
    name: &str,
) -> Result<Option<TreeEntry>, GatewayError> {
    let entries = tree
        .list_directory(dir)
        .await
        .map_err(|e| GatewayError::Hashtree(e.to_string()))?;
    Ok(entries.into_iter().find(|entry| entry.name == name))
}

async fn list_public_directory(
    tree: &HashTree<FsBlobStore>,
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

async fn serve_file(
    tree: &HashTree<FsBlobStore>,
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

    let range = match options
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

fn directory_response(
    entries: &[TreeEntry],
    display_path: &str,
    head: bool,
    cache_policy: CachePolicy,
    set_key_cookie: Option<&str>,
) -> Response {
    let mut html = String::new();
    html.push_str("<!doctype html><meta charset=\"utf-8\"><title>Iris Drive</title>");
    html.push_str("<style>body{font:15px system-ui,sans-serif;max-width:860px;margin:32px auto;padding:0 16px;color:#111}a{color:#0645ad;text-decoration:none}a:hover{text-decoration:underline}ul{line-height:1.9;padding-left:1.2rem}.muted{color:#666}</style>");
    html.push_str("<h1>");
    if display_path.is_empty() {
        html.push('/');
    } else {
        html.push_str(&html_escape(display_path));
    }
    html.push_str("</h1><ul>");
    if !display_path.is_empty() {
        html.push_str("<li><a href=\"../\">../</a></li>");
    }
    for entry in entries {
        let suffix = if entry.link_type == LinkType::Dir {
            "/"
        } else {
            ""
        };
        let href = format!("{}{}", percent_encode_path_segment(&entry.name), suffix);
        html.push_str("<li><a href=\"");
        html.push_str(&href);
        html.push_str("\">");
        html.push_str(&html_escape(&entry.name));
        html.push_str(suffix);
        html.push_str("</a>");
        if entry.link_type != LinkType::Dir {
            html.push_str(" <span class=\"muted\">");
            html.push_str(&entry.size.to_string());
            html.push_str(" bytes</span>");
        }
        html.push_str("</li>");
    }
    html.push_str("</ul>");

    let bytes = html.into_bytes();
    let mut builder = response_builder(StatusCode::OK, head)
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .header(CONTENT_LENGTH, bytes.len().to_string())
        .header(CACHE_CONTROL, cache_control(cache_policy))
        .header(X_CONTENT_TYPE_OPTIONS, "nosniff");
    if let Some(key) = set_key_cookie {
        builder = builder.header(SET_COOKIE, key_cookie_value(key));
    }
    builder
        .body(if head {
            Body::empty()
        } else {
            Body::from(bytes)
        })
        .expect("response")
}

fn text_response(status: StatusCode, message: &str) -> Response {
    response_builder(status, false)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(message.to_string()))
        .expect("response")
}

fn response_builder(status: StatusCode, _head: bool) -> http::response::Builder {
    Response::builder().status(status)
}

fn entry_cid(entry: &TreeEntry) -> Cid {
    Cid {
        hash: entry.hash,
        key: entry.key,
    }
}

fn cid_from_nhash(value: &str) -> Result<Cid, GatewayError> {
    let NHashData { hash, decrypt_key } =
        nhash_decode(value).map_err(|e| GatewayError::InvalidRequest(e.to_string()))?;
    Ok(Cid {
        hash,
        key: decrypt_key,
    })
}

fn cid_with_request_key(
    mut cid: Cid,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<(Cid, Option<String>), GatewayError> {
    if cid.key.is_some() {
        return Ok((cid, None));
    }
    let key = query_param(uri.query(), "k").or_else(|| cookie_value(headers, KEY_COOKIE));
    let Some(key) = key else {
        return Ok((cid, None));
    };
    let parsed = from_hex(&key).map_err(|_| GatewayError::InvalidRequest("invalid key".into()))?;
    cid.key = Some(parsed);
    Ok((cid, Some(to_hex(&parsed))))
}

fn parse_gateway_path(path: &str) -> Result<(Vec<String>, Option<PathRoute>), GatewayError> {
    let mut segments = decode_path_segments(path)?;
    if segments.first().is_some_and(|segment| segment == "drive") {
        if segments.len() < 2 {
            return Err(GatewayError::InvalidRequest("missing drive id".into()));
        }
        let drive_id = segments.remove(1);
        segments.remove(0);
        return Ok((segments, Some(PathRoute::Drive(drive_id))));
    }
    if segments.first().is_some_and(|segment| segment == "nhash") {
        if segments.len() < 2 {
            return Err(GatewayError::InvalidRequest("missing nhash".into()));
        }
        let nhash = segments.remove(1);
        segments.remove(0);
        return Ok((segments, Some(PathRoute::Nhash(nhash))));
    }
    Ok((segments, None))
}

fn decode_path_segments(path: &str) -> Result<Vec<String>, GatewayError> {
    let mut out = Vec::new();
    for raw in path.split('/').filter(|segment| !segment.is_empty()) {
        let segment = percent_decode(raw)?;
        if segment == "." || segment == ".." || segment.contains('\0') {
            return Err(GatewayError::InvalidRequest("invalid path segment".into()));
        }
        out.push(segment);
    }
    Ok(out)
}

fn percent_decode(value: &str) -> Result<String, GatewayError> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(GatewayError::InvalidRequest("bad percent encoding".into()));
            }
            let hi = hex_value(bytes[i + 1])
                .ok_or_else(|| GatewayError::InvalidRequest("bad percent encoding".into()))?;
            let lo = hex_value(bytes[i + 2])
                .ok_or_else(|| GatewayError::InvalidRequest("bad percent encoding".into()))?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| GatewayError::InvalidRequest("path is not utf-8".into()))
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn query_param(query: Option<&str>, name: &str) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode(key).ok().as_deref() == Some(name) {
            return percent_decode(value).ok();
        }
    }
    None
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name).then(|| value.to_string())
    })
}

fn key_cookie_value(key: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{KEY_COOKIE}={key}; Path=/; HttpOnly; SameSite=Strict"
    ))
    .expect("valid cookie")
}

fn parse_byte_range(value: &str, size: u64) -> Result<(u64, u64), String> {
    let range = value
        .strip_prefix("bytes=")
        .ok_or_else(|| "only bytes ranges are supported".to_string())?;
    let (start_raw, end_raw) = range
        .split_once('-')
        .ok_or_else(|| "missing range delimiter".to_string())?;
    if start_raw.is_empty() {
        let suffix = end_raw
            .parse::<u64>()
            .map_err(|_| "invalid suffix range".to_string())?;
        if suffix == 0 {
            return Err("empty suffix range".into());
        }
        let start = size.saturating_sub(suffix);
        return Ok((start, size));
    }

    let start = start_raw
        .parse::<u64>()
        .map_err(|_| "invalid start".to_string())?;
    let end_inclusive = if end_raw.is_empty() {
        size.saturating_sub(1)
    } else {
        end_raw
            .parse::<u64>()
            .map_err(|_| "invalid end".to_string())?
    };
    if start >= size || end_inclusive < start {
        return Err("range outside file".into());
    }
    Ok((start, end_inclusive.saturating_add(1).min(size)))
}

fn cache_control(policy: CachePolicy) -> &'static str {
    match policy {
        CachePolicy::Immutable => "public, max-age=31536000, immutable",
        CachePolicy::Mutable => "no-cache",
    }
}

fn etag_for(cid: &Cid) -> String {
    format!("\"{}\"", to_hex(&cid.hash))
}

fn mime_type_for_path(
    path: &str,
    meta: Option<&std::collections::HashMap<String, serde_json::Value>>,
) -> String {
    if let Some(mime) = meta
        .and_then(|meta| meta.get("mimeType"))
        .and_then(serde_json::Value::as_str)
        .filter(|mime| !mime.trim().is_empty())
    {
        return mime.to_string();
    }
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

fn append_path(path: &str, child: &str) -> String {
    if path.is_empty() {
        child.to_string()
    } else {
        format!("{path}/{child}")
    }
}

fn normalize_host(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if let Some(inner) = trimmed
        .strip_prefix('[')
        .and_then(|v| v.split_once(']').map(|(h, _)| h))
    {
        return inner.to_string();
    }
    trimmed
        .rsplit_once(':')
        .and_then(|(head, tail)| tail.parse::<u16>().ok().map(|_| head.to_string()))
        .unwrap_or(trimmed)
}

fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

fn is_safe_drive_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_'))
}

fn percent_encode_path_segment(segment: &str) -> String {
    let mut encoded = String::new();
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(&mut encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

fn html_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

const BASE32_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

#[must_use]
pub fn encode_immutable_host_label(hash: &Hash) -> String {
    let mut bits = 0u32;
    let mut value = 0u32;
    let mut output = String::new();
    for byte in hash {
        value = (value << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            let index = ((value >> (bits - 5)) & 31) as usize;
            output.push(char::from(BASE32_ALPHABET[index]));
            bits -= 5;
        }
    }
    if bits > 0 {
        let index = ((value << (5 - bits)) & 31) as usize;
        output.push(char::from(BASE32_ALPHABET[index]));
    }
    output
}

fn decode_base32_hash(label: &str) -> Option<Hash> {
    let mut bits = 0u32;
    let mut current = 0u32;
    let mut bytes = Vec::with_capacity(32);
    for ch in label.trim().bytes() {
        let index = BASE32_ALPHABET.iter().position(|b| *b == ch)?;
        current = (current << 5) | u32::try_from(index).ok()?;
        bits += 5;
        if bits >= 8 {
            bytes.push(((current >> (bits - 8)) & 0xff) as u8);
            bits -= 8;
        }
    }
    if bytes.len() != 32 {
        return None;
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Some(hash)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::Account;
    use crate::config::Drive;
    use crate::paths::config_path_in;
    use tempfile::tempdir;

    fn init_account_config(dir: &Path) {
        let account = Account::create(dir, Some("gateway-test".into())).unwrap();
        let mut cfg = AppConfig {
            account: Some(account.state.clone()),
            ..AppConfig::default()
        };
        cfg.upsert_drive(Drive::primary(account.state.owner_pubkey.clone()));
        cfg.save(config_path_in(dir)).unwrap();
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
        daemon.import_working_dir(work.path()).await.unwrap();

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

    #[tokio::test]
    async fn gateway_serves_immutable_hash_host() {
        let cfg_dir = tempdir().unwrap();
        init_account_config(cfg_dir.path());
        let work = tempdir().unwrap();
        std::fs::write(work.path().join("app.js"), b"console.log('iris')").unwrap();

        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        let report = daemon.import_working_dir(work.path()).await.unwrap();
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

    async fn http_get(addr: SocketAddr, host: &str, path: &str) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let request = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n\r\n");
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        String::from_utf8_lossy(&response).into_owned()
    }
}
