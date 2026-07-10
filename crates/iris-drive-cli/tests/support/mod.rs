use std::collections::{BTreeMap, BTreeSet};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{
        Path as AxumPath, State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::Response,
    routing::{get, put},
};
use futures::{SinkExt, StreamExt};
use hashtree_core::{sha256, to_hex};
use iris_drive_core::{AppConfig, paths::config_path_in};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, broadcast};

#[derive(Clone)]
struct LocalRelayState {
    events: Arc<Mutex<Vec<Value>>>,
    broadcasts: broadcast::Sender<Value>,
    drop_kinds: Arc<StdMutex<BTreeSet<u64>>>,
    reject_kinds: Arc<StdMutex<BTreeSet<u64>>>,
}

pub(crate) struct LocalNostrRelay {
    pub(crate) url: String,
    task: tokio::task::JoinHandle<()>,
    #[allow(dead_code)]
    drop_kinds: Arc<StdMutex<BTreeSet<u64>>>,
    #[allow(dead_code)]
    reject_kinds: Arc<StdMutex<BTreeSet<u64>>>,
    #[allow(dead_code)]
    events: Arc<Mutex<Vec<Value>>>,
}

impl LocalNostrRelay {
    pub(crate) async fn spawn() -> Self {
        let (broadcasts, _rx) = broadcast::channel(256);
        let state = LocalRelayState {
            events: Arc::new(Mutex::new(Vec::new())),
            broadcasts,
            drop_kinds: Arc::new(StdMutex::new(BTreeSet::new())),
            reject_kinds: Arc::new(StdMutex::new(BTreeSet::new())),
        };
        let drop_kinds = state.drop_kinds.clone();
        let reject_kinds = state.reject_kinds.clone();
        let events = state.events.clone();
        let app = Router::new().route("/", get(relay_ws)).with_state(state);
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url: format!("ws://{addr}"),
            task,
            drop_kinds,
            reject_kinds,
            events,
        }
    }

    #[allow(dead_code)]
    pub(crate) fn drop_kinds(&self, kinds: &[u16]) {
        self.drop_kinds
            .lock()
            .unwrap()
            .extend(kinds.iter().map(|kind| u64::from(*kind)));
    }

    #[allow(dead_code)]
    pub(crate) fn reject_kinds(&self, kinds: &[u16]) {
        self.reject_kinds
            .lock()
            .unwrap()
            .extend(kinds.iter().map(|kind| u64::from(*kind)));
    }

    #[allow(dead_code)]
    pub(crate) async fn events(&self) -> Vec<Value> {
        self.events.lock().await.clone()
    }

    // Relay fixtures expose a uniform async API even when this accessor is local-only.
    #[allow(clippy::unused_async)]
    pub(crate) async fn pending_approval_request_url(
        &self,
        config_dir: &std::path::Path,
    ) -> String {
        let config = AppConfig::load_or_default(config_path_in(config_dir)).unwrap();
        config
            .profile
            .as_ref()
            .and_then(|profile| profile.outbound_app_key_link_request.as_ref())
            .expect("pending app-key approval request")
            .request_url
            .clone()
    }
}

impl Drop for LocalNostrRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone)]
struct Subscription {
    id: String,
    filters: Vec<Value>,
}

async fn relay_ws(ws: WebSocketUpgrade, State(state): State<LocalRelayState>) -> Response {
    ws.on_upgrade(move |socket| relay_socket(socket, state))
}

async fn relay_socket(socket: WebSocket, state: LocalRelayState) {
    let (mut sender, mut receiver) = socket.split();
    let mut broadcasts = state.broadcasts.subscribe();
    let mut subscriptions: Vec<Subscription> = Vec::new();

    loop {
        tokio::select! {
            message = receiver.next() => {
                let Some(Ok(message)) = message else {
                    break;
                };
                let text = match message {
                    WsMessage::Text(text) => text,
                    WsMessage::Ping(bytes) => {
                        let _ = sender.send(WsMessage::Pong(bytes)).await;
                        continue;
                    }
                    WsMessage::Close(_) => break,
                    _ => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                let Some(items) = value.as_array() else {
                    continue;
                };
                let Some(command) = items.first().and_then(Value::as_str) else {
                    continue;
                };
                match command {
                    "EVENT" => {
                        let Some(event) = items.get(1).cloned() else {
                            continue;
                        };
                        let event_id = event["id"].as_str().unwrap_or_default().to_string();
                        let kind = event["kind"].as_u64().unwrap_or_default();
                        if state.reject_kinds.lock().unwrap().contains(&kind) {
                            let _ = sender
                                .send(WsMessage::Text(
                                    json!(["OK", event_id, false, "rejected by test relay"])
                                        .to_string(),
                                ))
                                .await;
                            continue;
                        }
                        if state.drop_kinds.lock().unwrap().contains(&kind) {
                            let _ = sender
                                .send(WsMessage::Text(json!(["OK", event_id, true, ""]).to_string()))
                                .await;
                            continue;
                        }
                        state.events.lock().await.push(event.clone());
                        let _ = state.broadcasts.send(event);
                        let _ = sender
                            .send(WsMessage::Text(json!(["OK", event_id, true, ""]).to_string()))
                            .await;
                    }
                    "REQ" => {
                        let Some(subscription_id) = items.get(1).and_then(Value::as_str) else {
                            continue;
                        };
                        let filters = items.iter().skip(2).cloned().collect::<Vec<_>>();
                        subscriptions.push(Subscription {
                            id: subscription_id.to_string(),
                            filters: filters.clone(),
                        });
                        let events = state.events.lock().await.clone();
                        for event in events {
                            if filters.iter().any(|filter| event_matches_filter(&event, filter)) {
                                let _ = sender
                                    .send(WsMessage::Text(
                                        json!(["EVENT", subscription_id, event]).to_string(),
                                    ))
                                    .await;
                            }
                        }
                        let _ = sender
                            .send(WsMessage::Text(json!(["EOSE", subscription_id]).to_string()))
                            .await;
                    }
                    "CLOSE" => {
                        if let Some(subscription_id) = items.get(1).and_then(Value::as_str) {
                            subscriptions.retain(|subscription| subscription.id != subscription_id);
                        }
                    }
                    _ => {}
                }
            }
            event = broadcasts.recv() => {
                let Ok(event) = event else {
                    continue;
                };
                for subscription in &subscriptions {
                    if subscription
                        .filters
                        .iter()
                        .any(|filter| event_matches_filter(&event, filter))
                    {
                        let _ = sender
                            .send(WsMessage::Text(
                                json!(["EVENT", subscription.id, event]).to_string(),
                            ))
                            .await;
                    }
                }
            }
        }
    }
}

pub(crate) fn add_config_relay(config_dir: &std::path::Path, relay_url: &str) {
    let config_path = config_path_in(config_dir);
    let mut config = AppConfig::load_or_default(&config_path).unwrap();
    config.relays = vec![relay_url.to_owned()];
    config.save(config_path).unwrap();
}

fn event_matches_filter(event: &Value, filter: &Value) -> bool {
    if let Some(kinds) = filter.get("kinds").and_then(Value::as_array) {
        let Some(kind) = event.get("kind").and_then(Value::as_u64) else {
            return false;
        };
        if !kinds
            .iter()
            .any(|candidate| candidate.as_u64() == Some(kind))
        {
            return false;
        }
    }
    if let Some(authors) = filter.get("authors").and_then(Value::as_array) {
        let Some(author) = event.get("pubkey").and_then(Value::as_str) else {
            return false;
        };
        if !authors
            .iter()
            .any(|candidate| candidate.as_str() == Some(author))
        {
            return false;
        }
    }
    if let Some(d_values) = filter.get("#d").and_then(Value::as_array) {
        let Some(tags) = event.get("tags").and_then(Value::as_array) else {
            return false;
        };
        let has_matching_d_tag = tags.iter().any(|tag| {
            let Some(tag_items) = tag.as_array() else {
                return false;
            };
            tag_items.first().and_then(Value::as_str) == Some("d")
                && tag_items
                    .get(1)
                    .and_then(Value::as_str)
                    .is_some_and(|value| {
                        d_values
                            .iter()
                            .any(|candidate| candidate.as_str() == Some(value))
                    })
        });
        if !has_matching_d_tag {
            return false;
        }
    }
    true
}

#[derive(Clone)]
struct LocalBlossomState {
    blobs: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    upload_delay: Duration,
}

pub(crate) struct LocalBlossomServer {
    pub(crate) url: String,
    #[allow(dead_code)]
    state: LocalBlossomState,
    task: tokio::task::JoinHandle<()>,
}

impl LocalBlossomServer {
    #[allow(dead_code)]
    pub(crate) async fn spawn() -> Self {
        Self::spawn_with_upload_delay(Duration::ZERO).await
    }

    pub(crate) async fn spawn_with_upload_delay(upload_delay: Duration) -> Self {
        let state = LocalBlossomState {
            blobs: Arc::new(Mutex::new(BTreeMap::new())),
            upload_delay,
        };
        let app = Router::new()
            .route("/upload", put(blossom_upload))
            .route("/:name", get(blossom_get).head(blossom_head))
            .with_state(state.clone());
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url: format!("http://{addr}"),
            state,
            task,
        }
    }
    #[allow(dead_code)]
    pub(crate) async fn blob_count(&self) -> usize {
        self.state.blobs.lock().await.len()
    }
}

impl Drop for LocalBlossomServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn blossom_upload(
    State(state): State<LocalBlossomState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !state.upload_delay.is_zero() {
        tokio::time::sleep(state.upload_delay).await;
    }
    let hash = to_hex(&sha256(&body));
    if let Some(expected) = headers
        .get("x-sha-256")
        .and_then(|value| value.to_str().ok())
        && expected != hash
    {
        return text_response(StatusCode::BAD_REQUEST, "hash mismatch");
    }
    let mut blobs = state.blobs.lock().await;
    if blobs.contains_key(&hash) {
        return text_response(StatusCode::CONFLICT, "already exists");
    }
    blobs.insert(hash, body.to_vec());
    text_response(StatusCode::CREATED, "created")
}

async fn blossom_get(
    State(state): State<LocalBlossomState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    let Some(hash) = name.strip_suffix(".bin") else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };
    let Some(bytes) = state.blobs.lock().await.get(hash).cloned() else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };
    blob_response(StatusCode::OK, bytes)
}

async fn blossom_head(
    State(state): State<LocalBlossomState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    let Some(hash) = name.strip_suffix(".bin") else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };
    let Some(size) = state.blobs.lock().await.get(hash).map(Vec::len) else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, size.to_string())
        .body(Body::empty())
        .unwrap()
}

fn blob_response(status: StatusCode, bytes: Vec<u8>) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .body(Body::from(bytes))
        .unwrap()
}

fn text_response(status: StatusCode, text: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(text.to_string()))
        .unwrap()
}
