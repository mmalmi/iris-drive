use super::*;
use async_trait::async_trait;
use fips_core::PeerIdentity;
use hashtree_core::{
    BLOB_DEFAULT_HTL, BlobReply, BlobRequest, BlobRoute, Hash, HashTree, HashTreeConfig, StoreError,
};
use hashtree_fips_transport::{
    FipsEndpointOptions, SameHostBlobStore, TcpBlobTransport, TcpBlobTransportConfig,
    bind_fips_endpoint_at_local_rendezvous,
};
use hashtree_network::{MeshReadSource, MeshRoutingConfig, NamedBlobRoute, blob_resolver};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};

use super::super::settings_runtime::FIPS_PACKET_CHANNEL_CAPACITY;

#[derive(Clone, Copy)]
enum ProviderOutcome {
    NoResult,
    Failure,
}

struct RecordingProviderRoute {
    outcome: ProviderOutcome,
    request_htl: AtomicU8,
}

#[async_trait]
impl BlobRoute for RecordingProviderRoute {
    async fn route(&self, request: BlobRequest) -> Result<BlobReply, StoreError> {
        self.request_htl.store(request.htl, Ordering::Relaxed);
        match self.outcome {
            ProviderOutcome::NoResult => Ok(BlobReply::NoResult),
            ProviderOutcome::Failure => Err(StoreError::Other(
                "deliberate same-host route failure".to_string(),
            )),
        }
    }
}

struct RecordingStandaloneRoute {
    hash: Hash,
    data: Vec<u8>,
    request_htl: AtomicU8,
}

#[async_trait]
impl BlobRoute for RecordingStandaloneRoute {
    async fn route(&self, request: BlobRequest) -> Result<BlobReply, StoreError> {
        self.request_htl.store(request.htl, Ordering::Relaxed);
        if request.hash == self.hash {
            Ok(BlobReply::Data(self.data.clone()))
        } else {
            Ok(BlobReply::NoResult)
        }
    }
}

#[tokio::test]
async fn same_host_no_result_uses_nonzero_htl_then_falls_through_to_standalone() {
    assert_same_host_route_falls_through(ProviderOutcome::NoResult).await;
}

#[tokio::test]
async fn same_host_failure_uses_nonzero_htl_then_falls_through_to_standalone() {
    assert_same_host_route_falls_through(ProviderOutcome::Failure).await;
}

#[tokio::test]
async fn same_host_provider_can_continue_over_remote_mesh_with_one_htl_decrement() {
    let remote_device = AppKey::generate("remote-mesh-route-test");
    let provider_device = AppKey::generate("local-provider-mesh-route-test");
    let target_device = AppKey::generate("drive-target-mesh-route-test");
    let rendezvous_addr = reserve_udp_address();
    let remote_bound =
        bind_local_test_endpoint(&remote_device, "remote-mesh-route-test", rendezvous_addr).await;
    let provider_bound = bind_local_test_endpoint(
        &provider_device,
        "local-provider-mesh-route-test",
        rendezvous_addr,
    )
    .await;
    let target_bound = bind_local_test_endpoint(
        &target_device,
        "drive-target-mesh-route-test",
        rendezvous_addr,
    )
    .await;

    let data = b"Drive to local htree to remote FIPS mesh".to_vec();
    let hash = hashtree_core::sha256(&data);
    let remote_route = Arc::new(RecordingStandaloneRoute {
        hash,
        data: data.clone(),
        request_htl: AtomicU8::new(0),
    });
    let remote_transport = TcpBlobTransport::bind_route_with_config(
        remote_bound.native_endpoint.clone(),
        Arc::new(MemoryStore::new()),
        remote_route.clone(),
        TcpBlobTransportConfig::default(),
    )
    .await
    .unwrap();

    let provider_local = Arc::new(MemoryStore::new());
    let provider_resolver = Arc::new(blob_resolver(
        provider_local.clone(),
        provider_device.pubkey_bech32(),
        Duration::from_secs(2),
        MeshRoutingConfig::default(),
    ));
    let provider_transport = Arc::new(
        TcpBlobTransport::bind_advertised_route_with_config(
            provider_bound.native_endpoint.clone(),
            provider_local.clone(),
            provider_resolver.clone(),
            TcpBlobTransportConfig::default(),
            100,
        )
        .await
        .unwrap(),
    );
    let remote_peer = PeerIdentity::from_npub(&remote_device.pubkey_bech32()).unwrap();
    provider_resolver
        .set_read_sources(vec![Arc::new(NamedBlobRoute::mesh_peer(
            remote_device.pubkey_bech32(),
            Arc::new(provider_transport.weak_route_to(remote_peer)),
        )) as Arc<dyn MeshReadSource>])
        .await;

    wait_for_blob_provider(&target_bound, &provider_device.pubkey_bech32()).await;
    let target_local = Arc::new(MemoryStore::new());
    let target_store = SameHostBlobStore::bind(
        target_bound.native_endpoint.clone(),
        target_local.clone(),
        None,
        drive_same_host_blob_store_config(),
    )
    .await
    .unwrap();

    assert_eq!(target_store.get(&hash).await.unwrap(), Some(data.clone()));
    assert_eq!(target_local.get(&hash).await.unwrap(), Some(data.clone()));
    assert_eq!(provider_local.get(&hash).await.unwrap(), Some(data));
    assert_eq!(
        remote_route.request_htl.load(Ordering::Relaxed),
        BLOB_DEFAULT_HTL - 1,
    );

    drop(target_store);
    drop(provider_resolver);
    drop(provider_transport);
    drop(remote_transport);
    target_bound.native_endpoint.shutdown().await.unwrap();
    provider_bound.native_endpoint.shutdown().await.unwrap();
    remote_bound.native_endpoint.shutdown().await.unwrap();
}

#[tokio::test]
async fn drive_blob_service_accepts_roster_peer_and_rejects_unrelated_identity() {
    let drive_device = AppKey::generate("drive-inbound-policy-test");
    let roster_device = AppKey::generate("drive-roster-reader-test");
    let unrelated_device = AppKey::generate("drive-unrelated-reader-test");
    let rendezvous_addr = reserve_udp_address();
    let drive_bound =
        bind_local_test_endpoint(&drive_device, "drive-inbound-policy-test", rendezvous_addr).await;
    let roster_bound =
        bind_local_test_endpoint(&roster_device, "drive-roster-reader-test", rendezvous_addr).await;
    let unrelated_bound = bind_local_test_endpoint(
        &unrelated_device,
        "drive-unrelated-reader-test",
        rendezvous_addr,
    )
    .await;
    let data = b"only an authorized Iris Drive AppKey may read this blob".to_vec();
    let hash = hashtree_core::sha256(&data);
    let drive_local = Arc::new(MemoryStore::new());
    drive_local.put(hash, data.clone()).await.unwrap();
    let drive = DriveBlobRuntime::bind(
        drive_bound.native_endpoint.clone(),
        drive_local,
        drive_device.pubkey_bech32(),
        &[FipsPeerConfig::new(roster_device.pubkey_bech32())],
    )
    .await
    .unwrap();
    let roster_reader = TcpBlobTransport::bind(
        roster_bound.native_endpoint.clone(),
        Arc::new(MemoryStore::new()),
    )
    .await
    .unwrap();
    let unrelated_reader = TcpBlobTransport::bind(
        unrelated_bound.native_endpoint.clone(),
        Arc::new(MemoryStore::new()),
    )
    .await
    .unwrap();
    wait_for_peer_connection(&roster_bound, &drive_device.pubkey_bech32()).await;
    wait_for_peer_connection(&unrelated_bound, &drive_device.pubkey_bech32()).await;
    let drive_identity = PeerIdentity::from_npub(&drive_device.pubkey_bech32()).unwrap();

    assert_eq!(
        roster_reader
            .fetch_from_peer(&hash, drive_identity)
            .await
            .unwrap(),
        Some(data),
    );
    assert!(
        unrelated_reader
            .fetch_from_peer(&hash, drive_identity)
            .await
            .is_err(),
        "an unrelated authenticated identity reached Drive's blob resolver",
    );

    unrelated_reader.shutdown().await.unwrap();
    roster_reader.shutdown().await.unwrap();
    drop(drive);
    unrelated_bound.native_endpoint.shutdown().await.unwrap();
    roster_bound.native_endpoint.shutdown().await.unwrap();
    drive_bound.native_endpoint.shutdown().await.unwrap();
}

async fn assert_same_host_route_falls_through(outcome: ProviderOutcome) {
    let provider_device = AppKey::generate("provider-route-fallback-test");
    let target_device = AppKey::generate("target-route-fallback-test");
    let rendezvous_addr = reserve_udp_address();
    let provider_bound = bind_local_test_endpoint(
        &provider_device,
        "provider-route-fallback-test",
        rendezvous_addr,
    )
    .await;
    let target_bound = bind_local_test_endpoint(
        &target_device,
        "target-route-fallback-test",
        rendezvous_addr,
    )
    .await;

    let provider_route = Arc::new(RecordingProviderRoute {
        outcome,
        request_htl: AtomicU8::new(0),
    });
    let provider_transport = TcpBlobTransport::bind_advertised_route_with_config(
        provider_bound.native_endpoint.clone(),
        Arc::new(MemoryStore::new()),
        provider_route.clone(),
        TcpBlobTransportConfig::default(),
        100,
    )
    .await
    .unwrap();

    wait_for_blob_provider(&target_bound, &provider_device.pubkey_bech32()).await;

    let data = b"standalone route remains available after a local provider miss".to_vec();
    let hash = hashtree_core::sha256(&data);
    let standalone = Arc::new(RecordingStandaloneRoute {
        hash,
        data: data.clone(),
        request_htl: AtomicU8::new(0),
    });
    let local = Arc::new(MemoryStore::new());
    let store = SameHostBlobStore::bind(
        target_bound.native_endpoint.clone(),
        local.clone(),
        Some(standalone.clone()),
        drive_same_host_blob_store_config(),
    )
    .await
    .unwrap();

    assert_eq!(store.get(&hash).await.unwrap(), Some(data.clone()));
    assert_eq!(local.get(&hash).await.unwrap(), Some(data));
    assert_eq!(
        provider_route.request_htl.load(Ordering::Relaxed),
        BLOB_DEFAULT_HTL
    );
    assert_eq!(
        standalone.request_htl.load(Ordering::Relaxed),
        BLOB_DEFAULT_HTL
    );

    drop(store);
    drop(provider_transport);
    target_bound.native_endpoint.shutdown().await.unwrap();
    provider_bound.native_endpoint.shutdown().await.unwrap();
}

async fn wait_for_blob_provider(endpoint: &BoundFipsEndpoint, npub: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if endpoint
                .native_endpoint
                .local_instance_advertisements()
                .unwrap()
                .iter()
                .any(|advert| {
                    advert.npub == npub
                        && advert
                            .capability(hashtree_fips_transport::TCP_BLOB_CAPABILITY)
                            .is_some()
                })
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("same-host blob provider did not become visible");
}

pub(super) async fn wait_for_peer_connection(endpoint: &BoundFipsEndpoint, npub: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if endpoint
                .native_endpoint
                .peers()
                .await
                .unwrap()
                .iter()
                .any(|peer| peer.npub == npub && peer.connected)
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("local FIPS peers did not connect");
}

#[tokio::test]
async fn real_same_host_provider_failure_still_uses_drive_standalone_route() {
    let source_device = AppKey::generate("source-fips-test-key");
    let target_device = AppKey::generate("target-fips-test-key");
    let provider_device = AppKey::generate("provider-fips-test-key");
    let config = authorized_pair_config(&target_device, &source_device);
    let rendezvous_addr = reserve_udp_address();
    let source_udp_addr = reserve_udp_address();
    let target_udp_addr = reserve_udp_address();
    let provider_udp_addr = reserve_udp_address();
    let source_bound = Box::pin(bind_test_endpoint(
        &source_device,
        "drive-source-fallback-test",
        rendezvous_addr,
        source_udp_addr,
        false,
    ))
    .await
    .unwrap();
    let provider_bound = Box::pin(bind_test_endpoint(
        &provider_device,
        "failing-provider-fallback-test",
        rendezvous_addr,
        provider_udp_addr,
        true,
    ))
    .await
    .unwrap();
    let target_bound = Box::pin(bind_test_endpoint(
        &target_device,
        &discovery_scope(&config),
        rendezvous_addr,
        target_udp_addr,
        true,
    ))
    .await
    .unwrap();
    let source_native = source_bound.native_endpoint.clone();
    let provider_native = provider_bound.native_endpoint.clone();
    let target_native = target_bound.native_endpoint.clone();

    let source_store = Arc::new(MemoryStore::new());
    let source_tree = HashTree::new(HashTreeConfig::new(source_store.clone()));
    let file_bytes = (0_u8..=250).cycle().take(192 * 1024).collect::<Vec<_>>();
    let (file_cid, _) = source_tree.put(&file_bytes).await.unwrap();
    let root_cid = source_tree
        .put_directory(vec![DirEntry {
            name: "fallback.txt".to_string(),
            hash: file_cid.hash,
            key: file_cid.key,
            link_type: LinkType::File,
            size: file_bytes.len() as u64,
            meta: None,
        }])
        .await
        .unwrap();
    let source_transport = TcpBlobTransport::bind(source_bound.native_endpoint, source_store)
        .await
        .unwrap();

    let provider_gets = Arc::new(AtomicUsize::new(0));
    let provider = SameHostBlobStore::bind(
        provider_bound.native_endpoint,
        Arc::new(FailingSameHostStore {
            gets: provider_gets.clone(),
        }),
        None,
        SameHostBlobStoreConfig::provider(100),
    )
    .await
    .unwrap();
    let target_store = Arc::new(MemoryStore::new());
    let sync = FipsBlockSync::start_with_bound_endpoint(
        target_bound,
        target_store.clone(),
        &config,
        local_only_settings(&source_device, source_udp_addr, target_udp_addr),
    )
    .await
    .unwrap();
    set_fips_peer_configs(
        source_native.as_ref(),
        vec![FipsPeerConfig {
            npub: target_device.pubkey_bech32(),
            udp_addresses: vec![target_udp_addr.to_string()],
        }],
    )
    .await
    .unwrap();

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let advertised = target_native
                .local_instance_advertisements()
                .unwrap()
                .iter()
                .any(|advert| {
                    advert.npub == provider_device.pubkey_bech32()
                        && advert
                            .capability(hashtree_fips_transport::TCP_BLOB_CAPABILITY)
                            .is_some()
                });
            if advertised {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("failing same-host provider did not become visible");
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if target_native
                .peers()
                .await
                .unwrap()
                .iter()
                .any(|peer| peer.npub == source_device.pubkey_bech32() && peer.connected)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("Drive standalone FIPS route did not connect");

    let report = tokio::time::timeout(Duration::from_secs(15), sync.download_tree(&root_cid))
        .await
        .expect("Drive fallback retrieval timed out")
        .unwrap();

    assert_eq!(report.fetched, 2);
    assert_eq!(report.already_local, 0);
    assert!(target_store.has(&root_cid.hash).await.unwrap());
    assert!(target_store.has(&file_cid.hash).await.unwrap());
    assert!(provider_gets.load(Ordering::Relaxed) > 0);
    assert_eq!(
        sync.authorized_peer_ids(),
        vec![source_device.pubkey_bech32()]
    );
    let source_status = sync
        .fips_peer_statuses()
        .await
        .into_iter()
        .find(|status| status.npub == source_device.pubkey_bech32())
        .expect("application-owned source peer disappeared after provider failure");
    assert_eq!(source_status.transport_type.as_deref(), Some("udp"));
    assert_eq!(
        source_status
            .transport_addr
            .as_deref()
            .and_then(|addr| addr.parse::<SocketAddr>().ok()),
        Some(SocketAddr::V4(source_udp_addr))
    );

    sync.shutdown().await.unwrap();
    drop(provider);
    source_transport.shutdown().await.unwrap();
    source_native.shutdown().await.unwrap();
    provider_native.shutdown().await.unwrap();
}

pub(super) async fn bind_test_endpoint(
    device: &AppKey,
    scope: &str,
    rendezvous_addr: SocketAddrV4,
    udp_bind_addr: SocketAddrV4,
    enable_local_rendezvous: bool,
) -> Result<BoundFipsEndpoint, hashtree_fips_transport::FipsTransportError> {
    let identity_nsec = device.keys().secret_key().to_bech32().unwrap();
    Box::pin(bind_fips_endpoint_at_local_rendezvous(
        FipsEndpointOptions {
            identity_nsec,
            discovery_scope: scope.to_string(),
            relays: Vec::new(),
            enable_udp: true,
            enable_webrtc: false,
            enable_local_rendezvous,
            ethernet_interfaces: Vec::new(),
            enable_lan_discovery: false,
            udp_bind_addr: Some(udp_bind_addr.to_string()),
            udp_public: false,
            udp_external_addr: None,
            share_local_candidates: false,
            webrtc_auto_connect: false,
            webrtc_max_connections: 8,
            open_discovery_max_pending: 0,
            packet_channel_capacity: FIPS_PACKET_CHANNEL_CAPACITY,
        },
        rendezvous_addr,
    ))
    .await
}

async fn bind_local_test_endpoint(
    device: &AppKey,
    scope: &str,
    rendezvous_addr: SocketAddrV4,
) -> BoundFipsEndpoint {
    Box::pin(bind_test_endpoint(
        device,
        scope,
        rendezvous_addr,
        reserve_udp_address(),
        true,
    ))
    .await
    .unwrap()
}

fn local_only_settings(
    source: &AppKey,
    source_udp_addr: SocketAddrV4,
    target_udp_addr: SocketAddrV4,
) -> FipsTransportSettings {
    FipsTransportSettings {
        enable_udp: true,
        enable_webrtc: false,
        enable_lan_discovery: false,
        enable_mesh_pubsub: false,
        udp_bind_addr: Some(target_udp_addr.to_string()),
        udp_public: false,
        udp_external_addr: None,
        share_local_candidates: false,
        static_peer_hints: vec![(source.pubkey_bech32(), vec![source_udp_addr.to_string()])],
        bootstrap_peer_hints: Vec::new(),
        webrtc_max_connections: 1,
        open_discovery_max_pending: 0,
    }
}

pub(super) fn reserve_udp_address() -> SocketAddrV4 {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).expect("reserve loopback UDP port");
    let SocketAddr::V4(address) = socket.local_addr().expect("reserved loopback UDP address")
    else {
        unreachable!("IPv4 loopback bind returned IPv6");
    };
    address
}

fn authorized_pair_config(target: &AppKey, source: &AppKey) -> AppConfig {
    let profile_id = crate::NostrIdentityId::new_v4();
    let target_pubkey = target.pubkey_hex();
    AppConfig {
        profile: Some(crate::ProfileState {
            profile_id,
            app_key_pubkey: target_pubkey.clone(),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "test-link-secret".to_string(),
            authorization_state: crate::AppKeyAuthorizationState::Authorized,
            app_key_label: Some("target".to_string()),
            app_keys: Some(crate::app_keys::AppKeysProjection {
                profile_id: profile_id.to_string(),
                signed_by_pubkey: Some(target_pubkey.clone()),
                created_at: 1,
                app_actors: vec![
                    crate::app_keys::AppActorEntry::admin(
                        target_pubkey,
                        1,
                        Some("target".to_string()),
                    ),
                    crate::app_keys::AppActorEntry::member(
                        source.pubkey_hex(),
                        1,
                        Some("source".to_string()),
                    ),
                ],
                dck_generation: 0,
                wrapped_dck: std::collections::BTreeMap::new(),
            }),
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
            pending_device_approval_receipts: Vec::new(),
        }),
        relays: Vec::new(),
        ..AppConfig::default()
    }
}

struct FailingSameHostStore {
    gets: Arc<AtomicUsize>,
}

#[async_trait]
impl Store for FailingSameHostStore {
    async fn put(&self, _hash: Hash, _data: Vec<u8>) -> Result<bool, StoreError> {
        Ok(false)
    }

    async fn get(&self, _hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        self.gets.fetch_add(1, Ordering::Relaxed);
        Err(StoreError::Other(
            "deliberate same-host provider failure".to_string(),
        ))
    }

    async fn has(&self, _hash: &Hash) -> Result<bool, StoreError> {
        Ok(false)
    }

    async fn delete(&self, _hash: &Hash) -> Result<bool, StoreError> {
        Ok(false)
    }
}
