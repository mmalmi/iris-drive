//! Reuse the running app's pubsub client, or bootstrap a temporary FIPS client.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use fips_core::FipsEndpoint;
use hashtree_fips_transport::{FipsPeerConfig, set_fips_peer_configs};
use nostr_pubsub::{
    EventPolicyContext, NostrEventHandler, NostrEventSubscriber, NostrEventSubscription,
    NostrPubsubRouter, PolicyDecision, PubsubPolicy, RouterLiveSource, SourcePolicyContext,
    SourceRoute,
};
use nostr_pubsub_fips::{FipsPubsubClient, FipsPubsubClientOptions, FipsPubsubPolicyOptions};
use nostr_pubsub_relay::RelayEventBus;
use nostr_sdk::{Filter, Keys, PublicKey, ToBech32};

use super::{ProductUpdateConfig, UPDATE_MANIFEST_TIMEOUT_SECS};
use crate::AppConfig;
use crate::fips_sync::FipsTransportSettings;
use crate::fips_sync::endpoint_config::bind_drive_fips_endpoint;
use crate::fips_sync::settings_runtime::fips_endpoint_options;

type SharedClients = HashMap<PathBuf, Weak<FipsPubsubClient>>;
static SHARED_CLIENTS: OnceLock<Mutex<SharedClients>> = OnceLock::new();

pub(crate) fn register(config_dir: &Path, client: &Arc<FipsPubsubClient>) {
    let mut clients = SHARED_CLIENTS
        .get_or_init(Mutex::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    clients.retain(|_, client| client.strong_count() > 0);
    clients.insert(config_dir.to_path_buf(), Arc::downgrade(client));
}

/// The existing provider owns its connections; only standalone checks own an
/// endpoint. Dropping the resolver also releases a cancelled check's resources.
pub(super) struct UpdatePubsub {
    router: NostrPubsubRouter,
    connections: UpdateConnections,
}

struct UpdateConnections {
    owned_fips: Option<(Arc<FipsPubsubClient>, Arc<FipsEndpoint>)>,
    relay: Option<RelayEventBus>,
}

impl UpdatePubsub {
    pub(super) async fn connect(config: &ProductUpdateConfig) -> Result<Arc<Self>> {
        let shared = config
            .config_dir
            .as_ref()
            .and_then(|directory| SHARED_CLIENTS.get()?.lock().ok()?.get(directory)?.upgrade());
        let settings = FipsTransportSettings::from_env();
        let (client, owned_fips) = if let Some(client) = shared {
            (Some(client), None)
        } else if settings.enable_mesh_pubsub {
            let settings = FipsTransportSettings {
                // A check has its own ephemeral identity and must not claim the
                // app's configured listening ports or change host networking.
                websocket_bind_addr: None,
                udp_bind_addr: Some("0.0.0.0:0".into()),
                ..settings
            };
            let endpoint = Box::pin(bind_drive_fips_endpoint(fips_endpoint_options(
                Keys::generate().secret_key().to_bech32()?,
                crate::fips_sync::IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.into(),
                Vec::new(),
                &AppConfig::default(),
                &settings,
            )))
            .await
            .context("starting update pubsub endpoint")?
            .native_endpoint;
            let peers: Vec<_> = settings
                .static_peer_hints
                .iter()
                .chain(&settings.bootstrap_peer_hints)
                .filter_map(|(key, addresses)| {
                    Some(FipsPeerConfig {
                        npub: PublicKey::parse(key).ok()?.to_bech32().ok()?,
                        udp_addresses: addresses.clone(),
                    })
                })
                .collect();
            set_fips_peer_configs(&endpoint, peers.clone()).await?;
            let mut policy = FipsPubsubPolicyOptions::default();
            policy.reputation.trusted_raters = settings.trusted_raters.into_iter().collect();
            let options = FipsPubsubClientOptions {
                routed_peers: peers.into_iter().map(|peer| peer.npub).collect(),
                ..Default::default()
            };
            let client =
                match FipsPubsubClient::start_with_reputation(endpoint.clone(), options, policy)
                    .await
                {
                    Ok(client) => Arc::new(client),
                    Err(error) => {
                        let _ = endpoint.shutdown().await;
                        return Err(error.into());
                    }
                };
            (Some(client.clone()), Some((client, endpoint)))
        } else {
            (None, None)
        };
        let mut runtime = Self {
            router: NostrPubsubRouter::new(Arc::new(ReleasePolicy)),
            connections: UpdateConnections {
                owned_fips,
                relay: None,
            },
        };
        if let Some(client) = &client {
            runtime.router = runtime.router.with_live_source(RouterLiveSource::new(
                SourceRoute::fips_peer_default("iris-drive-updates"),
                Arc::new(client.fresh_subscriber()),
            ));
        }
        // Honor the application's configured sources. An empty list stays
        // empty; the updater never substitutes a private default relay list.
        let relays = std::env::var("IRIS_DRIVE_UPDATE_RELAYS")
            .ok()
            .map(|value| hashtree_updater::split_csv(&value))
            .unwrap_or_else(|| config.relays.clone());
        if client.is_none() && relays.is_empty() {
            bail!("update discovery requires an enabled pubsub transport");
        }
        if !relays.is_empty() {
            let relay = RelayEventBus::new(
                relays,
                std::time::Duration::from_secs(UPDATE_MANIFEST_TIMEOUT_SECS),
            )
            .await?;
            runtime.router = runtime.router.with_live_source(RouterLiveSource::new(
                SourceRoute::relay("iris-drive-updates"),
                Arc::new(relay.clone()),
            ));
            runtime.connections.relay = Some(relay);
        }
        Ok(Arc::new(runtime))
    }
}

#[async_trait]
impl NostrEventSubscriber for UpdatePubsub {
    async fn subscribe(
        &self,
        filters: Vec<Filter>,
        handler: NostrEventHandler,
    ) -> nostr_pubsub::Result<Box<dyn NostrEventSubscription>> {
        self.router.subscribe(filters, handler).await
    }
}

impl Drop for UpdateConnections {
    fn drop(&mut self) {
        let owned = self.owned_fips.take();
        let relay = self.relay.take();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Some(relay) = relay {
                    relay.client().disconnect().await;
                }
                if let Some((client, endpoint)) = owned {
                    client.shutdown_shared().await;
                    let _ = endpoint.shutdown().await;
                }
            });
        }
    }
}

// The shared resolver validates the pinned publisher/tree and signatures.
// Source routing must not depend on personal follows or application ACLs.
struct ReleasePolicy;

#[async_trait]
impl PubsubPolicy for ReleasePolicy {
    async fn check_event(
        &self,
        context: EventPolicyContext<'_>,
    ) -> nostr_pubsub::Result<PolicyDecision> {
        Ok(PolicyDecision::allow_with_priority(
            context.source.kind.default_priority(),
        ))
    }
    async fn check_source(
        &self,
        context: SourcePolicyContext<'_>,
    ) -> nostr_pubsub::Result<PolicyDecision> {
        Ok(PolicyDecision::allow_with_priority(
            context.candidate.priority,
        ))
    }
}

#[cfg(test)]
mod tests;
