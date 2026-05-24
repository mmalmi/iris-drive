//! Direct block replication over hashtree's FIPS transport.
//!
//! Nostr relay events carry Iris Drive metadata: the device roster and
//! per-device root CIDs. This module moves the actual hashtree blocks directly
//! between authorized devices. Blossom remains useful as a public fallback, but
//! the local app should first ask peer instances over FIPS.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use hashtree_core::diff::collect_hashes;
use hashtree_core::{
    Cid, Hash, HashTree, HashTreeConfig, HashTreeError, Store, StoreError, to_hex,
};
use hashtree_fips_transport::{
    FipsAppMessage, FipsEndpointOptions, FipsMeshPubsub, FipsMeshPubsubEvent, FipsPeerConfig,
    HashtreeFipsTransport, PubsubPublishStats, bind_fips_endpoint,
};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::ToBech32;
use thiserror::Error;
use tokio::task::JoinHandle;

use crate::blossom_sync::DownloadReport;
use crate::config::AppConfig;
use crate::identity::DeviceIdentity;

const IRIS_DRIVE_FIPS_SCOPE_PREFIX: &str = "iris-drive-v1";
const FIPS_REQUEST_TIMEOUT: Duration = Duration::from_millis(5_500);
const FIPS_REQUEST_RETRY_INTERVAL: Duration = Duration::from_millis(750);
const FIPS_REQUEST_MAX_ATTEMPTS: usize = 4;
const FIPS_PACKET_CHANNEL_CAPACITY: usize = 1024;
const FIPS_WEBRTC_MAX_CONNECTIONS: usize = 64;
const FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING: usize = 8;
pub const FIPS_NOSTR_DISCOVERY_APP: &str = "fips-overlay-v1";

/// Shared public FIPS bootstrap/transit nodes. Kept in sync with nostr-vpn's
/// defaults so native Iris instances can join the same fallback overlay when
/// direct device-to-device UDP/WebRTC is unavailable.
const DEFAULT_FIPS_BOOTSTRAP_PEERS: &[(&str, &[&str])] = &[
    (
        "npub1260n42s06vzc7796w0fh3ny7zcpw6tlk4gq3940gmfrzl5c9pv2s3657q8",
        &["udp:217.160.76.169:2121"],
    ),
    (
        "npub17lpmzulpc98d8ff727k6e98atxn3phzupzsqqwe54ytduym747ws4tw5zm",
        &["udp:82.223.139.182:2121"],
    ),
    (
        "npub1u0z26dc4qeneu5rvwvmpfhtwh3522ed6rlgxr9jarrfnjrc6ew4qxjysrs",
        &["udp:88.208.241.33:2121"],
    ),
    (
        "npub1qmc3cvfz0yu2hx96nq3gp55zdan2qclealn7xshgr448d3nh6lks7zel98",
        &["udp:217.77.8.91:2121", "tcp:217.77.8.91:443"],
    ),
    (
        "npub10yffd020a4ag8zcy75f9pruq3rnghvvhd5hphl9s62zgp35s560qrksp9u",
        &["udp:23.182.128.74:2121", "tcp:23.182.128.74:443"],
    ),
    (
        "npub136yqae6na688fs75g95ppps3lxe07fvxefj77938zf47uhm6074sxw8ctm",
        &["udp:54.183.70.180:2121", "tcp:54.183.70.180:443"],
    ),
    (
        "npub1gd7ye2qp2lphhzx75fynnjzaxx4dqanddecet0wtt5ss5ek8h9ps62wdkf",
        &["udp:74.208.245.160:2121"],
    ),
];

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
    receiver_task: JoinHandle<()>,
    mesh_pubsub: FipsMeshPubsub<L>,
    endpoint_npub: String,
    discovery_scope: String,
    transport_settings: FipsTransportSettings,
}

pub type FsFipsBlockSync = FipsBlockSync<hashtree_fs::FsBlobStore>;

impl<L: Store + Send + Sync + 'static> FipsBlockSync<L> {
    pub async fn start(
        device: &DeviceIdentity,
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
        let transport_settings = FipsTransportSettings::from_env();
        let endpoint = Box::pin(bind_fips_endpoint(fips_endpoint_options(
            identity_nsec,
            discovery_scope.clone(),
            config.relays.clone(),
            &transport_settings,
        )))
        .await
        .map_err(|error| FipsSyncError::Endpoint(error.to_string()))?;

        let transport = Arc::new(
            HashtreeFipsTransport::new(endpoint.endpoint, local_store.clone())
                .with_request_timeout(FIPS_REQUEST_TIMEOUT)
                .with_request_retry_interval(FIPS_REQUEST_RETRY_INTERVAL)
                .with_request_max_attempts(FIPS_REQUEST_MAX_ATTEMPTS),
        );
        transport
            .set_peer_configs_with_routing_peers(
                authorized_device_fips_peers(config, &transport_settings),
                bootstrap_fips_peers(&transport_settings),
            )
            .await;
        let receiver_task = transport.start();
        let mesh_pubsub = transport
            .start_mesh_pubsub(
                local_store.clone(),
                endpoint.local_peer_id.clone(),
                FIPS_REQUEST_TIMEOUT,
            )
            .await
            .map_err(|error| FipsSyncError::Endpoint(error.to_string()))?;

        Ok(Self {
            transport,
            local_store,
            receiver_task,
            mesh_pubsub,
            endpoint_npub: endpoint.local_peer_id,
            discovery_scope,
            transport_settings,
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
    pub fn nostr_discovery_app(&self) -> &'static str {
        FIPS_NOSTR_DISCOVERY_APP
    }

    #[must_use]
    pub fn transport_settings(&self) -> &FipsTransportSettings {
        &self.transport_settings
    }

    pub async fn refresh_authorized_peers(&self, config: &AppConfig) {
        self.transport
            .set_peer_configs_with_routing_peers(
                authorized_device_fips_peers(config, &self.transport_settings),
                bootstrap_fips_peers(&self.transport_settings),
            )
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

    #[must_use]
    pub fn subscribe_app_messages(&self) -> tokio::sync::broadcast::Receiver<FipsAppMessage> {
        self.transport.subscribe_app_messages()
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
        self.mesh_pubsub.subscribe_pubsub(stream_id).await
    }

    pub async fn publish_mesh_pubsub(
        &self,
        stream_id: String,
        seq: u64,
        payload: Vec<u8>,
    ) -> PubsubPublishStats {
        self.mesh_pubsub
            .publish_pubsub(stream_id, seq, payload)
            .await
    }

    pub async fn drain_mesh_pubsub_events(&self) -> Vec<FipsMeshPubsubEvent> {
        self.mesh_pubsub.drain_pubsub_events().await
    }

    pub async fn mesh_peer_count(&self) -> usize {
        self.mesh_pubsub.peer_count().await
    }

    pub async fn mesh_peer_ids(&self) -> Vec<String> {
        self.mesh_pubsub.peer_ids().await
    }

    pub async fn download_tree(&self, root: &Cid) -> Result<DownloadReport, FipsSyncError> {
        download_tree_with_transport(self.local_store.clone(), root, self.transport.clone()).await
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FipsTransportSettings {
    pub enable_udp: bool,
    pub enable_webrtc: bool,
    pub udp_bind_addr: Option<String>,
    pub udp_public: bool,
    pub udp_external_addr: Option<String>,
    pub static_peer_hints: Vec<(String, Vec<String>)>,
    pub bootstrap_peer_hints: Vec<(String, Vec<String>)>,
    pub open_discovery_max_pending: usize,
}

impl Default for FipsTransportSettings {
    fn default() -> Self {
        Self {
            enable_udp: true,
            enable_webrtc: true,
            udp_bind_addr: None,
            udp_public: false,
            udp_external_addr: None,
            static_peer_hints: Vec::new(),
            bootstrap_peer_hints: default_fips_bootstrap_peer_hints(),
            open_discovery_max_pending: FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING,
        }
    }
}

impl FipsTransportSettings {
    #[must_use]
    pub fn from_env() -> Self {
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
            enable_udp: bool_env("IRIS_DRIVE_FIPS_ENABLE_UDP").unwrap_or(true),
            enable_webrtc: bool_env("IRIS_DRIVE_FIPS_ENABLE_WEBRTC").unwrap_or(true),
            udp_bind_addr,
            udp_public,
            udp_external_addr,
            static_peer_hints: parse_static_peer_hints(
                &std::env::var("IRIS_DRIVE_FIPS_STATIC_PEERS").unwrap_or_default(),
            ),
            bootstrap_peer_hints,
            open_discovery_max_pending: usize_env("IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING")
                .unwrap_or(FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING),
        }
    }
}

fn fips_endpoint_options(
    identity_nsec: String,
    discovery_scope: String,
    relays: Vec<String>,
    settings: &FipsTransportSettings,
) -> FipsEndpointOptions {
    FipsEndpointOptions {
        identity_nsec,
        discovery_scope,
        relays,
        enable_udp: settings.enable_udp,
        enable_webrtc: settings.enable_webrtc,
        udp_bind_addr: settings.udp_bind_addr.clone(),
        udp_public: settings.udp_public,
        udp_external_addr: settings.udp_external_addr.clone(),
        webrtc_auto_connect: true,
        webrtc_max_connections: FIPS_WEBRTC_MAX_CONNECTIONS,
        open_discovery_max_pending: settings.open_discovery_max_pending,
        packet_channel_capacity: FIPS_PACKET_CHANNEL_CAPACITY,
    }
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
        self.receiver_task.abort();
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
    let hashes = collect_hashes(&tree, root, 4).await?;
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
    config.account.as_ref().map_or_else(
        || IRIS_DRIVE_FIPS_SCOPE_PREFIX.to_string(),
        |account| format!("{IRIS_DRIVE_FIPS_SCOPE_PREFIX}:{}", account.owner_pubkey),
    )
}

fn authorized_device_fips_peers(
    config: &AppConfig,
    settings: &FipsTransportSettings,
) -> Vec<FipsPeerConfig> {
    let Some(account) = config.account.as_ref() else {
        return Vec::new();
    };
    let Some(app_keys) = account.app_keys.as_ref() else {
        return Vec::new();
    };
    let local_device = &account.device_pubkey;
    app_keys
        .devices
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
        })
        .collect()
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
    device: &crate::app_keys::DeviceEntry,
    npub: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (key, addresses) in hints {
        if !static_peer_key_matches_device(key, device, npub) {
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

fn static_peer_key_matches_device(
    key: &str,
    device: &crate::app_keys::DeviceEntry,
    npub: &str,
) -> bool {
    let key = key.trim();
    if key.is_empty() {
        return false;
    }
    key.eq_ignore_ascii_case(npub)
        || key.eq_ignore_ascii_case(&device.pubkey)
        || device
            .label
            .as_deref()
            .is_some_and(|label| key.eq_ignore_ascii_case(label.trim()))
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

#[cfg(test)]
mod tests {
    use super::*;
    use hashtree_core::{DirEntry, LinkType, MemoryStore};
    use hashtree_fips_transport::{FipsEndpointIo, FipsEndpointPacket, FipsTransportError};
    use tokio::sync::{Mutex as TokioMutex, mpsc};

    struct FakeEndpoint {
        id: String,
        network: Arc<
            TokioMutex<
                std::collections::HashMap<String, mpsc::UnboundedSender<FipsEndpointPacket>>,
            >,
        >,
        rx: TokioMutex<mpsc::UnboundedReceiver<FipsEndpointPacket>>,
    }

    impl FakeEndpoint {
        async fn new(
            id: &str,
            network: Arc<
                TokioMutex<
                    std::collections::HashMap<String, mpsc::UnboundedSender<FipsEndpointPacket>>,
                >,
            >,
        ) -> Arc<Self> {
            let (tx, rx) = mpsc::unbounded_channel();
            network.lock().await.insert(id.to_string(), tx);
            Arc::new(Self {
                id: id.to_string(),
                network,
                rx: TokioMutex::new(rx),
            })
        }
    }

    #[async_trait]
    impl FipsEndpointIo for FakeEndpoint {
        async fn send(&self, peer_id: &str, data: Vec<u8>) -> Result<(), FipsTransportError> {
            let tx = self
                .network
                .lock()
                .await
                .get(peer_id)
                .cloned()
                .ok_or_else(|| FipsTransportError::Send(format!("unknown peer {peer_id}")))?;
            tx.send(FipsEndpointPacket {
                peer_id: self.id.clone(),
                data,
            })
            .map_err(|_| FipsTransportError::Send("receiver closed".to_string()))
        }

        async fn recv(&self) -> Option<FipsEndpointPacket> {
            self.rx.lock().await.recv().await
        }

        async fn peer_ids(&self) -> Vec<String> {
            self.network
                .lock()
                .await
                .keys()
                .filter(|id| *id != &self.id)
                .cloned()
                .collect()
        }

        fn local_peer_id(&self) -> Option<String> {
            Some(self.id.clone())
        }
    }

    #[tokio::test]
    async fn downloads_tree_blocks_from_direct_fips_peer() {
        let network = Arc::new(TokioMutex::new(std::collections::HashMap::new()));
        let source_endpoint = FakeEndpoint::new("source", network.clone()).await;
        let target_endpoint = FakeEndpoint::new("target", network).await;

        let source_store = Arc::new(MemoryStore::new());
        let source_tree = HashTree::new(HashTreeConfig::new(source_store.clone()));
        let (file_cid, _) = source_tree.put(b"hello from fips").await.unwrap();
        let root_cid = source_tree
            .put_directory(vec![DirEntry {
                name: "hello.txt".to_string(),
                hash: file_cid.hash,
                key: file_cid.key,
                link_type: LinkType::File,
                size: 15,
                meta: None,
            }])
            .await
            .unwrap();

        let source_transport = Arc::new(HashtreeFipsTransport::new(source_endpoint, source_store));
        let source_task = source_transport.start();

        let target_store = Arc::new(MemoryStore::new());
        let target_transport = Arc::new(HashtreeFipsTransport::new(
            target_endpoint,
            target_store.clone(),
        ));
        target_transport.set_peers(vec!["source".to_string()]).await;
        let target_task = target_transport.start();

        let report =
            download_tree_with_transport(target_store.clone(), &root_cid, target_transport)
                .await
                .unwrap();

        assert_eq!(report.fetched, 2);
        assert_eq!(report.already_local, 0);
        assert!(target_store.has(&root_cid.hash).await.unwrap());
        assert!(target_store.has(&file_cid.hash).await.unwrap());

        source_task.abort();
        target_task.abort();
    }

    #[test]
    fn discovery_scope_is_owner_scoped() {
        let config = AppConfig {
            account: Some(crate::AccountState {
                owner_pubkey: "aa".repeat(32),
                device_pubkey: "bb".repeat(32),
                has_owner_signing_authority: false,
                authorization_state: crate::DeviceAuthorizationState::AwaitingApproval,
                device_label: None,
                app_keys: None,
            }),
            ..Default::default()
        };

        assert_eq!(
            discovery_scope(&config),
            format!("{IRIS_DRIVE_FIPS_SCOPE_PREFIX}:{}", "aa".repeat(32))
        );
    }

    #[test]
    fn endpoint_options_can_advertise_native_udp_without_disabling_webrtc() {
        let settings = FipsTransportSettings {
            enable_udp: true,
            enable_webrtc: true,
            udp_bind_addr: Some("0.0.0.0:2121".to_string()),
            udp_public: true,
            udp_external_addr: Some("10.44.94.98:2121".to_string()),
            static_peer_hints: Vec::new(),
            bootstrap_peer_hints: Vec::new(),
            open_discovery_max_pending: 8,
        };

        let options = fips_endpoint_options(
            "nsec1example".to_string(),
            "iris-drive-v1:test".to_string(),
            vec!["wss://relay.example".to_string()],
            &settings,
        );

        assert!(options.enable_udp);
        assert!(options.enable_webrtc);
        assert_eq!(options.udp_bind_addr.as_deref(), Some("0.0.0.0:2121"));
        assert!(options.udp_public);
        assert_eq!(
            options.udp_external_addr.as_deref(),
            Some("10.44.94.98:2121")
        );
        assert!(options.webrtc_auto_connect);
        assert_eq!(options.open_discovery_max_pending, 8);
    }

    #[test]
    fn default_transport_settings_seed_fips_bootstrap_transit() {
        let settings = FipsTransportSettings::default();

        assert_eq!(settings.open_discovery_max_pending, 8);
        assert_eq!(
            settings.bootstrap_peer_hints.len(),
            DEFAULT_FIPS_BOOTSTRAP_PEERS.len()
        );
        assert!(settings.bootstrap_peer_hints.iter().any(|(_, addresses)| {
            addresses
                .iter()
                .any(|address| address.starts_with("tcp:") || address.starts_with("udp:"))
        }));
    }

    #[test]
    fn static_peer_hints_match_authorized_devices_by_label_or_npub() {
        let first_keys = nostr_sdk::Keys::generate();
        let second_keys = nostr_sdk::Keys::generate();
        let first_pubkey = first_keys.public_key().to_hex();
        let second_pubkey = second_keys.public_key().to_hex();
        let first_npub = first_keys.public_key().to_bech32().unwrap();
        let settings = FipsTransportSettings {
            static_peer_hints: parse_static_peer_hints(&format!(
                "ubuntu-dev=10.44.214.2:22121,{first_npub}=10.44.34.102:22121"
            )),
            ..Default::default()
        };
        let config = AppConfig {
            account: Some(crate::AccountState {
                owner_pubkey: "aa".repeat(32),
                device_pubkey: "dd".repeat(32),
                has_owner_signing_authority: false,
                authorization_state: crate::DeviceAuthorizationState::Authorized,
                device_label: None,
                app_keys: Some(crate::app_keys::AppKeysSnapshot {
                    owner_pubkey: "aa".repeat(32),
                    created_at: 1,
                    devices: vec![
                        crate::app_keys::DeviceEntry {
                            pubkey: first_pubkey,
                            added_at: 1,
                            label: Some("macos-utm".into()),
                        },
                        crate::app_keys::DeviceEntry {
                            pubkey: second_pubkey,
                            added_at: 1,
                            label: Some("ubuntu-dev".into()),
                        },
                    ],
                    dck_generation: 0,
                    wrapped_dck: Default::default(),
                }),
            }),
            ..Default::default()
        };

        let peers = authorized_device_fips_peers(&config, &settings);

        assert_eq!(peers.len(), 2);
        assert!(peers.iter().any(|peer| peer.npub == first_npub
            && peer.udp_addresses == vec!["10.44.34.102:22121".to_string()]));
        assert!(
            peers
                .iter()
                .any(|peer| peer.udp_addresses == vec!["10.44.214.2:22121".to_string()])
        );
    }

    #[test]
    fn endpoint_options_keep_native_udp_private_by_default() {
        let settings = FipsTransportSettings::default();

        let options = fips_endpoint_options(
            "nsec1example".to_string(),
            "iris-drive-v1:test".to_string(),
            Vec::new(),
            &settings,
        );

        assert!(options.enable_udp);
        assert!(options.enable_webrtc);
        assert!(!options.udp_public);
        assert!(options.udp_bind_addr.is_none());
        assert!(options.udp_external_addr.is_none());
    }

    #[test]
    fn bool_env_parser_accepts_common_spellings() {
        for value in ["1", "true", "TRUE", "yes", "on"] {
            assert_eq!(parse_bool_env_value(value), Some(true));
        }
        for value in ["0", "false", "FALSE", "no", "off"] {
            assert_eq!(parse_bool_env_value(value), Some(false));
        }
        assert_eq!(parse_bool_env_value("maybe"), None);
    }
}
