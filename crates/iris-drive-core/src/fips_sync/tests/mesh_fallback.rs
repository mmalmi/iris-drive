use super::*;
use fips_core::config::{RoutingMode, TransportInstances};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::sync::atomic::{AtomicUsize, Ordering};

struct NoResultBlobStore;

#[async_trait]
impl Store for NoResultBlobStore {
    async fn put(&self, _hash: Hash, _data: Vec<u8>) -> Result<bool, StoreError> {
        Ok(false)
    }

    async fn get(&self, _hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        Ok(None)
    }

    async fn has(&self, _hash: &Hash) -> Result<bool, StoreError> {
        Ok(false)
    }

    async fn delete(&self, _hash: &Hash) -> Result<bool, StoreError> {
        Ok(false)
    }
}

#[tokio::test]
async fn same_host_no_result_does_not_suppress_existing_mesh_route() {
    let network = Arc::new(TokioMutex::new(std::collections::HashMap::new()));
    let links = Arc::new(TokioMutex::new(std::collections::BTreeMap::from([
        ("target".to_string(), vec!["relay".to_string()]),
        (
            "relay".to_string(),
            vec!["target".to_string(), "source".to_string()],
        ),
        ("source".to_string(), vec!["relay".to_string()]),
    ])));
    let source_endpoint = FakeEndpoint::new_linked("source", network.clone(), links.clone()).await;
    let relay_endpoint = FakeEndpoint::new_linked("relay", network.clone(), links.clone()).await;
    let target_endpoint = FakeEndpoint::new_linked("target", network, links).await;

    let source_store = Arc::new(MemoryStore::new());
    let source_tree = HashTree::new(HashTreeConfig::new(source_store.clone()));
    let (file_cid, _) = source_tree.put(b"hello after direct miss").await.unwrap();
    let root_cid = source_tree
        .put_directory(vec![DirEntry {
            name: "hello.txt".to_string(),
            hash: file_cid.hash,
            key: file_cid.key,
            link_type: LinkType::File,
            size: 23,
            meta: None,
        }])
        .await
        .unwrap();

    let source_transport = Arc::new(HashtreeFipsTransport::new(
        source_endpoint,
        source_store.clone(),
    ));
    let relay_store = Arc::new(MemoryStore::new());
    let relay_transport = Arc::new(HashtreeFipsTransport::new(
        relay_endpoint,
        relay_store.clone(),
    ));
    let target_store = Arc::new(MemoryStore::new());
    let target_transport = Arc::new(HashtreeFipsTransport::new(
        target_endpoint,
        target_store.clone(),
    ));
    target_transport
        .set_peers(vec!["source".to_string(), "relay".to_string()])
        .await;

    let source_task = source_transport.start();
    let relay_task = relay_transport.start();
    let target_task = target_transport.start();
    let _source_mesh = Arc::new(
        source_transport
            .start_mesh_pubsub(
                source_store.clone(),
                "source".to_string(),
                Duration::from_secs(2),
            )
            .await
            .unwrap(),
    );
    let _relay_mesh = Arc::new(
        relay_transport
            .start_mesh_pubsub(relay_store, "relay".to_string(), Duration::from_secs(2))
            .await
            .unwrap(),
    );
    let target_mesh = Arc::new(
        target_transport
            .start_mesh_pubsub(
                target_store.clone(),
                "target".to_string(),
                Duration::from_secs(2),
            )
            .await
            .unwrap(),
    );

    assert!(wait_for_mesh_neighbors(&target_mesh, &["relay"]).await);
    let sync = FipsBlockSync {
        transport: target_transport,
        blob_store: Arc::new(NoResultBlobStore),
        local_store: target_store.clone(),
        receiver_task: None,
        mesh_pubsub: Some(target_mesh),
        endpoint_npub: "target".to_string(),
        discovery_scope: IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string(),
        transport_settings: FipsTransportSettings::default(),
        last_peer_config: Mutex::new(None),
    };

    let report = sync.download_tree(&root_cid).await.unwrap();

    assert_eq!(report.fetched, 2);
    assert_eq!(report.already_local, 0);
    assert!(target_store.has(&root_cid.hash).await.unwrap());
    assert!(target_store.has(&file_cid.hash).await.unwrap());

    source_task.abort();
    relay_task.abort();
    target_task.abort();
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
    let (file_cid, _) = source_tree
        .put(b"Drive standalone retrieval after a real same-host failure")
        .await
        .unwrap();
    let root_cid = source_tree
        .put_directory(vec![DirEntry {
            name: "fallback.txt".to_string(),
            hash: file_cid.hash,
            key: file_cid.key,
            link_type: LinkType::File,
            size: 57,
            meta: None,
        }])
        .await
        .unwrap();
    let source_transport = Arc::new(HashtreeFipsTransport::new(
        source_bound.endpoint,
        source_store,
    ));
    source_transport
        .set_peer_configs(vec![FipsPeerConfig {
            npub: target_device.pubkey_bech32(),
            udp_addresses: vec![target_udp_addr.to_string()],
        }])
        .await;
    let source_task = source_transport.start();

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
        sync.authorized_peer_ids().await,
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
    source_task.abort();
    let _ = source_task.await;
    source_native.shutdown().await.unwrap();
    provider_native.shutdown().await.unwrap();
}

async fn bind_test_endpoint(
    device: &AppKey,
    scope: &str,
    rendezvous_addr: SocketAddrV4,
    udp_bind_addr: SocketAddrV4,
    enable_local_rendezvous: bool,
) -> Result<BoundFipsEndpoint, fips_core::FipsEndpointError> {
    let identity_nsec = device.keys().secret_key().to_bech32().unwrap();
    let mut config = fips_core::Config::new();
    config.node.identity = fips_core::IdentityConfig {
        nsec: Some(identity_nsec),
        persistent: false,
    };
    config.node.routing.mode = RoutingMode::ReplyLearned;
    config.node.control.enabled = false;
    config.node.system_files_enabled = false;
    config.node.discovery.local.rendezvous_addr = rendezvous_addr;
    config.node.discovery.lan.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.node.discovery.nostr.advertise = false;
    config.tun.enabled = false;
    config.dns.enabled = false;
    config.transports.udp = TransportInstances::Single(fips_core::UdpConfig {
        bind_addr: Some(udp_bind_addr.to_string()),
        advertise_on_nostr: Some(false),
        public: Some(false),
        ..fips_core::UdpConfig::default()
    });
    let builder = fips_core::FipsEndpoint::builder()
        .config(config)
        .discovery_scope(scope)
        .without_system_tun();
    let builder = if enable_local_rendezvous {
        builder.local_rendezvous()
    } else {
        builder
    };
    let endpoint = Arc::new(Box::pin(builder.bind()).await?);
    Ok(BoundFipsEndpoint {
        endpoint: endpoint.clone(),
        native_endpoint: endpoint.clone(),
        local_peer_id: endpoint.npub().to_string(),
        discovery_scope: scope.to_string(),
    })
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

fn reserve_udp_address() -> SocketAddrV4 {
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
