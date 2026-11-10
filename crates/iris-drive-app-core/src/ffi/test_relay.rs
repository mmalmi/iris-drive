//! Loopback Nostr relay for tests that sync, so they neither wait on nor
//! publish to the public default relays. It stores events and answers queries.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::Response,
    routing::get,
};
use futures::{SinkExt, StreamExt};
use iris_drive_core::{AppConfig, paths::config_path_in};
use serde_json::{Value, json};

type Events = Arc<Mutex<Vec<Value>>>;

pub(super) struct TestRelay {
    url: String,
    _runtime: tokio::runtime::Runtime,
}

impl TestRelay {
    pub(super) fn spawn() -> Self {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind(SocketAddr::from((
                Ipv4Addr::LOCALHOST,
                0,
            ))))
            .unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let app = Router::new()
            .route("/", get(upgrade))
            .with_state(Events::default());
        runtime.spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self {
            url,
            _runtime: runtime,
        }
    }

    /// Make this the only relay of the profile stored in `config_dir`.
    pub(super) fn use_for(&self, config_dir: &Path) {
        let path = config_path_in(config_dir);
        let mut config = AppConfig::load_or_default(&path).unwrap();
        config.relays = vec![self.url.clone()];
        config.save(&path).unwrap();
    }
}

async fn upgrade(ws: WebSocketUpgrade, State(events): State<Events>) -> Response {
    ws.on_upgrade(move |socket| serve(socket, events))
}

async fn serve(socket: WebSocket, events: Events) {
    let (mut sender, mut receiver) = socket.split();
    while let Some(Ok(message)) = receiver.next().await {
        let Message::Text(text) = message else {
            continue;
        };
        let Ok(Value::Array(items)) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let replies = match items.first().and_then(Value::as_str) {
            Some("EVENT") => {
                let Some(event) = items.get(1).cloned() else {
                    continue;
                };
                let reply = json!(["OK", event["id"], true, ""]);
                events.lock().unwrap().push(event);
                vec![reply]
            }
            Some("REQ") => {
                let Some(subscription) = items.get(1) else {
                    continue;
                };
                let mut replies = events
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|event| items[2..].iter().any(|filter| matches(event, filter)))
                    .map(|event| json!(["EVENT", subscription, event]))
                    .collect::<Vec<_>>();
                replies.push(json!(["EOSE", subscription]));
                replies
            }
            _ => continue,
        };
        for reply in replies {
            if sender.send(Message::Text(reply.to_string())).await.is_err() {
                return;
            }
        }
    }
}

fn matches(event: &Value, filter: &Value) -> bool {
    let Some(filter) = filter.as_object() else {
        return false;
    };
    filter.iter().all(|(key, wanted)| {
        let Some(wanted) = wanted.as_array() else {
            // `since`, `until` and `limit` only narrow a result set that is
            // already small here.
            return true;
        };
        match key.as_str() {
            "ids" => wanted.contains(&event["id"]),
            "kinds" => wanted.contains(&event["kind"]),
            "authors" => wanted.contains(&event["pubkey"]),
            key => key.strip_prefix('#').is_none_or(|name| {
                event["tags"].as_array().is_some_and(|tags| {
                    tags.iter()
                        .any(|tag| tag[0] == name && wanted.contains(&tag[1]))
                })
            }),
        }
    })
}
