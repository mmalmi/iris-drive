//! Verified Nostr events over the shared FIPS pubsub adapter.

use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use fips_core::FipsEndpoint;
use fips_core::discovery::nostr::{ADVERT_IDENTIFIER, ADVERT_KIND};
use nostr_pubsub::{
    EventBus, EventRetentionPolicy, EventSource, FileEventBus, Filter, VerifiedEvent,
};
use nostr_pubsub_fips::{
    FIPS_NOSTR_PUBSUB_DEFAULT_MAX_HOPS, FIPS_NOSTR_PUBSUB_MAX_FRAME_BYTES, FipsPubsubClient,
    FipsPubsubClientOptions, FipsPubsubPolicyOptions,
};
use nostr_sdk::Event;
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use super::FipsSyncError;

const DELIVERY_CAPACITY: usize = 256;
const SUBSCRIPTION_RETRY_INTERVAL: Duration = Duration::from_millis(500);
const RECENT_EVENT_IDS: usize = 256;

#[derive(Debug, Clone)]
pub struct FipsNostrPubsubEvent {
    pub origin_peer_id: String,
    pub event: Event,
}

pub(super) struct DriveNostrPubsubRuntime {
    client: Option<Arc<FipsPubsubClient>>,
    announcement_store: Option<Arc<FileEventBus>>,
    update_routes: Arc<crate::updater::pubsub::UpdateRoutes>,
    deliveries: broadcast::Sender<FipsNostrPubsubEvent>,
    receiver_task: Option<JoinHandle<()>>,
}

impl DriveNostrPubsubRuntime {
    pub(super) async fn bind(
        endpoint: Arc<FipsEndpoint>,
        routed_peers: Vec<String>,
        trusted_raters: Vec<String>,
        device_directory: Option<&Path>,
    ) -> Result<Self, FipsSyncError> {
        #[cfg(feature = "stack-fixture")]
        let max_connected_peers = std::env::var("IRIS_STACK_FIXTURE_PUBSUB_MAX_PEERS")
            .map_or(64, |value| value.parse().unwrap_or(0));
        #[cfg(not(feature = "stack-fixture"))]
        let max_connected_peers = 64;
        let mut reputation = FipsPubsubPolicyOptions::default();
        reputation.reputation.trusted_raters = trusted_raters.into_iter().collect();
        let announcement_store = match device_directory {
            Some(directory) => Some(open_announcement_store(directory, &endpoint).await?),
            None => None,
        };
        let client = Arc::new(
            FipsPubsubClient::start_with_reputation(
                endpoint.clone(),
                FipsPubsubClientOptions {
                    query_timeout: Duration::from_millis(500),
                    max_frame_bytes: FIPS_NOSTR_PUBSUB_MAX_FRAME_BYTES,
                    max_connected_peers,
                    fanout: nostr_pubsub::DEFAULT_INV_WANT_FANOUT.min(max_connected_peers),
                    max_active_subscriptions: 4,
                    max_filters_per_subscription: 4,
                    max_replay_events: 32,
                    receive_batch_size: 64,
                    max_hops: FIPS_NOSTR_PUBSUB_DEFAULT_MAX_HOPS,
                    routed_peers: bounded_routed_peers(routed_peers, max_connected_peers),
                    ..FipsPubsubClientOptions::default()
                },
                reputation,
            )
            .await
            .map_err(|error| endpoint_error(error.to_string()))?,
        );
        if let Some(store) = &announcement_store
            && let Err(error) = client
                .set_peer_announcement_store(Some(store.clone()))
                .await
                .and_then(|()| client.set_replay_source(Some(store.clone())))
        {
            client.shutdown_shared().await;
            store.close().await;
            return Err(endpoint_error(error.to_string()));
        }
        let update_routes = crate::updater::pubsub::UpdateRoutes::new(&client);
        let (deliveries, _) = broadcast::channel(DELIVERY_CAPACITY);
        let task_deliveries = deliveries.clone();
        let task_client = client.clone();
        let task_announcement_store = announcement_store.clone();
        let receiver_task = tokio::spawn(async move {
            let mut recent_order = VecDeque::new();
            let mut recent_ids = HashSet::new();
            loop {
                // The adapter replays this subscription when peers arrive or
                // reconnect. Keep queued deliveries and dedup across link churn.
                let Ok(mut subscription) = task_client.subscribe(vec![Filter::new()]).await else {
                    tokio::time::sleep(SUBSCRIPTION_RETRY_INTERVAL).await;
                    continue;
                };
                while let Some(delivery) = subscription.recv().await {
                    if delivery.event.as_event().kind == nostr_sdk::Kind::Custom(ADVERT_KIND)
                        && let Err(error) = observe_announcement(
                            &endpoint,
                            task_announcement_store.as_deref(),
                            &delivery,
                        )
                        .await
                    {
                        tracing::warn!(%error, "processing Drive peer announcement");
                    }
                    let event = delivery.event.into_event();
                    let event_id = event.id.to_string();
                    if !recent_ids.insert(event_id.clone()) {
                        continue;
                    }
                    recent_order.push_back(event_id);
                    while recent_order.len() > RECENT_EVENT_IDS {
                        if let Some(expired) = recent_order.pop_front() {
                            recent_ids.remove(&expired);
                        }
                    }
                    let _ = task_deliveries.send(FipsNostrPubsubEvent {
                        origin_peer_id: delivery.source.id.0,
                        event,
                    });
                }
                tokio::time::sleep(SUBSCRIPTION_RETRY_INTERVAL).await;
            }
        });
        Ok(Self {
            client: Some(client),
            announcement_store,
            update_routes,
            deliveries,
            receiver_task: Some(receiver_task),
        })
    }

    #[cfg(feature = "stack-fixture")]
    pub(super) fn stack_fixture_client(&self) -> Option<&FipsPubsubClient> {
        self.client.as_deref()
    }

    pub(super) fn subscribe(&self) -> broadcast::Receiver<FipsNostrPubsubEvent> {
        self.deliveries.subscribe()
    }

    pub(super) fn register_updater(&self, config_dir: &std::path::Path) {
        if let Some(client) = &self.client {
            crate::updater::pubsub::register_routed(config_dir, client, &self.update_routes);
        }
    }

    pub(super) fn max_connected_peers(&self) -> Option<usize> {
        self.client
            .as_ref()
            .map(|client| client.options().max_connected_peers)
    }

    pub(super) fn connected_peer_count(&self) -> usize {
        match self.client.as_ref() {
            Some(client) => client.connected_peer_count().unwrap_or_default(),
            None => 0,
        }
    }

    pub(super) fn set_routed_peers(&self, peers: Vec<String>) -> Result<(), FipsSyncError> {
        if let Some(client) = &self.client {
            self.update_routes
                .set_application_peers(bounded_routed_peers(
                    peers,
                    client.options().max_connected_peers,
                ))
                .map_err(|error| endpoint_error(error.to_string()))?;
        }
        Ok(())
    }

    pub(super) async fn publish(&self, event: Event) -> Result<usize, FipsSyncError> {
        let event =
            VerifiedEvent::try_from(event).map_err(|error| endpoint_error(error.to_string()))?;
        let Some(client) = self.client.as_ref() else {
            return Err(endpoint_error("Drive Nostr pubsub runtime is closed"));
        };
        let peers = client
            .connected_peer_count()
            .map_err(|error| endpoint_error(error.to_string()))?;
        client
            .publish(event, EventSource::local_index("iris-drive"))
            .await
            .map_err(|error| endpoint_error(error.to_string()))?;
        Ok(peers)
    }

    pub(super) async fn shutdown(&mut self) {
        if let Some(task) = self.receiver_task.take() {
            task.abort();
            let _ = task.await;
        }
        if let Some(client) = self.client.take() {
            client.shutdown_shared().await;
        }
        if let Some(store) = self.announcement_store.take() {
            store.close().await;
        }
    }
}

impl Drop for DriveNostrPubsubRuntime {
    fn drop(&mut self) {
        if let Some(task) = self.receiver_task.take() {
            task.abort();
        }
    }
}

pub(super) async fn recv_optional_nostr_pubsub_event(
    receiver: Option<&tokio::sync::Mutex<broadcast::Receiver<FipsNostrPubsubEvent>>>,
) -> FipsNostrPubsubEvent {
    let Some(receiver) = receiver else {
        return std::future::pending().await;
    };
    receiver
        .lock()
        .await
        .recv()
        .await
        .expect("Nostr pubsub runtime stopped")
}

async fn observe_announcement(
    endpoint: &FipsEndpoint,
    store: Option<&FileEventBus>,
    delivery: &nostr_pubsub::QueryEvent,
) -> nostr_pubsub::Result<()> {
    if nostr_pubsub_fips::ingest_fips_discovery_event(endpoint, delivery.event.clone()).await?
        && let Some(store) = store
    {
        store
            .publish(delivery.event.clone(), delivery.source.clone())
            .await?;
    }
    Ok(())
}

async fn open_announcement_store(
    directory: &Path,
    endpoint: &FipsEndpoint,
) -> Result<Arc<FileEventBus>, FipsSyncError> {
    let scope = endpoint
        .discovery_scope()
        .map(str::trim)
        .filter(|scope| !scope.is_empty())
        .unwrap_or(ADVERT_IDENTIFIER);
    let namespace = format!("{}\0{scope}", endpoint.npub());
    let key = hex::encode(hashtree_core::sha256(namespace.as_bytes()));
    let path = directory
        .join("fips-peer-announcements")
        .join(format!("{key}.json"));
    let filter = Filter::new()
        .kind(nostr_sdk::Kind::Custom(ADVERT_KIND))
        .identifier(scope);
    FileEventBus::open(path, EventRetentionPolicy::new(256, vec![filter]))
        .await
        .map(Arc::new)
        .map_err(|error| endpoint_error(error.to_string()))
}

fn endpoint_error(error: impl Into<String>) -> FipsSyncError {
    FipsSyncError::Endpoint(error.into())
}

fn bounded_routed_peers(peers: Vec<String>, max_connected_peers: usize) -> Vec<String> {
    peers
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .take(max_connected_peers.min(63))
        .collect()
}
