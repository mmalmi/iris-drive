use super::*;
use std::time::Duration;

use fips_core::PeerIdentity;
use fips_tcp::{Config as TcpConfig, State};
use fips_tcp_endpoint::FipsTcpEndpoint;
use hashtree_core::Store;
use hashtree_fips_transport::{
    FipsPeerConfig, TcpBlobTransport, TcpBlobTransportConfig, set_fips_peer_configs,
};

use crate::DIRECT_ROOT_APP_TOPIC;
use crate::app_key_link_transport::APP_KEY_LINK_REQUEST_APP_TOPIC;

use super::super::control_runtime::{DRIVE_CONTROL_SERVICE_PORT, encode_record};
use super::control::{now_ms, wait_for_tcp_state};
use super::mesh_fallback::{
    bind_test_endpoint, local_only_settings, reserve_udp_address, wait_for_peer_connection,
};

#[tokio::test]
async fn authenticated_same_host_blob_provider_never_enters_drive_data_acl() {
    let admin_dir = tempfile::tempdir().unwrap();
    let admin = crate::Profile::create(admin_dir.path(), Some("admin".into())).unwrap();
    let ambient = AppKey::generate("ambient-same-host-hashtree");
    let config = AppConfig {
        profile: Some(admin.state.clone()),
        relays: Vec::new(),
        ..AppConfig::default()
    };
    let rendezvous = reserve_udp_address();
    let admin_addr = reserve_udp_address();
    let ambient_addr = reserve_udp_address();
    let admin_bound = bind_test_endpoint(
        &admin.app_key,
        &discovery_scope(&config),
        rendezvous,
        admin_addr,
        true,
    )
    .await
    .unwrap();
    let ambient_bound = bind_test_endpoint(
        &ambient,
        "ambient-same-host-hashtree",
        rendezvous,
        ambient_addr,
        true,
    )
    .await
    .unwrap();
    let ambient_endpoint = ambient_bound.native_endpoint.clone();
    let ambient_store = Arc::new(MemoryStore::new());
    let provider_data = b"ambient provider data is a valid outbound optimization".to_vec();
    let provider_hash = hashtree_core::sha256(&provider_data);
    ambient_store
        .put(provider_hash, provider_data.clone())
        .await
        .unwrap();
    let ambient_blobs = TcpBlobTransport::bind_advertised_with_config(
        ambient_endpoint.clone(),
        ambient_store,
        TcpBlobTransportConfig::default(),
        100,
    )
    .await
    .unwrap();
    let admin_store = Arc::new(MemoryStore::new());
    let sync = FipsBlockSync::start_with_bound_endpoint(
        admin_bound,
        admin_store,
        &config,
        local_only_settings(&ambient, ambient_addr, admin_addr),
        None,
    )
    .await
    .unwrap();

    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if sync
                .same_host_blob_provider_ids()
                .contains(&ambient.pubkey_bech32())
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("ambient same-host provider was not discovered");
    assert_eq!(
        sync.blob_router.get(&provider_hash, None).await.unwrap(),
        Some(provider_data),
        "same-host Hashtree reuse stopped working",
    );
    wait_for_peer_connection(&ambient_bound, &admin.app_key.pubkey_bech32()).await;
    sync.refresh_authorized_peers(&config).await;
    assert!(
        !sync
            .authorized_peer_ids()
            .contains(&ambient.pubkey_bech32()),
        "an ambient authenticated process was promoted to a Drive application peer",
    );

    let admin_identity = PeerIdentity::from_npub(&admin.app_key.pubkey_bech32()).unwrap();
    let mut deliveries = sync.subscribe_app_messages();
    let mut ambient_control = FipsTcpEndpoint::bind(
        ambient_endpoint.clone(),
        DRIVE_CONTROL_SERVICE_PORT,
        TcpConfig {
            max_connections: 2,
            max_connections_per_peer: 2,
            ..TcpConfig::default()
        },
        now_ms(),
    )
    .await
    .unwrap();
    let connection = ambient_control
        .connect(admin_identity, now_ms())
        .await
        .unwrap();
    wait_for_tcp_state(
        &mut ambient_control,
        connection,
        Some(State::Established),
        Duration::from_secs(5),
    )
    .await;
    let protected = encode_record(DIRECT_ROOT_APP_TOPIC, b"ambient protected read").unwrap();
    ambient_control
        .write(connection, &protected, now_ms())
        .await
        .unwrap();
    wait_for_tcp_state(
        &mut ambient_control,
        connection,
        None,
        Duration::from_secs(5),
    )
    .await;
    assert!(
        tokio::time::timeout(Duration::from_millis(300), deliveries.recv())
            .await
            .is_err(),
        "an ambient authenticated process delivered protected Drive control data",
    );

    drop(ambient_control);
    sync.shutdown().await.unwrap();
    ambient_blobs.shutdown().await.unwrap();
    ambient_endpoint.shutdown().await.unwrap();
}

#[tokio::test]
async fn pending_link_peer_can_bootstrap_but_cannot_access_drive_roots() {
    let admin_dir = tempfile::tempdir().unwrap();
    let mut admin = crate::Profile::create(admin_dir.path(), Some("admin".into())).unwrap();
    let pending = AppKey::generate("pending-link-peer");
    admin
        .state
        .inbound_app_key_link_requests
        .push(crate::profile::InboundAppKeyLinkRequest {
            app_key_pubkey: pending.pubkey_hex(),
            label: Some("pending".to_string()),
            invite_pubkey: "11".repeat(32),
            request_url: String::new(),
            requested_at: 1,
        });
    let config = AppConfig {
        profile: Some(admin.state.clone()),
        relays: Vec::new(),
        ..AppConfig::default()
    };
    let rendezvous = reserve_udp_address();
    let admin_addr = reserve_udp_address();
    let pending_addr = reserve_udp_address();
    let admin_bound = bind_test_endpoint(
        &admin.app_key,
        &discovery_scope(&config),
        rendezvous,
        admin_addr,
        true,
    )
    .await
    .unwrap();
    let pending_bound = bind_test_endpoint(
        &pending,
        "pending-link-peer",
        rendezvous,
        pending_addr,
        false,
    )
    .await
    .unwrap();
    let pending_endpoint = pending_bound.native_endpoint.clone();
    set_fips_peer_configs(
        pending_endpoint.as_ref(),
        vec![FipsPeerConfig {
            npub: admin.app_key.pubkey_bech32(),
            udp_addresses: vec![admin_addr.to_string()],
        }],
    )
    .await
    .unwrap();
    let sync = FipsBlockSync::start_with_bound_endpoint(
        admin_bound,
        Arc::new(MemoryStore::new()),
        &config,
        local_only_settings(&pending, pending_addr, admin_addr),
        None,
    )
    .await
    .unwrap();
    assert!(
        sync.authorized_peer_ids().is_empty(),
        "a pending device entered the approved Drive roster"
    );

    let mut deliveries = sync.subscribe_app_messages();
    let mut control = FipsTcpEndpoint::bind(
        pending_endpoint.clone(),
        DRIVE_CONTROL_SERVICE_PORT,
        TcpConfig {
            max_connections: 2,
            max_connections_per_peer: 2,
            ..TcpConfig::default()
        },
        now_ms(),
    )
    .await
    .unwrap();
    let admin_identity = PeerIdentity::from_npub(&admin.app_key.pubkey_bech32()).unwrap();
    let protected = control.connect(admin_identity, now_ms()).await.unwrap();
    wait_for_tcp_state(
        &mut control,
        protected,
        Some(State::Established),
        Duration::from_secs(5),
    )
    .await;
    let record = encode_record(DIRECT_ROOT_APP_TOPIC, b"pre-approval root request").unwrap();
    control.write(protected, &record, now_ms()).await.unwrap();
    wait_for_tcp_state(&mut control, protected, None, Duration::from_secs(5)).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(300), deliveries.recv())
            .await
            .is_err(),
        "a pending device delivered a protected Drive root request"
    );

    let bootstrap = control.connect(admin_identity, now_ms()).await.unwrap();
    wait_for_tcp_state(
        &mut control,
        bootstrap,
        Some(State::Established),
        Duration::from_secs(5),
    )
    .await;
    let record = encode_record(APP_KEY_LINK_REQUEST_APP_TOPIC, b"link request").unwrap();
    control.write(bootstrap, &record, now_ms()).await.unwrap();
    let delivered = tokio::time::timeout(Duration::from_secs(5), deliveries.recv())
        .await
        .expect("pending device bootstrap request was not delivered")
        .unwrap();
    assert_eq!(delivered.peer_id, pending.pubkey_bech32());
    assert_eq!(delivered.topic, APP_KEY_LINK_REQUEST_APP_TOPIC);

    drop(control);
    sync.shutdown().await.unwrap();
    pending_endpoint.shutdown().await.unwrap();
}
