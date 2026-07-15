//! One canonical Hashtree resolver over Drive's existing FIPS endpoint.
//!
//! Same-host providers are an optimization. Authorized Drive `AppKeys` remain
//! independent mesh routes over the same owned TCP/FIPS blob transport.

use std::sync::{Arc, RwLock};

use fips_core::{FipsEndpoint, PeerIdentity};
use hashtree_core::Store;
use hashtree_fips_transport::{FipsPeerConfig, InboundBlobPolicy, SameHostBlobStore};
use hashtree_network::{
    BlobResolver, MeshReadSource, MeshRoutingConfig, NamedBlobRoute, blob_resolver,
};

use super::{FIPS_REQUEST_TIMEOUT, FipsSyncError, drive_same_host_blob_store_config};

pub(super) struct DriveBlobRuntime<L: Store + Send + Sync + 'static> {
    pub(super) store: Arc<SameHostBlobStore<L>>,
    resolver: Arc<BlobResolver<L>>,
    authorized_inbound: Arc<RwLock<Vec<PeerIdentity>>>,
}

impl<L: Store + Send + Sync + 'static> DriveBlobRuntime<L> {
    pub(super) async fn bind(
        endpoint: Arc<FipsEndpoint>,
        local_store: Arc<L>,
        local_peer_id: String,
        peers: &[FipsPeerConfig],
    ) -> Result<Self, FipsSyncError> {
        let resolver = Arc::new(blob_resolver(
            local_store.clone(),
            local_peer_id,
            FIPS_REQUEST_TIMEOUT,
            MeshRoutingConfig::default(),
        ));
        let authorized_inbound = Arc::new(RwLock::new(Vec::new()));
        let inbound_policy: InboundBlobPolicy = {
            let authorized_inbound = authorized_inbound.clone();
            Arc::new(move |peer| {
                authorized_inbound
                    .read()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .contains(&peer)
            })
        };
        let store = Arc::new(
            SameHostBlobStore::bind_route_with_policy(
                endpoint,
                local_store,
                None,
                resolver.clone(),
                inbound_policy,
                drive_same_host_blob_store_config(),
            )
            .await
            .map_err(|error| FipsSyncError::Endpoint(error.to_string()))?,
        );
        let runtime = Self {
            store,
            resolver,
            authorized_inbound,
        };
        runtime.set_authorized_peers(peers).await;
        runtime
            .store
            .set_standalone_route(Some(runtime.resolver.clone()));
        Ok(runtime)
    }

    pub(super) async fn set_authorized_peers(&self, peers: &[FipsPeerConfig]) {
        let mut identities = Vec::new();
        for peer in peers {
            let Ok(identity) = PeerIdentity::from_npub(&peer.npub) else {
                continue;
            };
            if !identities.contains(&identity) {
                identities.push(identity);
            }
        }
        let sources = identities
            .iter()
            .copied()
            .map(|peer| {
                Arc::new(NamedBlobRoute::mesh_peer(
                    peer.npub(),
                    Arc::new(self.store.weak_peer_route(peer)),
                )) as Arc<dyn MeshReadSource>
            })
            .collect();
        *self
            .authorized_inbound
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = identities;
        self.resolver.set_read_sources(sources).await;
    }
}
