//! Direct block replication over hashtree's FIPS transport.
//!
//! Nostr relay events carry Iris Drive metadata: the `AppKey` roster and
//! per-AppKey root CIDs. This module moves the actual hashtree blocks directly
//! between authorized app installs. Blossom remains useful as a public remote,
//! but the local app should first ask peer instances over FIPS.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fips_core::FipsEndpoint;
use hashtree_core::{BLOB_DEFAULT_HTL, Cid, HashTreeError, Store};
use hashtree_fips_transport::{
    BoundFipsEndpoint, FipsPeerConfig, SameHostBlobStoreConfig, bind_fips_endpoint,
    set_fips_peer_configs,
};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::ToBech32;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::app_key_link_transport::{
    APP_KEY_APPROVAL_APPLIED_ACK_APP_TOPIC, APP_KEY_APPROVAL_RECEIPT_APP_TOPIC,
    APP_KEY_LINK_REQUEST_APP_TOPIC, APP_KEY_LINK_ROSTER_ACK_APP_TOPIC,
    APP_KEY_LINK_ROSTER_APP_TOPIC,
};
use crate::blossom_sync::DownloadReport;
use crate::config::AppConfig;
use crate::fips_bootstrap::DEFAULT_FIPS_BOOTSTRAP_PEERS;
use crate::identity::AppKey;

mod blob_runtime;
mod control_runtime;
mod download;
mod nostr_runtime;
mod settings_runtime;
use blob_runtime::DriveBlobRuntime;
use control_runtime::DriveControlRuntime;
pub use control_runtime::FipsAppMessage;
use download::download_tree_with_resolver;
use nostr_runtime::DriveNostrPubsubRuntime;
pub use nostr_runtime::FipsNostrPubsubEvent;
use settings_runtime::fips_endpoint_options;
pub use settings_runtime::{FipsTransportSettings, IRIS_DRIVE_FIPS_DISCOVERY_SCOPE};

const DRIVE_BLOB_SEARCH_TIMEOUT: Duration = Duration::from_secs(10);
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
    endpoint: Arc<FipsEndpoint>,
    blob_store: Arc<dyn Store>,
    blob_runtime: Option<DriveBlobRuntime<L>>,
    control_runtime: Option<DriveControlRuntime>,
    nostr_runtime: Option<DriveNostrPubsubRuntime>,
    nostr_receiver:
        Option<tokio::sync::Mutex<tokio::sync::broadcast::Receiver<FipsNostrPubsubEvent>>>,
    local_store: Arc<L>,
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
        let transport_settings = FipsTransportSettings::from_env();
        let endpoint = Box::pin(bind_fips_endpoint(fips_endpoint_options(
            identity_nsec,
            discovery_scope.clone(),
            config.relays.clone(),
            config,
            &transport_settings,
        )))
        .await
        .map_err(|error| FipsSyncError::Endpoint(error.to_string()))?;

        Self::start_with_bound_endpoint(endpoint, local_store, config, transport_settings).await
    }

    async fn start_with_bound_endpoint(
        endpoint: BoundFipsEndpoint,
        local_store: Arc<L>,
        config: &AppConfig,
        transport_settings: FipsTransportSettings,
    ) -> Result<Self, FipsSyncError> {
        let BoundFipsEndpoint {
            native_endpoint,
            local_peer_id,
            discovery_scope,
            ..
        } = endpoint;

        let application_peers = authorized_device_fips_peers(config, &transport_settings);
        let routing_peers = routing_fips_peers(config, &transport_settings);
        let blob_peers = authorized_blob_fips_peers(config, &transport_settings);
        set_fips_peer_configs(
            native_endpoint.as_ref(),
            merged_endpoint_peers(&application_peers, &routing_peers, &blob_peers),
        )
        .await
        .map_err(|error| FipsSyncError::Endpoint(error.to_string()))?;
        let blob_runtime = DriveBlobRuntime::bind(
            native_endpoint.clone(),
            local_store.clone(),
            local_peer_id.clone(),
            &blob_peers,
        )
        .await?;
        let blob_store: Arc<dyn Store> = blob_runtime.store.clone();
        let control_runtime = DriveControlRuntime::bind(
            native_endpoint.clone(),
            peer_ids(&application_peers),
            control_bootstrap_topics(config),
        )
        .await?;
        let nostr_runtime = if transport_settings.enable_mesh_pubsub {
            Some(DriveNostrPubsubRuntime::bind(native_endpoint.clone()).await?)
        } else {
            None
        };
        let nostr_receiver = nostr_runtime
            .as_ref()
            .map(|runtime| tokio::sync::Mutex::new(runtime.subscribe()));
        let peer_snapshot = fips_peer_config_snapshot(
            Some(local_peer_id.as_str()),
            &application_peers,
            &routing_peers,
            &blob_peers,
        );

        Ok(Self {
            endpoint: native_endpoint,
            blob_store,
            blob_runtime: Some(blob_runtime),
            control_runtime: Some(control_runtime),
            nostr_runtime,
            nostr_receiver,
            local_store,
            endpoint_npub: local_peer_id,
            discovery_scope,
            transport_settings,
            last_peer_config: Mutex::new(Some(peer_snapshot)),
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
        let blob_peers = authorized_blob_fips_peers(config, &self.transport_settings);
        if accepts_app_key_link_requests(config) {
            add_connected_app_key_link_application_peers(
                &mut application_peers,
                &routing_peers,
                self.endpoint_npub.as_str(),
                self.connected_peer_ids().await,
            );
        }
        let snapshot = fips_peer_config_snapshot(
            Some(self.endpoint_npub.as_str()),
            &application_peers,
            &routing_peers,
            &blob_peers,
        );
        if !self.update_peer_config_snapshot(snapshot) {
            return;
        }
        if let Err(error) = set_fips_peer_configs(
            self.endpoint.as_ref(),
            merged_endpoint_peers(&application_peers, &routing_peers, &blob_peers),
        )
        .await
        {
            tracing::warn!(%error, "failed to refresh Drive FIPS peers");
        }
        if let Some(runtime) = self.control_runtime.as_ref()
            && let Err(error) = runtime
                .set_policy(
                    peer_ids(&application_peers),
                    control_bootstrap_topics(config),
                )
                .await
        {
            tracing::warn!(%error, "failed to refresh Drive control policy");
        }
        if let Some(runtime) = self.blob_runtime.as_ref() {
            runtime.set_authorized_peers(&blob_peers).await;
        }
    }

    pub async fn peer_ids(&self) -> Vec<String> {
        endpoint_peers(self.endpoint.as_ref())
            .await
            .into_iter()
            .map(|peer| peer.npub)
            .collect()
    }

    pub fn authorized_peer_ids(&self) -> Vec<String> {
        self.last_peer_config
            .lock()
            .expect("FIPS peer config snapshot lock poisoned")
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .application
                    .iter()
                    .map(|peer| peer.npub.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub async fn connected_peer_ids(&self) -> Vec<String> {
        endpoint_peers(self.endpoint.as_ref())
            .await
            .into_iter()
            .filter(|peer| peer.connected)
            .map(|peer| peer.npub)
            .collect()
    }

    #[must_use]
    pub fn same_host_blob_provider_ids(&self) -> Vec<String> {
        same_host_blob_provider_ids(
            self.endpoint
                .local_instance_advertisements()
                .unwrap_or_default(),
        )
    }

    pub async fn fips_peer_statuses(&self) -> Vec<FipsPeerStatus> {
        endpoint_peers(self.endpoint.as_ref())
            .await
            .into_iter()
            .map(|peer| FipsPeerStatus {
                npub: peer.npub,
                connected: peer.connected,
                transport_addr: peer.transport_addr,
                transport_type: peer.transport_type,
                srtt_ms: peer.srtt_ms,
                packets_sent: peer.packets_sent,
                packets_recv: peer.packets_recv,
                bytes_sent: peer.bytes_sent,
                bytes_recv: peer.bytes_recv,
            })
            .collect()
    }

    pub async fn fips_relay_statuses(&self) -> Vec<FipsRelayStatus> {
        self.endpoint
            .relay_statuses()
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|relay| FipsRelayStatus {
                url: relay.url,
                status: relay.status,
            })
            .collect()
    }

    #[must_use]
    pub fn subscribe_app_messages(&self) -> tokio::sync::broadcast::Receiver<FipsAppMessage> {
        self.control_runtime
            .as_ref()
            .expect("Drive control runtime is stopped")
            .subscribe()
    }

    #[must_use]
    pub fn mesh_pubsub_enabled(&self) -> bool {
        self.nostr_runtime.is_some()
    }

    pub async fn send_app_message(
        &self,
        peer_id: &str,
        topic: &str,
        data: Vec<u8>,
    ) -> Result<(), FipsSyncError> {
        self.control_runtime
            .as_ref()
            .ok_or_else(|| FipsSyncError::Endpoint("Drive control runtime is stopped".into()))?
            .send(peer_id.to_string(), topic.to_string(), data)
            .await
    }

    pub async fn broadcast_app_message(
        &self,
        topic: &str,
        data: Vec<u8>,
    ) -> Result<usize, FipsSyncError> {
        self.control_runtime
            .as_ref()
            .ok_or_else(|| FipsSyncError::Endpoint("Drive control runtime is stopped".into()))?
            .broadcast(topic.to_string(), data)
            .await
    }

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

    pub async fn recv_nostr_pubsub_event(&self) -> FipsNostrPubsubEvent {
        self.nostr_receiver
            .as_ref()
            .expect("Nostr pubsub is disabled")
            .lock()
            .await
            .recv()
            .await
            .expect("Nostr pubsub runtime stopped")
    }

    pub async fn mesh_peer_count(&self) -> usize {
        let Some(runtime) = self.nostr_runtime.as_ref() else {
            return 0;
        };
        runtime.connected_peer_count().await
    }

    pub async fn download_tree(&self, root: &Cid) -> Result<DownloadReport, FipsSyncError> {
        download_tree_with_resolver(self.local_store.clone(), root, self.blob_store.clone()).await
    }

    pub async fn shutdown(mut self) -> Result<(), FipsSyncError> {
        if let Some(mut runtime) = self.control_runtime.take() {
            runtime.shutdown().await?;
        }
        if let Some(mut runtime) = self.nostr_runtime.take() {
            runtime.shutdown().await;
        }
        self.shutdown_endpoint().await
    }

    pub async fn shutdown_endpoint(&self) -> Result<(), FipsSyncError> {
        self.endpoint
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

fn same_host_blob_provider_ids(
    advertisements: impl IntoIterator<Item = fips_core::discovery::local::LocalInstanceAdvertisement>,
) -> Vec<String> {
    let mut providers = advertisements
        .into_iter()
        .filter(|advertisement| {
            advertisement
                .capability(hashtree_fips_transport::TCP_BLOB_CAPABILITY)
                .and_then(|capability| capability.fsp_port)
                == Some(hashtree_fips_transport::TCP_BLOB_SERVICE_PORT)
        })
        .map(|advertisement| advertisement.npub)
        .collect::<Vec<_>>();
    providers.sort();
    providers.dedup();
    providers
}

fn drive_same_host_blob_store_config() -> SameHostBlobStoreConfig {
    SameHostBlobStoreConfig::default()
        .with_provider_htl(BLOB_DEFAULT_HTL)
        .with_standalone_htl(BLOB_DEFAULT_HTL)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FipsRelayStatus {
    pub url: String,
    pub status: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FipsPeerStatus {
    pub npub: String,
    pub connected: bool,
    pub transport_addr: Option<String>,
    pub transport_type: Option<String>,
    pub srtt_ms: Option<u64>,
    pub packets_sent: u64,
    pub packets_recv: u64,
    pub bytes_sent: u64,
    pub bytes_recv: u64,
}

async fn endpoint_peers(endpoint: &FipsEndpoint) -> Vec<fips_core::endpoint::FipsEndpointPeer> {
    endpoint.peers().await.unwrap_or_else(|error| {
        tracing::warn!(%error, "failed to snapshot Drive FIPS peers");
        Vec::new()
    })
}

fn peer_ids(peers: &[FipsPeerConfig]) -> BTreeSet<String> {
    peers
        .iter()
        .map(|peer| peer.npub.trim())
        .filter(|npub| !npub.is_empty())
        .map(str::to_string)
        .collect()
}

fn merged_endpoint_peers(
    groups: &[FipsPeerConfig],
    routing: &[FipsPeerConfig],
    blob: &[FipsPeerConfig],
) -> Vec<FipsPeerConfig> {
    let mut peers = std::collections::BTreeMap::<String, BTreeSet<String>>::new();
    for peer in groups.iter().chain(routing).chain(blob) {
        let npub = peer.npub.trim();
        if npub.is_empty() {
            continue;
        }
        let addresses = peers.entry(npub.to_string()).or_default();
        addresses.extend(
            peer.udp_addresses
                .iter()
                .map(|address| address.trim())
                .filter(|address| !address.is_empty())
                .map(str::to_string),
        );
    }
    peers
        .into_iter()
        .map(|(npub, udp_addresses)| FipsPeerConfig {
            npub,
            udp_addresses: udp_addresses.into_iter().collect(),
        })
        .collect()
}

fn control_bootstrap_topics(config: &AppConfig) -> BTreeSet<&'static str> {
    let mut topics = BTreeSet::from([
        APP_KEY_APPROVAL_RECEIPT_APP_TOPIC,
        APP_KEY_APPROVAL_APPLIED_ACK_APP_TOPIC,
        APP_KEY_LINK_ROSTER_APP_TOPIC,
        APP_KEY_LINK_ROSTER_ACK_APP_TOPIC,
    ]);
    if accepts_app_key_link_requests(config) {
        topics.insert(APP_KEY_LINK_REQUEST_APP_TOPIC);
    }
    topics
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct FipsPeerConfigSnapshot {
    application: Vec<FipsPeerConfig>,
    routing: Vec<FipsPeerConfig>,
    blob: Vec<FipsPeerConfig>,
}

fn fips_peer_config_snapshot(
    local: Option<&str>,
    application_peers: &[FipsPeerConfig],
    routing_peers: &[FipsPeerConfig],
    blob_peers: &[FipsPeerConfig],
) -> FipsPeerConfigSnapshot {
    let mut seen = std::collections::HashSet::new();
    let mut blob_seen = std::collections::HashSet::new();
    FipsPeerConfigSnapshot {
        application: normalize_fips_peer_configs(local, application_peers, &mut seen),
        routing: normalize_fips_peer_configs(local, routing_peers, &mut seen),
        blob: normalize_fips_peer_configs(local, blob_peers, &mut blob_seen),
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

fn accepts_app_key_link_requests(config: &AppConfig) -> bool {
    config
        .profile
        .as_ref()
        .is_some_and(crate::ProfileState::can_admin_profile)
}

#[must_use]
pub fn discovery_scope(config: &AppConfig) -> String {
    if let Some(profile) = &config.profile {
        return format!("iris-drive:{}", profile.profile_id);
    }
    IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string()
}

fn authorized_device_fips_peers(
    config: &AppConfig,
    settings: &FipsTransportSettings,
) -> Vec<FipsPeerConfig> {
    let Some(account) = config.profile.as_ref() else {
        return Vec::new();
    };
    let mut peers = authorized_blob_fips_peers(config, settings);
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

fn authorized_blob_fips_peers(
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
    let request = account.outbound_app_key_link_request.as_ref()?;
    let request_can_still_receive_full_roster = matches!(
        account.authorization_state,
        crate::AppKeyAuthorizationState::AwaitingApproval
    )
        || crate::app_key_link_transport::pending_app_key_approval_receipt_authorizes_app_key(
            request,
            &account.app_key_pubkey,
        );
    if account.can_admin_profile() || !request_can_still_receive_full_roster {
        return None;
    }
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

#[cfg(test)]
mod tests;
