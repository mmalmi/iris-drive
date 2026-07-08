use super::*;
use hashtree_core::{DirEntry, LinkType, MemoryStore};
use hashtree_fips_transport::{FipsEndpointIo, FipsEndpointPacket, FipsTransportError};
use tokio::sync::{Mutex as TokioMutex, mpsc};

type PacketSenderMap =
    Arc<TokioMutex<std::collections::HashMap<String, mpsc::UnboundedSender<FipsEndpointPacket>>>>;
type PeerLinkMap = Arc<TokioMutex<std::collections::BTreeMap<String, Vec<String>>>>;

struct FakeEndpoint {
    id: String,
    network: PacketSenderMap,
    links: Option<PeerLinkMap>,
    rx: TokioMutex<mpsc::UnboundedReceiver<FipsEndpointPacket>>,
}

impl FakeEndpoint {
    async fn new(id: &str, network: PacketSenderMap) -> Arc<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        network.lock().await.insert(id.to_string(), tx);
        Arc::new(Self {
            id: id.to_string(),
            network,
            links: None,
            rx: TokioMutex::new(rx),
        })
    }

    async fn new_linked(id: &str, network: PacketSenderMap, links: PeerLinkMap) -> Arc<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        network.lock().await.insert(id.to_string(), tx);
        Arc::new(Self {
            id: id.to_string(),
            network,
            links: Some(links),
            rx: TokioMutex::new(rx),
        })
    }

    async fn visible_peers(&self) -> Vec<String> {
        if let Some(links) = self.links.as_ref() {
            return links
                .lock()
                .await
                .get(&self.id)
                .cloned()
                .unwrap_or_default();
        }
        self.network
            .lock()
            .await
            .keys()
            .filter(|id| *id != &self.id)
            .cloned()
            .collect()
    }
}

#[async_trait]
impl FipsEndpointIo for FakeEndpoint {
    async fn send(&self, peer_id: &str, data: Vec<u8>) -> Result<(), FipsTransportError> {
        if !self
            .visible_peers()
            .await
            .iter()
            .any(|peer| peer == peer_id)
        {
            return Err(FipsTransportError::Send(format!(
                "peer {peer_id} is not linked from {}",
                self.id
            )));
        }
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
        self.visible_peers().await
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

    let report = download_tree_with_transport(target_store.clone(), &root_cid, target_transport)
        .await
        .unwrap();

    assert_eq!(report.fetched, 2);
    assert_eq!(report.already_local, 0);
    assert!(target_store.has(&root_cid.hash).await.unwrap());
    assert!(target_store.has(&file_cid.hash).await.unwrap());

    source_task.abort();
    target_task.abort();
}

#[tokio::test]
async fn overlay_download_uses_direct_fips_when_mesh_peer_is_absent() {
    let network = Arc::new(TokioMutex::new(std::collections::HashMap::new()));
    let source_endpoint = FakeEndpoint::new("source", network.clone()).await;
    let target_endpoint = FakeEndpoint::new("target", network).await;

    let source_store = Arc::new(MemoryStore::new());
    let source_tree = HashTree::new(HashTreeConfig::new(source_store.clone()));
    let (file_cid, _) = source_tree.put(b"hello direct remote").await.unwrap();
    let root_cid = source_tree
        .put_directory(vec![DirEntry {
            name: "hello.txt".to_string(),
            hash: file_cid.hash,
            key: file_cid.key,
            link_type: LinkType::File,
            size: 21,
            meta: None,
        }])
        .await
        .unwrap();

    let source_transport = Arc::new(HashtreeFipsTransport::new(source_endpoint, source_store));
    let target_store = Arc::new(MemoryStore::new());
    let target_transport = Arc::new(HashtreeFipsTransport::new(
        target_endpoint,
        target_store.clone(),
    ));
    target_transport.set_peers(vec!["source".to_string()]).await;
    let source_task = source_transport.start();
    let target_task = target_transport.start();
    let target_mesh = Arc::new(
        target_transport
            .start_mesh_pubsub(
                target_store.clone(),
                "target".to_string(),
                Duration::from_millis(200),
            )
            .await
            .unwrap(),
    );

    let report = download_tree_with_overlay(
        target_store.clone(),
        &root_cid,
        target_transport,
        target_mesh,
    )
    .await
    .unwrap();

    assert_eq!(report.fetched, 2);
    assert_eq!(report.already_local, 0);
    assert!(target_store.has(&root_cid.hash).await.unwrap());
    assert!(target_store.has(&file_cid.hash).await.unwrap());

    source_task.abort();
    target_task.abort();
}

#[tokio::test]
async fn download_skips_unavailable_prev_history_target() {
    let network = Arc::new(TokioMutex::new(std::collections::HashMap::new()));
    let source_endpoint = FakeEndpoint::new("source", network.clone()).await;
    let target_endpoint = FakeEndpoint::new("target", network).await;

    let source_store = Arc::new(MemoryStore::new());
    let source_tree = HashTree::new(HashTreeConfig::new(source_store.clone()));
    let (file_cid, _) = source_tree.put(b"current visible bytes").await.unwrap();
    let visible_root = source_tree
        .put_directory(vec![DirEntry {
            name: "current.txt".to_string(),
            hash: file_cid.hash,
            key: file_cid.key,
            link_type: LinkType::File,
            size: 21,
            meta: None,
        }])
        .await
        .unwrap();
    let missing_prev = Cid {
        hash: [7; 32],
        key: None,
    };
    let root_with_history =
        crate::indexer::layer_prev_link(&source_tree, visible_root, &missing_prev)
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
        download_tree_with_transport(target_store.clone(), &root_with_history, target_transport)
            .await
            .unwrap();

    assert!(report.fetched >= 3);
    assert!(target_store.has(&root_with_history.hash).await.unwrap());
    assert!(target_store.has(&file_cid.hash).await.unwrap());
    assert!(!target_store.has(&missing_prev.hash).await.unwrap());

    source_task.abort();
    target_task.abort();
}

async fn wait_for_mesh_neighbors(mesh: &FipsMeshPubsub<MemoryStore>, expected: &[&str]) -> bool {
    for _ in 0..50 {
        let peers = mesh.peer_ids().await;
        if expected
            .iter()
            .all(|expected_peer| peers.iter().any(|peer| peer == expected_peer))
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

#[tokio::test]
async fn downloads_tree_blocks_over_indirect_fips_mesh_peer() {
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
    let (file_cid, _) = source_tree.put(b"hello through mesh").await.unwrap();
    let root_cid = source_tree
        .put_directory(vec![DirEntry {
            name: "hello.txt".to_string(),
            hash: file_cid.hash,
            key: file_cid.key,
            link_type: LinkType::File,
            size: 18,
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
    let source_mesh = Arc::new(
        source_transport
            .start_mesh_pubsub(
                source_store.clone(),
                "source".to_string(),
                Duration::from_secs(2),
            )
            .await
            .unwrap(),
    );
    let relay_mesh = Arc::new(
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
    assert!(wait_for_mesh_neighbors(&relay_mesh, &["source", "target"]).await);
    assert!(wait_for_mesh_neighbors(&source_mesh, &["relay"]).await);
    assert!(
        download_tree_with_transport(target_store.clone(), &root_cid, target_transport.clone())
            .await
            .is_err(),
        "raw FIPS transport should not fetch through an indirect relay"
    );

    let report = download_tree_with_overlay(
        target_store.clone(),
        &root_cid,
        target_transport,
        target_mesh,
    )
    .await
    .unwrap();

    assert_eq!(report.fetched, 2);
    assert_eq!(report.already_local, 0);
    assert!(target_store.has(&root_cid.hash).await.unwrap());
    assert!(target_store.has(&file_cid.hash).await.unwrap());

    source_task.abort();
    relay_task.abort();
    target_task.abort();
}

#[test]
fn discovery_scope_uses_iris_drive_overlay() {
    let config = AppConfig {
        profile: Some(crate::ProfileState {
            profile_id: crate::NostrIdentityId::new_v4(),
            app_key_pubkey: "bb".repeat(32),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".into(),
            authorization_state: crate::AppKeyAuthorizationState::AwaitingApproval,
            app_key_label: None,
            app_keys: None,
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
        }),
        ..Default::default()
    };

    assert_eq!(discovery_scope(&config), IRIS_DRIVE_FIPS_DISCOVERY_SCOPE);
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
        webrtc_max_connections: 12,
        open_discovery_max_pending: 8,
    };

    let options = fips_endpoint_options(
        "nsec1example".to_string(),
        IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string(),
        vec!["wss://relay.example".to_string()],
        &AppConfig::default(),
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
    assert_eq!(options.webrtc_max_connections, 12);
    assert_eq!(options.open_discovery_max_pending, 8);
}

#[test]
fn default_transport_settings_seed_fips_bootstrap_transit() {
    let settings = FipsTransportSettings::default();

    assert_eq!(settings.webrtc_max_connections, 16);
    assert_eq!(settings.open_discovery_max_pending, 0);
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
fn single_device_profile_does_not_route_to_bootstrap_fips_peers() {
    let current_pubkey = "dd".repeat(32);
    let profile_id = crate::NostrIdentityId::new_v4();
    let config = AppConfig {
        profile: Some(crate::ProfileState {
            profile_id,
            app_key_pubkey: current_pubkey.clone(),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".into(),
            authorization_state: crate::AppKeyAuthorizationState::Authorized,
            app_key_label: None,
            app_keys: Some(crate::app_keys::AppKeysProjection {
                profile_id: profile_id.to_string(),
                signed_by_pubkey: Some(current_pubkey.clone()),
                created_at: 1,
                app_actors: vec![crate::app_keys::AppActorEntry::admin(
                    current_pubkey,
                    1,
                    Some("iPhone".into()),
                )],
                dck_generation: 0,
                wrapped_dck: std::collections::BTreeMap::default(),
            }),
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
        }),
        ..Default::default()
    };

    assert!(authorized_device_fips_peers(&config, &FipsTransportSettings::default()).is_empty());
    assert!(routing_fips_peers(&config, &FipsTransportSettings::default()).is_empty());
}

#[test]
fn remote_authorized_device_keeps_bootstrap_fips_routing_peers() {
    let current_pubkey = "dd".repeat(32);
    let remote_pubkey = "ee".repeat(32);
    let profile_id = crate::NostrIdentityId::new_v4();
    let config = AppConfig {
        profile: Some(crate::ProfileState {
            profile_id,
            app_key_pubkey: current_pubkey.clone(),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".into(),
            authorization_state: crate::AppKeyAuthorizationState::Authorized,
            app_key_label: None,
            app_keys: Some(crate::app_keys::AppKeysProjection {
                profile_id: profile_id.to_string(),
                signed_by_pubkey: Some(current_pubkey.clone()),
                created_at: 1,
                app_actors: vec![
                    crate::app_keys::AppActorEntry::admin(current_pubkey, 1, Some("Mac".into())),
                    crate::app_keys::AppActorEntry::member(remote_pubkey, 1, Some("iPhone".into())),
                ],
                dck_generation: 0,
                wrapped_dck: std::collections::BTreeMap::default(),
            }),
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
        }),
        ..Default::default()
    };

    assert_eq!(
        routing_fips_peers(&config, &FipsTransportSettings::default()).len(),
        DEFAULT_FIPS_BOOTSTRAP_PEERS.len()
    );
}

#[test]
fn fips_peer_config_snapshot_matches_endpoint_peer_sanitizing() {
    let snapshot = fips_peer_config_snapshot(
        Some("local"),
        &[
            FipsPeerConfig {
                npub: " remote ".to_string(),
                udp_addresses: vec![
                    " 10.44.1.2:22121 ".to_string(),
                    " ".to_string(),
                    "udp:10.44.1.3:22121".to_string(),
                ],
            },
            FipsPeerConfig {
                npub: "local".to_string(),
                udp_addresses: vec!["10.44.1.1:22121".to_string()],
            },
            FipsPeerConfig {
                npub: "remote".to_string(),
                udp_addresses: vec!["10.44.1.4:22121".to_string()],
            },
            FipsPeerConfig {
                npub: " ".to_string(),
                udp_addresses: vec!["10.44.1.5:22121".to_string()],
            },
        ],
        &[
            FipsPeerConfig {
                npub: "remote".to_string(),
                udp_addresses: vec!["10.44.1.6:22121".to_string()],
            },
            FipsPeerConfig {
                npub: " bootstrap ".to_string(),
                udp_addresses: vec![" udp:203.0.113.7:2121 ".to_string()],
            },
        ],
    );

    assert_eq!(
        snapshot.application_peers,
        vec![FipsPeerConfig {
            npub: "remote".to_string(),
            udp_addresses: vec![
                "10.44.1.2:22121".to_string(),
                "udp:10.44.1.3:22121".to_string()
            ],
        }]
    );
    assert_eq!(
        snapshot.routing_peers,
        vec![FipsPeerConfig {
            npub: "bootstrap".to_string(),
            udp_addresses: vec!["udp:203.0.113.7:2121".to_string()],
        }]
    );
}

#[test]
fn legacy_drive_roots_keep_bootstrap_fips_routing_peers() {
    let current_keys = nostr_sdk::Keys::generate();
    let remote_keys = nostr_sdk::Keys::generate();
    let current_pubkey = current_keys.public_key().to_hex();
    let remote_pubkey = remote_keys.public_key().to_hex();
    let profile_id = crate::NostrIdentityId::new_v4();
    let mut drive = crate::Drive::primary(profile_id.to_string());
    drive.app_key_roots.insert(
        current_pubkey.clone(),
        crate::AppKeyRootRef::legacy("current-root", 10, 1),
    );
    drive.app_key_roots.insert(
        remote_pubkey.clone(),
        crate::AppKeyRootRef::legacy("remote-root", 11, 1),
    );
    let config = AppConfig {
        profile: Some(crate::ProfileState {
            profile_id,
            app_key_pubkey: current_pubkey,
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".into(),
            authorization_state: crate::AppKeyAuthorizationState::Authorized,
            app_key_label: None,
            app_keys: None,
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
        }),
        drives: vec![drive],
        ..Default::default()
    };

    let remote_npub = remote_keys.public_key().to_bech32().unwrap();
    let authorized = authorized_device_fips_peers(&config, &FipsTransportSettings::default());
    assert_eq!(authorized.len(), 1);
    assert_eq!(authorized[0].npub, remote_npub);
    assert_eq!(
        routing_fips_peers(&config, &FipsTransportSettings::default()).len(),
        DEFAULT_FIPS_BOOTSTRAP_PEERS.len()
    );
}

#[test]
fn static_peer_hints_match_authorized_devices_by_label_or_npub() {
    let first_keys = nostr_sdk::Keys::generate();
    let second_keys = nostr_sdk::Keys::generate();
    let first_pubkey = first_keys.public_key().to_hex();
    let second_pubkey = second_keys.public_key().to_hex();
    let first_npub = first_keys.public_key().to_bech32().unwrap();
    let profile_id = crate::NostrIdentityId::new_v4();
    let settings = FipsTransportSettings {
        static_peer_hints: parse_static_peer_hints(&format!(
            "linux-peer=10.44.214.2:22121,{first_npub}=10.44.34.102:22121"
        )),
        ..Default::default()
    };
    let config = AppConfig {
        profile: Some(crate::ProfileState {
            profile_id,
            app_key_pubkey: "dd".repeat(32),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".into(),
            authorization_state: crate::AppKeyAuthorizationState::Authorized,
            app_key_label: None,
            app_keys: Some(crate::app_keys::AppKeysProjection {
                profile_id: profile_id.to_string(),
                signed_by_pubkey: Some("dd".repeat(32)),
                created_at: 1,
                app_actors: vec![
                    crate::app_keys::AppActorEntry::member(
                        first_pubkey,
                        1,
                        Some("macos-peer".into()),
                    ),
                    crate::app_keys::AppActorEntry::member(
                        second_pubkey,
                        1,
                        Some("linux-peer".into()),
                    ),
                ],
                dck_generation: 0,
                wrapped_dck: std::collections::BTreeMap::default(),
            }),
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
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
fn pending_app_key_link_admin_is_allowed_for_roster_app_messages() {
    let admin_keys = nostr_sdk::Keys::generate();
    let admin_pubkey = admin_keys.public_key().to_hex();
    let admin_npub = admin_keys.public_key().to_bech32().unwrap();
    let settings = FipsTransportSettings {
        bootstrap_peer_hints: Vec::new(),
        static_peer_hints: parse_static_peer_hints(&format!("{admin_npub}=10.44.1.9:22121")),
        ..Default::default()
    };
    let config = AppConfig {
        profile: Some(crate::ProfileState {
            profile_id: crate::NostrIdentityId::new_v4(),
            app_key_pubkey: "dd".repeat(32),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".into(),
            authorization_state: crate::AppKeyAuthorizationState::AwaitingApproval,
            app_key_label: None,
            app_keys: None,
            profile_roster_projection: None,
            outbound_app_key_link_request: Some(crate::profile::PendingAppKeyLinkRequest {
                admin_app_key_pubkey: admin_pubkey,
                invite_pubkey: "ee".repeat(32),
                request_url: String::new(),
                requested_at: 42,
            }),
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
        }),
        ..Default::default()
    };

    let authorized = authorized_device_fips_peers(&config, &settings);
    assert_eq!(authorized.len(), 1);
    assert_eq!(authorized[0].npub, admin_npub);
    assert_eq!(authorized[0].udp_addresses, vec!["10.44.1.9:22121"]);
    let routing = routing_fips_peers(&config, &settings);
    assert_eq!(routing.len(), 1);
    assert_eq!(routing[0].npub, admin_npub);
    assert_eq!(routing[0].udp_addresses, vec!["10.44.1.9:22121"]);
}

#[test]
fn endpoint_options_keep_native_udp_private_by_default() {
    let settings = FipsTransportSettings::default();

    let options = fips_endpoint_options(
        "nsec1example".to_string(),
        IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string(),
        Vec::new(),
        &AppConfig::default(),
        &settings,
    );

    assert!(options.enable_udp);
    assert!(options.enable_webrtc);
    assert!(!options.udp_public);
    assert!(options.udp_bind_addr.is_none());
    assert!(options.udp_external_addr.is_none());
}

#[test]
fn endpoint_options_keep_packet_queue_large_enough_for_root_bursts() {
    let settings = FipsTransportSettings::default();

    let options = fips_endpoint_options(
        "nsec1example".to_string(),
        IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string(),
        Vec::new(),
        &AppConfig::default(),
        &settings,
    );

    assert!(
        options.packet_channel_capacity >= 8192,
        "direct root hints share this queue with block traffic during file bursts"
    );
}

#[test]
fn admin_endpoint_options_allow_open_app_key_link_requests() {
    let settings = FipsTransportSettings::default();
    let dir = tempfile::tempdir().unwrap();
    let profile = crate::Profile::create(dir.path(), Some("admin".into())).unwrap();
    let config = AppConfig {
        profile: Some(profile.state),
        ..Default::default()
    };

    let options = fips_endpoint_options(
        "nsec1example".to_string(),
        IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string(),
        Vec::new(),
        &config,
        &settings,
    );

    assert_eq!(
        options.open_discovery_max_pending,
        APP_KEY_LINK_OPEN_DISCOVERY_MAX_PENDING
    );
}

#[test]
fn signed_control_topics_are_allowed_before_peer_is_configured() {
    assert_eq!(
        super::unconfigured_app_message_topics(),
        [
            crate::app_key_link_transport::APP_KEY_LINK_REQUEST_APP_TOPIC,
            crate::app_key_link_transport::APP_KEY_LINK_ROSTER_APP_TOPIC,
            crate::direct_root_transport::DIRECT_ROOT_APP_TOPIC,
        ]
    );
}

#[tokio::test]
async fn unconfigured_signed_control_app_messages_are_delivered() {
    let network = Arc::new(TokioMutex::new(std::collections::HashMap::new()));
    let admin_endpoint = FakeEndpoint::new("admin", network.clone()).await;
    let phone_endpoint = FakeEndpoint::new("phone", network).await;
    let admin_transport = Arc::new(HashtreeFipsTransport::new(
        admin_endpoint,
        Arc::new(MemoryStore::new()),
    ));
    let phone_transport = Arc::new(
        HashtreeFipsTransport::new(phone_endpoint, Arc::new(MemoryStore::new()))
            .with_unconfigured_app_message_topics(unconfigured_app_message_topics()),
    );
    phone_transport
        .set_peers(vec!["configured-but-not-admin".to_string()])
        .await;
    let mut app_messages = phone_transport.subscribe_app_messages();
    let admin_task = admin_transport.start();
    let phone_task = phone_transport.start();

    admin_transport
        .send_app_message(
            "phone",
            crate::app_key_link_transport::APP_KEY_LINK_ROSTER_APP_TOPIC,
            b"signed roster".to_vec(),
        )
        .await
        .unwrap();

    let message = tokio::time::timeout(Duration::from_millis(250), app_messages.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(message.peer_id, "admin");
    assert_eq!(
        message.topic,
        crate::app_key_link_transport::APP_KEY_LINK_ROSTER_APP_TOPIC
    );
    assert_eq!(message.data, b"signed roster");

    admin_transport
        .send_app_message(
            "phone",
            crate::direct_root_transport::DIRECT_ROOT_APP_TOPIC,
            b"signed direct root".to_vec(),
        )
        .await
        .unwrap();

    let message = tokio::time::timeout(Duration::from_millis(250), app_messages.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(message.peer_id, "admin");
    assert_eq!(
        message.topic,
        crate::direct_root_transport::DIRECT_ROOT_APP_TOPIC
    );
    assert_eq!(message.data, b"signed direct root");

    admin_task.abort();
    phone_task.abort();
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
