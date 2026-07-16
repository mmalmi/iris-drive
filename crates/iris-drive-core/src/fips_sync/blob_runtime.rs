//! Drive's one adaptive Hashtree read router over its existing FIPS endpoint.
//!
//! Each source remains opaque to the router. FIPS exclusively selects peers
//! inside one discovered-and-authorized provider-set route; Drive's configured
//! store remains the only cache and write target.

use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use fips_core::{FipsEndpoint, PeerIdentity};
use hashtree_config::StorageBackend;
use hashtree_core::{BlobRoute, Store, StoreBlobRoute};
use hashtree_fips_transport::{
    FipsBlobRoute, FipsPeerConfig, InboundBlobPolicy, TcpBlobTransport, TcpBlobTransportConfig,
};
use hashtree_lmdb::open_shared_lmdb_blob_store;
use hashtree_network::{BlobRouteEntry, BlobRouter, BlobRouterConfig};

use super::{DRIVE_BLOB_SEARCH_TIMEOUT, FipsSyncError};

pub(super) const LOCAL_ROUTE_ID: &str = "iris-drive.configured-store";
const SHARED_LMDB_ROUTE_ID: &str = "hashtree.shared-lmdb";
const FIPS_ROUTE_ID: &str = "fips.blob-providers";
const MAX_PROVIDER_ATTEMPTS: usize = 4;
const DRIVE_BLOB_SERVICE_PRIORITY: i16 = 100;

pub(super) struct DriveBlobRuntime<L: Store + Send + Sync + 'static> {
    pub(super) router: Arc<BlobRouter>,
    _transport: Arc<TcpBlobTransport<L>>,
    fips: Arc<FipsBlobRoute<L>>,
    authorized_inbound: Arc<RwLock<Vec<PeerIdentity>>>,
}

impl<L: Store + Send + Sync + 'static> DriveBlobRuntime<L> {
    pub(super) async fn bind(
        endpoint: Arc<FipsEndpoint>,
        local_store: Arc<L>,
        peers: &[FipsPeerConfig],
        shared_store: Option<Arc<dyn BlobRoute>>,
    ) -> Result<Self, FipsSyncError> {
        let mut route_entries = vec![BlobRouteEntry::new(
            LOCAL_ROUTE_ID,
            Arc::new(StoreBlobRoute::new(local_store.clone())),
        )];
        if let Some(shared_store) = shared_store {
            route_entries.push(BlobRouteEntry::new(SHARED_LMDB_ROUTE_ID, shared_store));
        }
        let cache: Arc<dyn Store> = local_store.clone();
        let router = Arc::new(
            BlobRouter::new(
                route_entries.clone(),
                Some(cache),
                drive_blob_router_config(),
            )
            .map_err(|error| FipsSyncError::Endpoint(error.to_string()))?,
        );
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
        let route: Arc<dyn BlobRoute> = router.clone();
        let transport = Arc::new(
            TcpBlobTransport::bind_advertised_route_with_config_and_policy(
                endpoint.clone(),
                local_store,
                route,
                TcpBlobTransportConfig::default(),
                DRIVE_BLOB_SERVICE_PRIORITY,
                inbound_policy,
            )
            .await
            .map_err(|error| FipsSyncError::Endpoint(error.to_string()))?,
        );
        let fips = Arc::new(
            FipsBlobRoute::discovered_and_explicit(
                endpoint,
                transport.clone(),
                Vec::new(),
                MAX_PROVIDER_ATTEMPTS,
            )
            .map_err(|error| FipsSyncError::Endpoint(error.to_string()))?,
        );
        route_entries.push(BlobRouteEntry::new(FIPS_ROUTE_ID, fips.clone()));
        router
            .set_routes(route_entries)
            .await
            .map_err(|error| FipsSyncError::Endpoint(error.to_string()))?;

        let runtime = Self {
            router,
            _transport: transport,
            fips,
            authorized_inbound,
        };
        runtime.set_authorized_peers(peers);
        Ok(runtime)
    }

    pub(super) fn set_authorized_peers(&self, peers: &[FipsPeerConfig]) {
        let identities = peer_identities(peers);
        self.authorized_inbound
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone_from(&identities);
        self.fips.set_explicit_peers(identities);
    }

    pub(super) fn same_host_provider_ids(&self) -> Vec<String> {
        self.fips.discovered_provider_ids().unwrap_or_default()
    }
}

fn peer_identities(peers: &[FipsPeerConfig]) -> Vec<PeerIdentity> {
    let mut identities = Vec::new();
    for peer in peers {
        let Ok(identity) = PeerIdentity::from_npub(&peer.npub) else {
            continue;
        };
        if !identities.contains(&identity) {
            identities.push(identity);
        }
    }
    identities
}

fn drive_blob_router_config() -> BlobRouterConfig {
    BlobRouterConfig {
        request_timeout: DRIVE_BLOB_SEARCH_TIMEOUT,
        max_routes: 3,
        max_route_attempts: MAX_PROVIDER_ATTEMPTS,
        route_attempt_budget: MAX_PROVIDER_ATTEMPTS,
        ..BlobRouterConfig::default()
    }
}

pub(super) fn configured_shared_lmdb_route() -> Result<Option<Arc<dyn BlobRoute>>, FipsSyncError> {
    let config = hashtree_config::Config::load()
        .map_err(|error| FipsSyncError::SharedStore(error.to_string()))?;
    if config.storage.backend == StorageBackend::Fs {
        return Ok(None);
    }
    let data_dir = std::env::var_os("HTREE_DATA_DIR")
        .map_or_else(|| PathBuf::from(config.storage.data_dir), PathBuf::from);
    let storage_budget_bytes = config
        .storage
        .max_size_gb
        .saturating_mul(1024 * 1024 * 1024);
    let store = Arc::new(
        open_shared_lmdb_blob_store(data_dir, storage_budget_bytes)
            .map_err(|error| FipsSyncError::SharedStore(error.to_string()))?,
    );
    Ok(Some(Arc::new(StoreBlobRoute::new(store))))
}
