//! Verified Nostr events over the shared FIPS pubsub adapter.

use std::collections::{HashSet, VecDeque};
use std::sync::Arc;
use std::time::Duration;

use fips_core::FipsEndpoint;
use nostr_pubsub::{EventBus, EventSource, Filter, VerifiedEvent};
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
    deliveries: broadcast::Sender<FipsNostrPubsubEvent>,
    receiver_task: Option<JoinHandle<()>>,
}

impl DriveNostrPubsubRuntime {
    pub(super) async fn bind(
        endpoint: Arc<FipsEndpoint>,
        routed_peers: Vec<String>,
        trusted_raters: Vec<String>,
    ) -> Result<Self, FipsSyncError> {
        #[cfg(feature = "stack-fixture")]
        let max_connected_peers = std::env::var("IRIS_STACK_FIXTURE_PUBSUB_MAX_PEERS")
            .map_or(64, |value| value.parse().unwrap_or(0));
        #[cfg(not(feature = "stack-fixture"))]
        let max_connected_peers = 64;
        let mut reputation = FipsPubsubPolicyOptions::default();
        reputation.reputation.trusted_raters = trusted_raters.into_iter().collect();
        let client = Arc::new(
            FipsPubsubClient::start_with_reputation(
                endpoint,
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
        let (deliveries, _) = broadcast::channel(DELIVERY_CAPACITY);
        let task_deliveries = deliveries.clone();
        let task_client = client.clone();
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
            deliveries,
            receiver_task: Some(receiver_task),
        })
    }

    pub(super) fn subscribe(&self) -> broadcast::Receiver<FipsNostrPubsubEvent> {
        self.deliveries.subscribe()
    }

    pub(super) fn register_updater(&self, config_dir: &std::path::Path) {
        if let Some(client) = &self.client {
            crate::updater::pubsub::register(config_dir, client);
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
            client
                .set_routed_peers(bounded_routed_peers(
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
    }
}

impl Drop for DriveNostrPubsubRuntime {
    fn drop(&mut self) {
        if let Some(task) = self.receiver_task.take() {
            task.abort();
        }
    }
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
