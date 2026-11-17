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
use tokio::sync::{broadcast, watch};
use tokio::task::JoinHandle;

use super::{FipsBlockSync, FipsSyncError};

const DELIVERY_CAPACITY: usize = 256;
const SUBSCRIPTION_RETRY_INTERVAL: Duration = Duration::from_millis(500);
const RECENT_EVENT_IDS: usize = 256;
/// The account's device roster (7368) and Drive's roots, notes and device
/// reports (30078). The account's devices exchange them over the FIPS feed,
/// as the web devices do, so they keep syncing when relays do not answer.
pub const DRIVE_FEED_KINDS: [u16; 2] = [7368, 30078];

#[must_use]
pub fn is_drive_feed_event(event: &Event) -> bool {
    DRIVE_FEED_KINDS.contains(&event.kind.as_u16())
}

fn drive_feed_filter() -> Filter {
    Filter::new().kinds(DRIVE_FEED_KINDS.map(nostr_sdk::Kind::Custom))
}

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
    feed_peers: watch::Sender<Vec<String>>,
    feed_task: Option<JoinHandle<()>>,
}

impl<L: hashtree_core::Store + Send + Sync + 'static> FipsBlockSync<L> {
    pub async fn publish_nostr_event(
        &self,
        event: nostr_sdk::Event,
    ) -> Result<usize, FipsSyncError> {
        let Some(runtime) = self.nostr_runtime.as_ref() else {
            return Ok(0);
        };
        runtime.publish(event).await
    }

    pub async fn drain_nostr_pubsub_events(&self) -> Vec<FipsNostrPubsubEvent> {
        let Some(receiver) = self.nostr_receiver.as_ref() else {
            return Vec::new();
        };
        let mut receiver = receiver.lock().await;
        let mut events = Vec::new();
        while events.len() < 64 {
            match receiver.try_recv() {
                Ok(event) => events.push(event),
                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
        events
    }

    /// Pass this device's Drive events into the FIPS feed, then admit the
    /// account's other devices to it. Negentropy then sends each side only the
    /// events it lacks, and later events follow live.
    pub async fn admit_drive_feed_peers(&self, peers: Vec<String>, events: Vec<Event>) {
        if let Some(runtime) = self.nostr_runtime.as_ref() {
            runtime.admit_drive_feed_peers(peers, events).await;
        }
    }

    pub async fn recv_nostr_pubsub_event(&self) -> FipsNostrPubsubEvent {
        recv_optional_nostr_pubsub_event(self.nostr_receiver.as_ref()).await
    }

    pub fn mesh_peer_limit(&self) -> Option<usize> {
        self.nostr_runtime
            .as_ref()
            .and_then(DriveNostrPubsubRuntime::max_connected_peers)
    }

    pub fn mesh_peer_count(&self) -> usize {
        let Some(runtime) = self.nostr_runtime.as_ref() else {
            return 0;
        };
        runtime.connected_peer_count()
    }
}

impl DriveNostrPubsubRuntime {
    pub(super) async fn bind(
        endpoint: Arc<FipsEndpoint>,
        routed_peers: Vec<String>,
        trusted_raters: Vec<String>,
        device_directory: Option<&Path>,
        filters: Vec<Filter>,
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
                let Ok(mut subscription) = task_client.subscribe(filters.clone()).await else {
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
        // The feed is open before any Drive event is passed into it, so its
        // window holds them when a device is admitted.
        let feed = match client
            .subscribe_feed(drive_feed_filter(), Some(Vec::new()))
            .await
        {
            Ok(feed) => feed,
            Err(error) => {
                receiver_task.abort();
                client.shutdown_shared().await;
                if let Some(store) = &announcement_store {
                    store.close().await;
                }
                return Err(endpoint_error(error.to_string()));
            }
        };
        let (feed_peers, feed_peers_rx) = watch::channel(Vec::new());
        let feed_task = tokio::spawn(run_drive_feed(
            client.clone(),
            feed,
            deliveries.clone(),
            feed_peers_rx,
        ));
        Ok(Self {
            client: Some(client),
            announcement_store,
            update_routes,
            deliveries,
            receiver_task: Some(receiver_task),
            feed_peers,
            feed_task: Some(feed_task),
        })
    }

    #[cfg(any(feature = "stack-fixture", test))]
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

    pub(super) async fn admit_drive_feed_peers(&self, peers: Vec<String>, events: Vec<Event>) {
        for event in events.into_iter().filter(is_drive_feed_event) {
            if let Err(error) = self.publish(event).await {
                tracing::debug!(%error, "passing a Drive event into the FIPS feed");
            }
        }
        let mut peers = peers;
        peers.sort_unstable();
        peers.dedup();
        self.feed_peers.send_if_modified(|current| {
            let changed = *current != peers;
            *current = peers;
            changed
        });
    }

    pub(super) async fn shutdown(&mut self) {
        for task in [self.receiver_task.take(), self.feed_task.take()]
            .into_iter()
            .flatten()
        {
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
        for task in [self.receiver_task.take(), self.feed_task.take()]
            .into_iter()
            .flatten()
        {
            task.abort();
        }
    }
}

/// The Drive feed: only the account's own devices are its peers. Events they
/// send are delivered like any pubsub event, for the daemon to apply as it
/// applies relay events.
async fn run_drive_feed(
    client: Arc<FipsPubsubClient>,
    mut feed: nostr_pubsub_fips::FipsPubsubSubscription,
    deliveries: broadcast::Sender<FipsNostrPubsubEvent>,
    mut peers: watch::Receiver<Vec<String>>,
) {
    loop {
        loop {
            tokio::select! {
                delivery = feed.recv() => {
                    let Some(delivery) = delivery else {
                        break;
                    };
                    let _ = deliveries.send(FipsNostrPubsubEvent {
                        origin_peer_id: delivery.source.id.0,
                        event: delivery.event.into_event(),
                    });
                }
                changed = peers.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    let admitted = peers.borrow_and_update().clone();
                    if let Err(error) = feed.set_feed_peers(Some(admitted)).await {
                        tracing::warn!(%error, "admitting devices to the Drive FIPS feed");
                    }
                }
            }
        }
        // A closed feed reopens with an empty window; the next announcement
        // passes this device's events into it again.
        loop {
            tokio::time::sleep(SUBSCRIPTION_RETRY_INTERVAL).await;
            let admitted = peers.borrow_and_update().clone();
            if let Ok(reopened) = client
                .subscribe_feed(drive_feed_filter(), Some(admitted))
                .await
            {
                feed = reopened;
                break;
            }
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
    #[cfg(target_os = "ios")]
    let path = tokio::task::spawn_blocking(move || {
        super::ios_announcement_cache::snapshot_path(&path, &std::env::temp_dir())
    })
    .await
    .map_err(|e| endpoint_error(e.to_string()))?
    .map_err(|e| endpoint_error(e.to_string()))?;
    let filter = peer_announcement_filter(scope);
    FileEventBus::open(path, EventRetentionPolicy::new(256, vec![filter]))
        .await
        .map(Arc::new)
        .map_err(|error| endpoint_error(error.to_string()))
}

fn peer_announcement_filter(scope: &str) -> Filter {
    Filter::new()
        .kind(nostr_sdk::Kind::Custom(ADVERT_KIND))
        .identifier(scope)
}

pub(super) fn drive_subscription_filters(
    scope: &str,
    reference: &hashtree_updater::UpdateRef,
) -> Result<Vec<Filter>, FipsSyncError> {
    let updates = hashtree_updater::UpdateEventCache::new(reference)
        .map_err(|error| endpoint_error(error.to_string()))?;
    Ok(vec![
        peer_announcement_filter(scope),
        updates.filter().clone(),
    ])
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
