//! Direct block replication over hashtree's FIPS transport.
//!
//! Nostr relay events carry Iris Drive metadata: the `AppKey` roster and
//! per-AppKey root CIDs. This module moves the actual hashtree blocks directly
//! between authorized app installs. Blossom remains useful as a public remote,
//! but the local app should first ask peer instances over FIPS.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use hashtree_core::{
    Cid, Hash, HashTree, HashTreeConfig, HashTreeError, Store, StoreError, to_hex,
};
use hashtree_fips_transport::{
    FipsAppMessage, FipsEndpointOptions, FipsMeshPubsub, FipsMeshPubsubEvent, FipsPeerConfig,
    FipsPeerStatus, FipsRelayStatus, HashtreeFipsTransport, PubsubPublishStats, bind_fips_endpoint,
};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::ToBech32;
use thiserror::Error;
use tokio::task::JoinHandle;

use crate::app_key_link_transport::{
    APP_KEY_LINK_REQUEST_APP_TOPIC, APP_KEY_LINK_ROSTER_APP_TOPIC,
};
use crate::block_sync::collect_live_sync_hashes;
use crate::blossom_sync::DownloadReport;
use crate::config::AppConfig;
use crate::direct_root_transport::DIRECT_ROOT_APP_TOPIC;
use crate::fips_bootstrap::DEFAULT_FIPS_BOOTSTRAP_PEERS;
use crate::identity::AppKey;

const FIPS_REQUEST_TIMEOUT: Duration = Duration::from_millis(1_250);
const FIPS_REQUEST_RETRY_INTERVAL: Duration = Duration::from_millis(250);
const FIPS_REQUEST_MAX_ATTEMPTS: usize = 4;
const FIPS_PACKET_CHANNEL_CAPACITY: usize = 8192;
const FIPS_WEBRTC_MAX_CONNECTIONS: usize = 16;
const FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING: usize = 0;
const APP_KEY_LINK_OPEN_DISCOVERY_MAX_PENDING: usize = 16;
pub const IRIS_DRIVE_FIPS_DISCOVERY_SCOPE: &str = "fips-overlay-v1";

#[derive(Debug, Error)]
pub enum FipsSyncError {
    #[error("fips endpoint: {0}")]
    Endpoint(String),
    #[error("tree walk: {0}")]
    Tree(#[from] HashTreeError),
    #[error("local store: {0}")]
    LocalStore(String),
    #[error("missing block on fips peers: {0}")]
    MissingOnFips(String),
    #[error("identity: {0}")]
    Identity(String),
}

/// Running FIPS block exchange bound to the Iris Drive block store.
pub struct FipsBlockSync<L: Store + Send + Sync + 'static> {
    transport: Arc<HashtreeFipsTransport<L>>,
    local_store: Arc<L>,
    receiver_task: Option<JoinHandle<()>>,
    mesh_pubsub: Option<Arc<FipsMeshPubsub<L>>>,
    endpoint_npub: String,
    discovery_scope: String,
    transport_settings: FipsTransportSettings,
    last_peer_config: Mutex<Option<FipsPeerConfigSnapshot>>,
}

pub type FsFipsBlockSync = FipsBlockSync<hashtree_fs::FsBlobStore>;

impl<L: Store + Send + Sync + 'static> FipsBlockSync<L> {
    pub async fn start(
        device: &AppKey,
        local_store: Arc<L>,
        config: &AppConfig,
    ) -> Result<Self, FipsSyncError> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let identity_nsec = device
            .keys()
            .secret_key()
            .to_bech32()
            .map_err(|error| FipsSyncError::Identity(error.to_string()))?;
        let discovery_scope = discovery_scope(config);
        let mut transport_settings = FipsTransportSettings::from_env();
        if accepts_app_key_link_requests(config) {
            transport_settings.open_discovery_max_pending = transport_settings
                .open_discovery_max_pending
                .max(APP_KEY_LINK_OPEN_DISCOVERY_MAX_PENDING);
        }
        let endpoint = Box::pin(bind_fips_endpoint(fips_endpoint_options(
            identity_nsec,
            discovery_scope.clone(),
            config.relays.clone(),
            config,
            &transport_settings,
        )))
        .await
        .map_err(|error| FipsSyncError::Endpoint(error.to_string()))?;

        let transport = Arc::new(
            HashtreeFipsTransport::new(endpoint.endpoint, local_store.clone())
                .with_request_timeout(FIPS_REQUEST_TIMEOUT)
                .with_request_retry_interval(FIPS_REQUEST_RETRY_INTERVAL)
                .with_request_max_attempts(FIPS_REQUEST_MAX_ATTEMPTS)
                .with_unconfigured_app_message_topics(unconfigured_app_message_topics()),
        );
        transport
            .set_peer_configs_with_routing_peers(
                authorized_device_fips_peers(config, &transport_settings),
                routing_fips_peers(config, &transport_settings),
            )
            .await;
        let receiver_task = transport.start();
        let mesh_pubsub = if transport_settings.enable_mesh_pubsub {
            Some(Arc::new(
                transport
                    .start_mesh_pubsub(
                        local_store.clone(),
                        endpoint.local_peer_id.clone(),
                        FIPS_REQUEST_TIMEOUT,
                    )
                    .await
                    .map_err(|error| FipsSyncError::Endpoint(error.to_string()))?,
            ))
        } else {
            None
        };

        Ok(Self {
            transport,
            local_store,
            receiver_task: Some(receiver_task),
            mesh_pubsub,
            endpoint_npub: endpoint.local_peer_id,
            discovery_scope,
            transport_settings,
            last_peer_config: Mutex::new(None),
        })
    }

    #[must_use]
    pub fn endpoint_npub(&self) -> &str {
        &self.endpoint_npub
    }

    #[must_use]
    pub fn discovery_scope(&self) -> &str {
        &self.discovery_scope
    }

    #[must_use]
    pub fn nostr_discovery_app(&self) -> &str {
        self.discovery_scope()
    }

    #[must_use]
    pub fn transport_settings(&self) -> &FipsTransportSettings {
        &self.transport_settings
    }

    pub async fn refresh_authorized_peers(&self, config: &AppConfig) {
        let mut application_peers = authorized_device_fips_peers(config, &self.transport_settings);
        let routing_peers = routing_fips_peers(config, &self.transport_settings);
        if accepts_app_key_link_requests(config) {
            add_connected_app_key_link_application_peers(
                &mut application_peers,
                &routing_peers,
                self.endpoint_npub.as_str(),
                self.transport.connected_peer_ids().await,
            );
        }
        let snapshot = fips_peer_config_snapshot(
            Some(self.endpoint_npub.as_str()),
            &application_peers,
            &routing_peers,
        );
        if !self.update_peer_config_snapshot(snapshot) {
            return;
        }
        self.transport
            .set_peer_configs_with_routing_peers(application_peers, routing_peers)
            .await;
    }

    pub async fn peer_ids(&self) -> Vec<String> {
        self.transport.peer_ids().await
    }

    pub async fn authorized_peer_ids(&self) -> Vec<String> {
        self.transport.configured_peer_ids().await
    }

    pub async fn connected_peer_ids(&self) -> Vec<String> {
        self.transport.connected_peer_ids().await
    }

    pub async fn fips_peer_statuses(&self) -> Vec<FipsPeerStatus> {
        self.transport.peer_statuses().await
    }

    pub async fn fips_relay_statuses(&self) -> Vec<FipsRelayStatus> {
        self.transport.relay_statuses().await
    }

    #[must_use]
    pub fn subscribe_app_messages(&self) -> tokio::sync::broadcast::Receiver<FipsAppMessage> {
        self.transport.subscribe_app_messages()
    }

    #[must_use]
    pub fn mesh_pubsub_enabled(&self) -> bool {
        self.mesh_pubsub.is_some()
    }

    pub async fn send_app_message(
        &self,
        peer_id: &str,
        topic: &str,
        data: Vec<u8>,
    ) -> Result<(), FipsSyncError> {
        self.transport
            .send_app_message(peer_id, topic, data)
            .await
            .map_err(|error| FipsSyncError::Endpoint(error.to_string()))
    }

    pub async fn broadcast_app_message(
        &self,
        topic: &str,
        data: Vec<u8>,
    ) -> Result<usize, FipsSyncError> {
        self.transport
            .broadcast_app_message(topic, data)
            .await
            .map_err(|error| FipsSyncError::Endpoint(error.to_string()))
    }

    pub async fn subscribe_mesh_pubsub(&self, stream_id: String) -> PubsubPublishStats {
        let Some(mesh_pubsub) = self.mesh_pubsub.as_ref() else {
            let _ = stream_id;
            return PubsubPublishStats::default();
        };
        mesh_pubsub.subscribe_pubsub(stream_id).await
    }

    pub async fn publish_mesh_pubsub(
        &self,
        stream_id: String,
        seq: u64,
        payload: Vec<u8>,
    ) -> PubsubPublishStats {
        let Some(mesh_pubsub) = self.mesh_pubsub.as_ref() else {
            let _ = (stream_id, seq, payload);
            return PubsubPublishStats::default();
        };
        mesh_pubsub.publish_pubsub(stream_id, seq, payload).await
    }

    pub async fn drain_mesh_pubsub_events(&self) -> Vec<FipsMeshPubsubEvent> {
        let Some(mesh_pubsub) = self.mesh_pubsub.as_ref() else {
            return Vec::new();
        };
        mesh_pubsub.drain_pubsub_events().await
    }

    pub async fn recv_mesh_pubsub_event(&self) -> FipsMeshPubsubEvent {
        self.mesh_pubsub
            .as_ref()
            .expect("mesh pubsub is disabled")
            .recv_pubsub_event()
            .await
    }

    pub async fn mesh_peer_count(&self) -> usize {
        let Some(mesh_pubsub) = self.mesh_pubsub.as_ref() else {
            return 0;
        };
        mesh_pubsub.peer_count().await
    }

    pub async fn mesh_peer_ids(&self) -> Vec<String> {
        let Some(mesh_pubsub) = self.mesh_pubsub.as_ref() else {
            return Vec::new();
        };
        mesh_pubsub.peer_ids().await
    }
    pub async fn download_tree(&self, root: &Cid) -> Result<DownloadReport, FipsSyncError> {
        let direct =
            download_tree_with_transport(self.local_store.clone(), root, self.transport.clone())
                .await;
        let Some(mesh_pubsub) = self.mesh_pubsub.as_ref() else {
            return direct;
        };
        match direct {
            Ok(report) => Ok(report),
            Err(FipsSyncError::MissingOnFips(_)) => {
                download_tree_with_mesh(self.local_store.clone(), root, mesh_pubsub.clone()).await
            }
            Err(error) => Err(error),
        }
    }

    pub async fn shutdown(mut self) -> Result<(), FipsSyncError> {
        let shutdown_result = self.shutdown_endpoint().await;
        if let Some(receiver_task) = self.receiver_task.take() {
            receiver_task.abort();
            let _ = receiver_task.await;
        }
        shutdown_result
    }

    pub async fn shutdown_endpoint(&self) -> Result<(), FipsSyncError> {
        self.transport
            .shutdown()
            .await
            .map_err(|error| FipsSyncError::Endpoint(error.to_string()))
    }

    fn update_peer_config_snapshot(&self, snapshot: FipsPeerConfigSnapshot) -> bool {
        let mut last = self
            .last_peer_config
            .lock()
            .expect("FIPS peer config snapshot lock poisoned");
        if last.as_ref() == Some(&snapshot) {
            return false;
        }
        *last = Some(snapshot);
        true
    }
}

fn unconfigured_app_message_topics() -> [&'static str; 3] {
    [
        APP_KEY_LINK_REQUEST_APP_TOPIC,
        APP_KEY_LINK_ROSTER_APP_TOPIC,
        DIRECT_ROOT_APP_TOPIC,
    ]
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct FipsPeerConfigSnapshot {
    application_peers: Vec<FipsPeerConfig>,
    routing_peers: Vec<FipsPeerConfig>,
}

fn fips_peer_config_snapshot(
    local: Option<&str>,
    application_peers: &[FipsPeerConfig],
    routing_peers: &[FipsPeerConfig],
) -> FipsPeerConfigSnapshot {
    let mut seen = std::collections::HashSet::new();
    FipsPeerConfigSnapshot {
        application_peers: normalize_fips_peer_configs(local, application_peers, &mut seen),
        routing_peers: normalize_fips_peer_configs(local, routing_peers, &mut seen),
    }
}

fn add_connected_app_key_link_application_peers(
    application_peers: &mut Vec<FipsPeerConfig>,
    routing_peers: &[FipsPeerConfig],
    local_npub: &str,
    connected_peer_ids: impl IntoIterator<Item = String>,
) {
    for npub in connected_peer_ids {
        if npub != local_npub
            && !application_peers.iter().any(|peer| peer.npub == npub)
            && !routing_peers.iter().any(|peer| peer.npub == npub)
        {
            application_peers.push(FipsPeerConfig::new(npub));
        }
    }
}

fn normalize_fips_peer_configs(
    local: Option<&str>,
    peers: &[FipsPeerConfig],
    seen: &mut std::collections::HashSet<String>,
) -> Vec<FipsPeerConfig> {
    let mut out = Vec::new();
    for peer in peers {
        let npub = peer.npub.trim().to_string();
        if npub.is_empty() || Some(npub.as_str()) == local || !seen.insert(npub.clone()) {
            continue;
        }
        let udp_addresses = peer
            .udp_addresses
            .iter()
            .map(|addr| addr.trim().to_string())
            .filter(|addr| !addr.is_empty())
            .collect();
        out.push(FipsPeerConfig {
            npub,
            udp_addresses,
        });
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FipsTransportSettings {
    pub enable_udp: bool,
    pub enable_webrtc: bool,
    pub enable_lan_discovery: bool,
    pub enable_mesh_pubsub: bool,
    pub udp_bind_addr: Option<String>,
    pub udp_public: bool,
    pub udp_external_addr: Option<String>,
    pub share_local_candidates: bool,
    pub static_peer_hints: Vec<(String, Vec<String>)>,
    pub bootstrap_peer_hints: Vec<(String, Vec<String>)>,
    pub webrtc_max_connections: usize,
    pub open_discovery_max_pending: usize,
}

impl Default for FipsTransportSettings {
    fn default() -> Self {
        Self {
            enable_udp: true,
            enable_webrtc: target_allows_default_fips_webrtc(std::env::consts::OS),
            enable_lan_discovery: true,
            enable_mesh_pubsub: true,
            udp_bind_addr: None,
            udp_public: false,
            udp_external_addr: None,
            share_local_candidates: target_allows_default_local_candidate_sharing(
                std::env::consts::OS,
            ),
            static_peer_hints: Vec::new(),
            bootstrap_peer_hints: default_fips_bootstrap_peer_hints(),
            webrtc_max_connections: FIPS_WEBRTC_MAX_CONNECTIONS,
            open_discovery_max_pending: FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING,
        }
    }
}

impl FipsTransportSettings {
    #[must_use]
    pub fn from_env() -> Self {
        let defaults = Self::default();
        let udp_bind_addr = non_empty_env("IRIS_DRIVE_FIPS_UDP_BIND_ADDR");
        let udp_external_addr = non_empty_env("IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR");
        let udp_public =
            bool_env("IRIS_DRIVE_FIPS_UDP_PUBLIC").unwrap_or_else(|| udp_external_addr.is_some());
        let bootstrap_enabled = bool_env("IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP").unwrap_or(true);
        let bootstrap_peer_hints = if !bootstrap_enabled {
            Vec::new()
        } else if let Ok(value) = std::env::var("IRIS_DRIVE_FIPS_BOOTSTRAP_PEERS") {
            parse_static_peer_hints(&value)
        } else {
            default_fips_bootstrap_peer_hints()
        };
        Self {
            enable_udp: bool_env("IRIS_DRIVE_FIPS_ENABLE_UDP").unwrap_or(defaults.enable_udp),
            enable_webrtc: bool_env("IRIS_DRIVE_FIPS_ENABLE_WEBRTC")
                .unwrap_or(defaults.enable_webrtc),
            enable_lan_discovery: bool_env("IRIS_DRIVE_FIPS_ENABLE_LAN_DISCOVERY")
                .unwrap_or(defaults.enable_lan_discovery),
            enable_mesh_pubsub: bool_env("IRIS_DRIVE_FIPS_ENABLE_MESH_PUBSUB")
                .unwrap_or(defaults.enable_mesh_pubsub),
            udp_bind_addr,
            udp_public,
            udp_external_addr,
            share_local_candidates: bool_env("IRIS_DRIVE_FIPS_SHARE_LOCAL_CANDIDATES")
                .unwrap_or(defaults.share_local_candidates),
            static_peer_hints: parse_static_peer_hints(
                &std::env::var("IRIS_DRIVE_FIPS_STATIC_PEERS").unwrap_or_default(),
            ),
            bootstrap_peer_hints,
            webrtc_max_connections: usize_env("IRIS_DRIVE_FIPS_WEBRTC_MAX_CONNECTIONS")
                .unwrap_or(FIPS_WEBRTC_MAX_CONNECTIONS)
                .max(1),
            open_discovery_max_pending: usize_env("IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING")
                .unwrap_or(FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING),
        }
    }
}

fn target_allows_default_local_candidate_sharing(target_os: &str) -> bool {
    !matches!(target_os, "android" | "ios")
}

fn target_allows_default_fips_webrtc(target_os: &str) -> bool {
    !matches!(target_os, "android" | "ios")
}

fn fips_endpoint_options(
    identity_nsec: String,
    discovery_scope: String,
    relays: Vec<String>,
    config: &AppConfig,
    settings: &FipsTransportSettings,
) -> FipsEndpointOptions {
    let mut open_discovery_max_pending = settings.open_discovery_max_pending;
    if accepts_app_key_link_requests(config) {
        open_discovery_max_pending =
            open_discovery_max_pending.max(APP_KEY_LINK_OPEN_DISCOVERY_MAX_PENDING);
    }
    FipsEndpointOptions {
        identity_nsec,
        discovery_scope,
        relays,
        enable_udp: settings.enable_udp,
        enable_webrtc: settings.enable_webrtc,
        enable_lan_discovery: settings.enable_lan_discovery,
        udp_bind_addr: settings.udp_bind_addr.clone(),
        udp_public: settings.udp_public,
        udp_external_addr: settings.udp_external_addr.clone(),
        share_local_candidates: settings.share_local_candidates,
        webrtc_auto_connect: true,
        webrtc_max_connections: settings.webrtc_max_connections,
        open_discovery_max_pending,
        packet_channel_capacity: FIPS_PACKET_CHANNEL_CAPACITY,
    }
}

fn accepts_app_key_link_requests(config: &AppConfig) -> bool {
    config
        .profile
        .as_ref()
        .is_some_and(crate::ProfileState::can_admin_profile)
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn bool_env(name: &str) -> Option<bool> {
    parse_bool_env_value(std::env::var(name).ok()?.trim())
}

fn usize_env(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.trim().parse().ok()
}

fn parse_bool_env_value(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

impl<L: Store + Send + Sync + 'static> Drop for FipsBlockSync<L> {
    fn drop(&mut self) {
        if let Some(receiver_task) = self.receiver_task.take() {
            receiver_task.abort();
        }
    }
}

pub async fn download_tree_with_transport<L>(
    local_store: Arc<L>,
    root: &Cid,
    transport: Arc<HashtreeFipsTransport<L>>,
) -> Result<DownloadReport, FipsSyncError>
where
    L: Store + Send + Sync + 'static,
{
    let writeback = Arc::new(WriteBackFipsStore::new(local_store, transport));
    let tree = HashTree::new(HashTreeConfig::new(writeback.clone()));
    let hashes = collect_live_sync_hashes(&tree, root, 4).await?;
    if writeback.missing() > 0 {
        let detail = writeback
            .first_missing()
            .unwrap_or_else(|| format!("{} blocks", writeback.missing()));
        return Err(FipsSyncError::MissingOnFips(detail));
    }

    Ok(DownloadReport {
        total_hashes: hashes.len(),
        fetched: writeback.fetched(),
        already_local: writeback.already_local(),
    })
}

pub async fn download_tree_with_mesh<L>(
    local_store: Arc<L>,
    root: &Cid,
    mesh: Arc<FipsMeshPubsub<L>>,
) -> Result<DownloadReport, FipsSyncError>
where
    L: Store + Send + Sync + 'static,
{
    let writeback = Arc::new(WriteBackMeshStore::new(local_store, mesh));
    let tree = HashTree::new(HashTreeConfig::new(writeback.clone()));
    let hashes = collect_live_sync_hashes(&tree, root, 4).await?;
    if writeback.missing() > 0 {
        let detail = writeback
            .first_missing()
            .unwrap_or_else(|| format!("{} blocks", writeback.missing()));
        return Err(FipsSyncError::MissingOnFips(detail));
    }

    Ok(DownloadReport {
        total_hashes: hashes.len(),
        fetched: writeback.fetched(),
        already_local: writeback.already_local(),
    })
}

pub async fn download_tree_with_overlay<L>(
    local_store: Arc<L>,
    root: &Cid,
    transport: Arc<HashtreeFipsTransport<L>>,
    mesh: Arc<FipsMeshPubsub<L>>,
) -> Result<DownloadReport, FipsSyncError>
where
    L: Store + Send + Sync + 'static,
{
    let writeback = Arc::new(WriteBackOverlayStore::new(local_store, transport, mesh));
    let tree = HashTree::new(HashTreeConfig::new(writeback.clone()));
    let hashes = collect_live_sync_hashes(&tree, root, 4).await?;
    if writeback.missing() > 0 {
        let detail = writeback
            .first_missing()
            .unwrap_or_else(|| format!("{} blocks", writeback.missing()));
        return Err(FipsSyncError::MissingOnFips(detail));
    }

    Ok(DownloadReport {
        total_hashes: hashes.len(),
        fetched: writeback.fetched(),
        already_local: writeback.already_local(),
    })
}

#[must_use]
pub fn discovery_scope(config: &AppConfig) -> String {
    let _ = config;
    IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string()
}

fn authorized_device_fips_peers(
    config: &AppConfig,
    settings: &FipsTransportSettings,
) -> Vec<FipsPeerConfig> {
    let Some(account) = config.profile.as_ref() else {
        return Vec::new();
    };
    let mut peers = Vec::new();
    let local_device = &account.app_key_pubkey;
    let devices = account.current_app_keys_projection().map_or_else(
        || legacy_drive_root_app_actors(config, account),
        |projection| projection.app_actors,
    );
    peers.extend(
        devices
            .iter()
            .filter(|device| device.pubkey != *local_device)
            .filter_map(|device| {
                PublicKey::from_hex(&device.pubkey)
                    .ok()
                    .and_then(|pubkey| pubkey.to_bech32().ok())
                    .map(|npub| FipsPeerConfig {
                        udp_addresses: static_peer_addresses_for_device(
                            &settings.static_peer_hints,
                            device,
                            &npub,
                        ),
                        npub,
                    })
            }),
    );
    if let Some(pending) = pending_app_key_link_fips_peer(config, settings)
        && !peers.iter().any(|peer| peer.npub == pending.npub)
    {
        peers.push(pending);
    }
    if account.can_admin_profile() {
        for request in &account.inbound_app_key_link_requests {
            if let Some(pending) = fips_peer_config_for_pubkey(&request.app_key_pubkey, settings)
                && !peers.iter().any(|peer| peer.npub == pending.npub)
            {
                peers.push(pending);
            }
        }
    }
    peers
}

fn legacy_drive_root_app_actors(
    config: &AppConfig,
    account: &crate::ProfileState,
) -> Vec<crate::AppActorEntry> {
    let Some(drive) = config.drive(crate::PRIMARY_DRIVE_ID) else {
        return Vec::new();
    };
    crate::drive_root_writer_app_key_pubkeys(account, drive)
        .into_iter()
        .map(|pubkey| {
            if pubkey == account.app_key_pubkey {
                crate::AppActorEntry::admin(pubkey, 0, account.app_key_label.clone())
            } else {
                crate::AppActorEntry::member(pubkey, 0, None)
            }
        })
        .collect()
}

fn routing_fips_peers(config: &AppConfig, settings: &FipsTransportSettings) -> Vec<FipsPeerConfig> {
    let pending = pending_app_key_link_fips_peers(config, settings);
    let mut peers = if should_use_bootstrap_fips_peers(config, !pending.is_empty()) {
        bootstrap_fips_peers(settings)
    } else {
        Vec::new()
    };
    peers.extend(pending);
    peers
}

fn should_use_bootstrap_fips_peers(config: &AppConfig, has_pending_link_peer: bool) -> bool {
    has_remote_authorized_app_key(config)
        || has_pending_link_peer
        || config
            .profile
            .as_ref()
            .is_some_and(|account| !account.inbound_app_key_link_requests.is_empty())
}

fn has_remote_authorized_app_key(config: &AppConfig) -> bool {
    let Some(account) = config.profile.as_ref() else {
        return false;
    };
    account.current_app_keys_projection().map_or_else(
        || {
            legacy_drive_root_app_actors(config, account)
                .iter()
                .any(|device| device.pubkey != account.app_key_pubkey)
        },
        |app_keys| {
            app_keys
                .app_actors
                .iter()
                .any(|device| device.pubkey != account.app_key_pubkey)
        },
    )
}

fn pending_app_key_link_fips_peers(
    config: &AppConfig,
    settings: &FipsTransportSettings,
) -> Vec<FipsPeerConfig> {
    pending_app_key_link_fips_peer(config, settings)
        .into_iter()
        .collect()
}

fn pending_app_key_link_fips_peer(
    config: &AppConfig,
    settings: &FipsTransportSettings,
) -> Option<FipsPeerConfig> {
    let account = config.profile.as_ref()?;
    if account.can_admin_profile()
        || account.authorization_state != crate::AppKeyAuthorizationState::AwaitingApproval
    {
        return None;
    }
    let request = account.outbound_app_key_link_request.as_ref()?;
    fips_peer_config_for_pubkey(&request.admin_app_key_pubkey, settings)
}

fn fips_peer_config_for_pubkey(
    pubkey_hex: &str,
    settings: &FipsTransportSettings,
) -> Option<FipsPeerConfig> {
    PublicKey::from_hex(pubkey_hex)
        .ok()
        .and_then(|pubkey| pubkey.to_bech32().ok())
        .map(|npub| FipsPeerConfig {
            udp_addresses: static_peer_addresses_for_keys(
                &settings.static_peer_hints,
                &[pubkey_hex, &npub],
            ),
            npub,
        })
}

fn bootstrap_fips_peers(settings: &FipsTransportSettings) -> Vec<FipsPeerConfig> {
    settings
        .bootstrap_peer_hints
        .iter()
        .filter_map(|(npub, addresses)| {
            let npub = normalize_fips_peer_npub(npub)?;
            let udp_addresses = addresses
                .iter()
                .map(|address| address.trim().to_string())
                .filter(|address| !address.is_empty())
                .collect::<Vec<_>>();
            (!udp_addresses.is_empty()).then_some(FipsPeerConfig {
                npub,
                udp_addresses,
            })
        })
        .collect()
}

fn default_fips_bootstrap_peer_hints() -> Vec<(String, Vec<String>)> {
    DEFAULT_FIPS_BOOTSTRAP_PEERS
        .iter()
        .map(|(npub, addresses)| {
            (
                (*npub).to_string(),
                addresses
                    .iter()
                    .map(|address| (*address).to_string())
                    .collect(),
            )
        })
        .collect()
}

fn normalize_fips_peer_npub(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    PublicKey::from_hex(trimmed)
        .ok()
        .and_then(|pubkey| pubkey.to_bech32().ok())
        .or_else(|| Some(trimmed.to_string()))
}

fn static_peer_addresses_for_device(
    hints: &[(String, Vec<String>)],
    device: &crate::app_keys::AppActorEntry,
    npub: &str,
) -> Vec<String> {
    let mut keys = vec![device.pubkey.as_str(), npub];
    if let Some(label) = device.label.as_deref() {
        keys.push(label);
    }
    static_peer_addresses_for_keys(hints, &keys)
}

fn static_peer_addresses_for_keys(hints: &[(String, Vec<String>)], keys: &[&str]) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (key, addresses) in hints {
        if !static_peer_key_matches_any(key, keys) {
            continue;
        }
        for address in addresses {
            let address = address.trim().to_string();
            if !address.is_empty() && seen.insert(address.clone()) {
                out.push(address);
            }
        }
    }
    out
}

fn static_peer_key_matches_any(key: &str, values: &[&str]) -> bool {
    let key = key.trim();
    if key.is_empty() {
        return false;
    }
    values
        .iter()
        .any(|value| key.eq_ignore_ascii_case(value.trim()))
}

fn parse_static_peer_hints(value: &str) -> Vec<(String, Vec<String>)> {
    value
        .split([',', ';'])
        .filter_map(|entry| {
            let (key, addresses) = entry.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            let addresses = addresses
                .split('|')
                .map(str::trim)
                .filter(|address| !address.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            (!addresses.is_empty()).then(|| (key.to_string(), addresses))
        })
        .collect()
}

struct WriteBackFipsStore<L: Store + Send + Sync + 'static> {
    local: Arc<L>,
    transport: Arc<HashtreeFipsTransport<L>>,
    fetched: std::sync::atomic::AtomicUsize,
    already_local: std::sync::atomic::AtomicUsize,
    missing: std::sync::atomic::AtomicUsize,
    first_missing: Mutex<Option<String>>,
}

impl<L: Store + Send + Sync + 'static> WriteBackFipsStore<L> {
    fn new(local: Arc<L>, transport: Arc<HashtreeFipsTransport<L>>) -> Self {
        Self {
            local,
            transport,
            fetched: std::sync::atomic::AtomicUsize::new(0),
            already_local: std::sync::atomic::AtomicUsize::new(0),
            missing: std::sync::atomic::AtomicUsize::new(0),
            first_missing: Mutex::new(None),
        }
    }

    fn fetched(&self) -> usize {
        self.fetched.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn already_local(&self) -> usize {
        self.already_local
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn missing(&self) -> usize {
        self.missing.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn first_missing(&self) -> Option<String> {
        self.first_missing
            .lock()
            .ok()
            .and_then(|value| value.clone())
    }
}

#[async_trait]
impl<L: Store + Send + Sync + 'static> Store for WriteBackFipsStore<L> {
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
        self.local.put(hash, data).await
    }

    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        if let Some(bytes) = self.local.get(hash).await? {
            self.already_local
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(Some(bytes));
        }

        if let Some(bytes) = self.transport.get(hash).await? {
            self.fetched
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(Some(bytes))
        } else {
            let hex = to_hex(hash);
            if self
                .missing
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                == 0
                && let Ok(mut first_missing) = self.first_missing.lock()
            {
                *first_missing = Some(hex);
            }
            Ok(None)
        }
    }

    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        if self.local.has(hash).await? {
            return Ok(true);
        }
        self.transport.has(hash).await
    }

    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.local.delete(hash).await
    }
}

struct WriteBackMeshStore<L: Store + Send + Sync + 'static> {
    local: Arc<L>,
    mesh: Arc<FipsMeshPubsub<L>>,
    fetched: std::sync::atomic::AtomicUsize,
    already_local: std::sync::atomic::AtomicUsize,
    missing: std::sync::atomic::AtomicUsize,
    first_missing: Mutex<Option<String>>,
}

impl<L: Store + Send + Sync + 'static> WriteBackMeshStore<L> {
    fn new(local: Arc<L>, mesh: Arc<FipsMeshPubsub<L>>) -> Self {
        Self {
            local,
            mesh,
            fetched: std::sync::atomic::AtomicUsize::new(0),
            already_local: std::sync::atomic::AtomicUsize::new(0),
            missing: std::sync::atomic::AtomicUsize::new(0),
            first_missing: Mutex::new(None),
        }
    }

    fn fetched(&self) -> usize {
        self.fetched.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn already_local(&self) -> usize {
        self.already_local
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn missing(&self) -> usize {
        self.missing.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn first_missing(&self) -> Option<String> {
        self.first_missing
            .lock()
            .ok()
            .and_then(|value| value.clone())
    }
}

#[async_trait]
impl<L: Store + Send + Sync + 'static> Store for WriteBackMeshStore<L> {
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
        self.local.put(hash, data).await
    }

    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        if let Some(bytes) = self.local.get(hash).await? {
            self.already_local
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(Some(bytes));
        }

        if let Some(bytes) = self.mesh.get(hash).await? {
            self.fetched
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(Some(bytes))
        } else {
            let hex = to_hex(hash);
            if self
                .missing
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                == 0
                && let Ok(mut first_missing) = self.first_missing.lock()
            {
                *first_missing = Some(hex);
            }
            Ok(None)
        }
    }

    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        if self.local.has(hash).await? {
            return Ok(true);
        }
        self.mesh.has(hash).await
    }

    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.local.delete(hash).await
    }
}

struct WriteBackOverlayStore<L: Store + Send + Sync + 'static> {
    local: Arc<L>,
    transport: Arc<HashtreeFipsTransport<L>>,
    mesh: Arc<FipsMeshPubsub<L>>,
    fetched: std::sync::atomic::AtomicUsize,
    already_local: std::sync::atomic::AtomicUsize,
    missing: std::sync::atomic::AtomicUsize,
    first_missing: Mutex<Option<String>>,
}

impl<L: Store + Send + Sync + 'static> WriteBackOverlayStore<L> {
    fn new(
        local: Arc<L>,
        transport: Arc<HashtreeFipsTransport<L>>,
        mesh: Arc<FipsMeshPubsub<L>>,
    ) -> Self {
        Self {
            local,
            transport,
            mesh,
            fetched: std::sync::atomic::AtomicUsize::new(0),
            already_local: std::sync::atomic::AtomicUsize::new(0),
            missing: std::sync::atomic::AtomicUsize::new(0),
            first_missing: Mutex::new(None),
        }
    }

    fn fetched(&self) -> usize {
        self.fetched.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn already_local(&self) -> usize {
        self.already_local
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    fn missing(&self) -> usize {
        self.missing.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn first_missing(&self) -> Option<String> {
        self.first_missing
            .lock()
            .ok()
            .and_then(|value| value.clone())
    }

    async fn fetch_remote(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        let direct_peers = self.transport.peer_ids().await;
        let direct = async {
            if direct_peers.is_empty() {
                Ok(None)
            } else {
                self.transport
                    .get_from_peers(hash, &direct_peers)
                    .await
                    .map_err(|error| StoreError::Other(error.to_string()))
            }
        };
        let mesh = self.mesh.get(hash);
        tokio::pin!(direct);
        tokio::pin!(mesh);

        let mut direct_done = false;
        let mut mesh_done = false;
        let mut first_error: Option<StoreError> = None;

        loop {
            tokio::select! {
                result = &mut direct, if !direct_done => {
                    direct_done = true;
                    match result {
                        Ok(Some(bytes)) => return Ok(Some(bytes)),
                        Ok(None) => {}
                        Err(error) => {
                            if first_error.is_none() {
                                first_error = Some(error);
                            }
                        }
                    }
                }
                result = &mut mesh, if !mesh_done => {
                    mesh_done = true;
                    match result {
                        Ok(Some(bytes)) => return Ok(Some(bytes)),
                        Ok(None) => {}
                        Err(error) => {
                            if first_error.is_none() {
                                first_error = Some(error);
                            }
                        }
                    }
                }
                else => break,
            }

            if direct_done && mesh_done {
                break;
            }
        }

        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(None)
    }
}

#[async_trait]
impl<L: Store + Send + Sync + 'static> Store for WriteBackOverlayStore<L> {
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
        self.local.put(hash, data).await
    }

    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        if let Some(bytes) = self.local.get(hash).await? {
            self.already_local
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            return Ok(Some(bytes));
        }

        if let Some(bytes) = self.fetch_remote(hash).await? {
            self.fetched
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Ok(Some(bytes))
        } else {
            let hex = to_hex(hash);
            if self
                .missing
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                == 0
                && let Ok(mut first_missing) = self.first_missing.lock()
            {
                *first_missing = Some(hex);
            }
            Ok(None)
        }
    }

    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.local.has(hash).await
    }

    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.local.delete(hash).await
    }
}

#[cfg(test)]
mod tests;
