//! Private friend commitments over authenticated FIPS control messages.
//! Payloads use Hashtree's existing read-only blob transport, never a new wire.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use anyhow::{Context, Result};
use fips_core::{FipsEndpoint, PeerIdentity};
use hashtree_core::{BlobRoute, MemoryStore};
use hashtree_fips_transport::{
    BoundFipsEndpoint, FipsPeerConfig, TcpBlobTransport, TcpBlobTransportConfig,
    set_fips_peer_configs,
};
use nostr_sdk::{Keys, nips::nip19::ToBech32};
use tokio::sync::watch;
use tokio::task::JoinHandle;

use super::exchange::{BACKUP_TOPIC, Exchange};
use super::routes::{FriendReadRoute, LocalBackupRoute};
use super::status::{read_runtime_status, write_runtime_status};
use super::storage::FriendBackupStore;
use super::{FriendBackupConfig, backup_keys};
use crate::fips_sync::FipsTransportSettings;
use crate::fips_sync::control_runtime::DriveControlRuntime;
use crate::fips_sync::endpoint_config::bind_drive_fips_endpoint;
use crate::fips_sync::settings_runtime::fips_endpoint_options;
use crate::{AppConfig, AppKey};

pub(crate) const BACKUP_SCOPE: &str = "iris-drive-friend-backups-v1";

pub(crate) struct FriendBackupRuntime {
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
    pub read_route: Arc<FriendReadRoute>,
}

impl FriendBackupRuntime {
    pub fn spawn(device: &AppKey) -> Result<Self> {
        let config_dir = device
            .path()
            .parent()
            .context("backup identity has no directory")?
            .to_path_buf();
        let keys = backup_keys(device)?;
        let read_route = Arc::new(FriendReadRoute::default());
        let route = read_route.clone();
        let (stop, stopped) = watch::channel(false);
        let task = tokio::spawn(async move {
            if let Err(error) = run(config_dir, keys, route, stopped).await {
                tracing::warn!(%error, "friend backup service stopped");
            }
        });
        Ok(Self {
            stop,
            task: Some(task),
            read_route,
        })
    }

    pub async fn shutdown(&mut self) {
        let _ = self.stop.send(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
        self.read_route.set_routes(Vec::new());
    }
}

impl Drop for FriendBackupRuntime {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

async fn run(
    config_dir: PathBuf,
    keys: Keys,
    read_route: Arc<FriendReadRoute>,
    mut stop: watch::Receiver<bool>,
) -> Result<()> {
    let mut service: Option<BackupService> = None;
    let mut lease = None;
    let mut tick = tokio::time::interval(Duration::from_secs(5));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let result: Result<()> = async {
    loop {
        tokio::select! {
            _ = stop.changed() => break,
            _ = tick.tick() => {
                let app_config = AppConfig::load_or_default(crate::paths::config_path_in(&config_dir))?;
                let friends = FriendBackupConfig::load(&config_dir)?;
                let enabled = app_config.sync_enabled && friends.friends.iter().any(|peer| peer.enabled);
                if !enabled {
                    if let Some(mut active) = service.take() { active.shutdown().await; }
                    read_route.set_routes(Vec::new());
                    lease.take();
                    continue;
                }
                if service.is_none() {
                    if lease.is_none() { lease = super::service_lease::try_acquire(&config_dir)?; }
                    if lease.is_none() { continue; }
                    match BackupService::start(&config_dir, keys.clone(), &app_config, read_route.clone()).await {
                        Ok(active) => service = Some(active),
                        Err(error) => {
                            record_service_error(&config_dir, &friends, &error)?;
                            continue;
                        }
                    }
                }
                if let Some(active) = service.as_mut() {
                    tokio::select! {
                        _ = stop.changed() => break,
                        result = active.tick(&friends) => {
                            if let Err(error) = result { record_service_error(&config_dir, &friends, &error)?; }
                        }
                    }
                }
            },
            message = receive(&mut service) => {
                if let Some((peer, data)) = message
                    && let Some(active) = service.as_mut() {
                    tokio::select! {
                        _ = stop.changed() => break,
                        result = tokio::time::timeout(Duration::from_mins(1), active.handle(&peer, &data)) => {
                            match result {
                                Ok(Ok(())) => {},
                                Ok(Err(error)) => active.exchange.record_error(&peer, &error.to_string())?,
                                Err(_) => active.exchange.record_error(&peer, "Backup transfer paused after its work limit; partial bytes are retained for retry")?,
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
    }.await;
    if let Some(mut active) = service {
        active.shutdown().await;
    }
    read_route.set_routes(Vec::new());
    drop(lease);
    result
}

async fn receive(service: &mut Option<BackupService>) -> Option<(String, Vec<u8>)> {
    let Some(service) = service.as_mut() else {
        return std::future::pending().await;
    };
    loop {
        match service.messages.recv().await {
            Ok(message) if message.topic == BACKUP_TOPIC => {
                return Some((message.peer_id, message.data));
            }
            Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
            Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                return std::future::pending().await;
            }
        }
    }
}

fn record_service_error(
    config_dir: &Path,
    friends: &FriendBackupConfig,
    error: &anyhow::Error,
) -> Result<()> {
    let mut status = read_runtime_status(config_dir)?;
    for peer in &friends.friends {
        status.entry(peer.npub.clone()).or_default().error = Some(error.to_string());
    }
    write_runtime_status(config_dir, &status)
}

pub(crate) struct BackupService {
    pub endpoint: Arc<FipsEndpoint>,
    pub transport: Arc<TcpBlobTransport<MemoryStore>>,
    pub control: DriveControlRuntime,
    messages: tokio::sync::broadcast::Receiver<crate::fips_sync::FipsAppMessage>,
    authorized: Arc<RwLock<Vec<PeerIdentity>>>,
    read_route: Arc<FriendReadRoute>,
    pub exchange: Exchange,
    peer_ids: BTreeSet<String>,
}

impl BackupService {
    pub(crate) async fn start(
        config_dir: &Path,
        keys: Keys,
        config: &AppConfig,
        read_route: Arc<FriendReadRoute>,
    ) -> Result<Self> {
        let mut settings = FipsTransportSettings::from_env();
        // The normal Drive endpoint owns its explicitly configured listeners.
        settings.udp_bind_addr = Some("0.0.0.0:0".into());
        settings.udp_public = false;
        settings.udp_external_addr = None;
        settings.websocket_bind_addr = None;
        settings.open_discovery_max_pending = 0;
        let options = fips_endpoint_options(
            keys.secret_key().to_bech32()?,
            BACKUP_SCOPE.into(),
            config.relays.clone(),
            config,
            &settings,
        );
        let bound = bind_drive_fips_endpoint(options).await?;
        let endpoint = bound.native_endpoint.clone();
        let result = Self::bind(config_dir, keys, bound, read_route).await;
        if result.is_err() {
            // The owner must finish teardown before its identity lease is released.
            let _ = endpoint.shutdown().await;
        }
        result
    }

    pub(crate) async fn bind(
        config_dir: &Path,
        keys: Keys,
        bound: BoundFipsEndpoint,
        read_route: Arc<FriendReadRoute>,
    ) -> Result<Self> {
        let endpoint = bound.native_endpoint;
        let authorized: Arc<RwLock<Vec<PeerIdentity>>> = Arc::default();
        let allowed = authorized.clone();
        let local_route = Arc::new(LocalBackupRoute::default());
        let transport = Arc::new(
            TcpBlobTransport::bind_route_with_config_and_policy(
                endpoint.clone(),
                Arc::new(MemoryStore::new()),
                local_route.clone(),
                TcpBlobTransportConfig {
                    idle_timeout: Duration::from_secs(10),
                },
                Arc::new(move |peer| {
                    allowed
                        .read()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .contains(&peer)
                }),
            )
            .await?,
        );
        let control =
            DriveControlRuntime::bind(endpoint.clone(), BTreeSet::new(), BTreeSet::new()).await?;
        let messages = control.subscribe();
        let exchange = Exchange {
            config_dir: config_dir.to_path_buf(),
            keys,
            store: FriendBackupStore::open(config_dir.join("friend-backups"))?,
            local_route,
            prepared: None,
            last_offer: None,
            status: read_runtime_status(config_dir)?,
        };
        Ok(Self {
            endpoint,
            transport,
            control,
            messages,
            authorized,
            read_route,
            exchange,
            peer_ids: BTreeSet::new(),
        })
    }

    pub(crate) async fn tick(&mut self, config: &FriendBackupConfig) -> Result<()> {
        let peers: BTreeSet<_> = config
            .friends
            .iter()
            .filter(|peer| peer.enabled)
            .map(|peer| peer.npub.clone())
            .collect();
        if self.peer_ids != peers {
            let settings = FipsTransportSettings::from_env();
            set_fips_peer_configs(
                &self.endpoint,
                peers
                    .iter()
                    .map(|npub| FipsPeerConfig {
                        npub: npub.clone(),
                        udp_addresses: settings
                            .static_peer_hints
                            .iter()
                            .find(|(key, _)| key == npub)
                            .map(|(_, addresses)| addresses.clone())
                            .unwrap_or_default(),
                    })
                    .collect(),
            )
            .await?;
            self.control
                .set_policy(peers.clone(), BTreeSet::new())
                .await?;
            *self
                .authorized
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = peers
                .iter()
                .filter_map(|npub| PeerIdentity::from_npub(npub).ok())
                .collect();
            self.read_route.set_routes(
                peers
                    .iter()
                    .filter_map(|npub| PeerIdentity::from_npub(npub).ok())
                    .map(|peer| Arc::new(self.transport.route_to(peer)) as Arc<dyn BlobRoute>)
                    .collect(),
            );
            self.peer_ids = peers;
            self.exchange.last_offer = None;
        }
        self.exchange
            .refresh(config, &self.control, &self.transport)
            .await
    }

    pub(crate) async fn handle(&mut self, peer: &str, data: &[u8]) -> Result<()> {
        self.exchange
            .handle(peer, data, &self.control, &self.transport)
            .await
    }

    pub(crate) async fn shutdown(&mut self) {
        self.read_route.set_routes(Vec::new());
        let _ = self.control.shutdown().await;
        let _ = self.endpoint.shutdown().await;
    }
}
