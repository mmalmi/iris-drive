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
use hashtree_fips_transport::{FipsEndpointOptions, HashtreeFipsTransport, bind_fips_endpoint};
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
    endpoint_npub: String,
    discovery_scope: String,
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
        let endpoint = bind_fips_endpoint(FipsEndpointOptions {
            identity_nsec,
            discovery_scope: discovery_scope.clone(),
            relays: config.relays.clone(),
            enable_udp: true,
            enable_webrtc: true,
            udp_bind_addr: None,
            udp_public: false,
            udp_external_addr: None,
            webrtc_auto_connect: true,
            webrtc_max_connections: FIPS_WEBRTC_MAX_CONNECTIONS,
            open_discovery_max_pending: 0,
            packet_channel_capacity: FIPS_PACKET_CHANNEL_CAPACITY,
        })
        .await
        .map_err(|error| FipsSyncError::Endpoint(error.to_string()))?;

        let transport = Arc::new(
            HashtreeFipsTransport::new(endpoint.endpoint, local_store.clone())
                .with_request_timeout(FIPS_REQUEST_TIMEOUT)
                .with_request_retry_interval(FIPS_REQUEST_RETRY_INTERVAL)
                .with_request_max_attempts(FIPS_REQUEST_MAX_ATTEMPTS),
        );
        let receiver_task = transport.start();
        transport.set_peers(authorized_device_npubs(config)).await;

        Ok(Self {
            transport,
            local_store,
            receiver_task,
            endpoint_npub: endpoint.local_peer_id,
            discovery_scope,
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

    pub async fn refresh_authorized_peers(&self, config: &AppConfig) {
        self.transport
            .set_peers(authorized_device_npubs(config))
            .await;
    }

    pub async fn peer_ids(&self) -> Vec<String> {
        self.transport.peer_ids().await
    }

    pub async fn download_tree(&self, root: &Cid) -> Result<DownloadReport, FipsSyncError> {
        download_tree_with_transport(self.local_store.clone(), root, self.transport.clone()).await
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

fn authorized_device_npubs(config: &AppConfig) -> Vec<String> {
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

        match self.transport.get(hash).await? {
            Some(bytes) => {
                self.fetched
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Ok(Some(bytes))
            }
            None => {
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
        let mut config = AppConfig::default();
        config.account = Some(crate::AccountState {
            owner_pubkey: "aa".repeat(32),
            device_pubkey: "bb".repeat(32),
            has_owner_signing_authority: false,
            authorization_state: crate::DeviceAuthorizationState::AwaitingApproval,
            device_label: None,
            app_keys: None,
        });

        assert_eq!(
            discovery_scope(&config),
            format!("{IRIS_DRIVE_FIPS_SCOPE_PREFIX}:{}", "aa".repeat(32))
        );
    }
}
