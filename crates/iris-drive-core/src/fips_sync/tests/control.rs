use super::*;
use std::collections::BTreeSet;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::DIRECT_ROOT_APP_TOPIC;
use fips_core::PeerIdentity;
use fips_tcp::{Config as TcpConfig, ConnectionId, State};
use fips_tcp_endpoint::FipsTcpEndpoint;
use hashtree_fips_transport::{BoundFipsEndpoint, FipsPeerConfig, set_fips_peer_configs};
use nostr_sdk::{Alphabet, SingleLetterTag, Tag, TagKind};

use super::super::control_runtime::{
    BOOTSTRAP_STREAM_LIFETIME_MS, DRIVE_CONTROL_SERVICE_PORT, encode_record,
};
use super::mesh_fallback::{bind_test_endpoint, reserve_udp_address, wait_for_peer_connection};

#[tokio::test]
async fn reliable_control_enforces_topic_acl_and_survives_service_restart() {
    let alice = AppKey::generate("control-alice");
    let bob = AppKey::generate("control-bob");
    let rendezvous = reserve_udp_address();
    let alice_addr = reserve_udp_address();
    let bob_addr = reserve_udp_address();
    let alice_bound =
        bind_test_endpoint(&alice, "drive-control-test", rendezvous, alice_addr, false)
            .await
            .unwrap();
    let bob_bound = bind_test_endpoint(&bob, "drive-control-test", rendezvous, bob_addr, false)
        .await
        .unwrap();
    let alice_endpoint = alice_bound.native_endpoint;
    let bob_endpoint = bob_bound.native_endpoint;
    set_fips_peer_configs(
        alice_endpoint.as_ref(),
        vec![FipsPeerConfig {
            npub: bob.pubkey_bech32(),
            udp_addresses: vec![bob_addr.to_string()],
        }],
    )
    .await
    .unwrap();
    set_fips_peer_configs(
        bob_endpoint.as_ref(),
        vec![FipsPeerConfig {
            npub: alice.pubkey_bech32(),
            udp_addresses: vec![alice_addr.to_string()],
        }],
    )
    .await
    .unwrap();

    let mut alice_runtime = DriveControlRuntime::bind(
        alice_endpoint.clone(),
        BTreeSet::from([bob.pubkey_bech32()]),
        BTreeSet::new(),
    )
    .await
    .unwrap();
    let mut bob_runtime = DriveControlRuntime::bind(
        bob_endpoint.clone(),
        BTreeSet::new(),
        BTreeSet::from([APP_KEY_LINK_REQUEST_APP_TOPIC]),
    )
    .await
    .unwrap();
    let mut received = bob_runtime.subscribe();

    alice_runtime
        .send(
            bob.pubkey_bech32(),
            APP_KEY_LINK_REQUEST_APP_TOPIC.to_string(),
            b"link request".to_vec(),
        )
        .await
        .unwrap();
    let request = tokio::time::timeout(Duration::from_secs(5), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(request.peer_id, alice.pubkey_bech32());
    assert_eq!(request.topic, APP_KEY_LINK_REQUEST_APP_TOPIC);

    bob_runtime
        .set_policy(
            BTreeSet::from([alice.pubkey_bech32()]),
            BTreeSet::from([APP_KEY_LINK_REQUEST_APP_TOPIC]),
        )
        .await
        .unwrap();
    alice_runtime
        .send(
            bob.pubkey_bech32(),
            DIRECT_ROOT_APP_TOPIC.to_string(),
            b"authorized root".to_vec(),
        )
        .await
        .unwrap();
    let root = tokio::time::timeout(Duration::from_secs(5), received.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(root.topic, DIRECT_ROOT_APP_TOPIC);
    assert_eq!(root.data, b"authorized root");

    bob_runtime.shutdown().await.unwrap();
    alice_runtime
        .send(
            bob.pubkey_bech32(),
            DIRECT_ROOT_APP_TOPIC.to_string(),
            b"queued first during restart".to_vec(),
        )
        .await
        .unwrap();
    alice_runtime
        .send(
            bob.pubkey_bech32(),
            DIRECT_ROOT_APP_TOPIC.to_string(),
            b"queued second during restart".to_vec(),
        )
        .await
        .unwrap();
    let mut replacement = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match DriveControlRuntime::bind(
                bob_endpoint.clone(),
                BTreeSet::from([alice.pubkey_bech32()]),
                BTreeSet::new(),
            )
            .await
            {
                Ok(runtime) => break runtime,
                Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
            }
        }
    })
    .await
    .expect("Drive control service port was not released after shutdown");
    let mut replacement_received = replacement.subscribe();
    for expected in [
        b"queued first during restart".as_slice(),
        b"queued second during restart".as_slice(),
    ] {
        let restarted = tokio::time::timeout(Duration::from_secs(8), replacement_received.recv())
            .await
            .expect("queued Drive control record was not replayed after service restart")
            .unwrap();
        assert_eq!(restarted.data, expected);
    }

    replacement.shutdown().await.unwrap();
    alice_runtime.shutdown().await.unwrap();
    alice_endpoint.shutdown().await.unwrap();
    bob_endpoint.shutdown().await.unwrap();
}

#[tokio::test]
async fn shared_pubsub_carries_only_verified_nostr_events() {
    let alice = AppKey::generate("pubsub-alice");
    let bob = AppKey::generate("pubsub-bob");
    let rendezvous = reserve_udp_address();
    let alice_addr = reserve_udp_address();
    let bob_addr = reserve_udp_address();
    let alice_endpoint =
        bind_test_endpoint(&alice, "drive-pubsub-test", rendezvous, alice_addr, false)
            .await
            .unwrap()
            .native_endpoint;
    let bob_endpoint = bind_test_endpoint(&bob, "drive-pubsub-test", rendezvous, bob_addr, false)
        .await
        .unwrap()
        .native_endpoint;
    set_fips_peer_configs(
        alice_endpoint.as_ref(),
        vec![FipsPeerConfig {
            npub: bob.pubkey_bech32(),
            udp_addresses: vec![bob_addr.to_string()],
        }],
    )
    .await
    .unwrap();

    set_fips_peer_configs(
        bob_endpoint.as_ref(),
        vec![FipsPeerConfig {
            npub: alice.pubkey_bech32(),
            udp_addresses: vec![alice_addr.to_string()],
        }],
    )
    .await
    .unwrap();
    wait_for_connected_endpoint(&alice_endpoint, &bob.pubkey_bech32()).await;
    wait_for_connected_endpoint(&bob_endpoint, &alice.pubkey_bech32()).await;

    let mut alice_runtime = DriveNostrPubsubRuntime::bind(alice_endpoint.clone())
        .await
        .unwrap();
    let mut bob_runtime = DriveNostrPubsubRuntime::bind(bob_endpoint.clone())
        .await
        .unwrap();
    let mut received = bob_runtime.subscribe();
    let release_keys = Keys::generate();
    let tree_name = "releases/iris-drive";
    let reference = hashtree_updater::UpdateRef {
        npub: release_keys.public_key().to_bech32().unwrap(),
        tree_name: tree_name.to_string(),
        path: Some("latest".to_string()),
    };
    let event = EventBuilder::new(Kind::Custom(30_064), "verified update")
        .tags([
            Tag::identifier(tree_name),
            Tag::custom(
                TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::L)),
                ["hashtree"],
            ),
            Tag::custom(TagKind::Custom("hash".into()), ["42".repeat(32)]),
        ])
        .sign_with_keys(&release_keys)
        .unwrap();
    let delivery = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if alice_runtime.publish(event.clone()).await.is_ok()
                && let Ok(Ok(delivery)) =
                    tokio::time::timeout(Duration::from_millis(300), received.recv()).await
            {
                break delivery;
            }
        }
    })
    .await
    .expect("verified update event was not delivered over shared FIPS pubsub");
    assert_eq!(delivery.event.id, event.id);
    assert_eq!(delivery.origin_peer_id, alice.pubkey_bech32());
    let target_dir = tempfile::tempdir().unwrap();
    let mut exchange =
        crate::UpdateAnnouncementExchange::load_for_reference(target_dir.path(), &reference)
            .unwrap();
    assert!(
        exchange
            .handle_nostr_event(target_dir.path(), &delivery)
            .unwrap()
    );
    assert_eq!(
        exchange.latest_event().map(|event| event.id),
        Some(event.id)
    );

    alice_runtime.shutdown().await;
    bob_runtime.shutdown().await;
    alice_endpoint.shutdown().await.unwrap();
    bob_endpoint.shutdown().await.unwrap();
}

#[tokio::test]
async fn disallowed_malformed_and_partial_streams_are_reset_and_release_capacity() {
    let server_key = AppKey::generate("control-reset-server");
    let client_key = AppKey::generate("control-reset-client");
    let rendezvous = reserve_udp_address();
    let server_addr = reserve_udp_address();
    let client_addr = reserve_udp_address();
    let server_bound = bind_test_endpoint(
        &server_key,
        "drive-control-reset-test",
        rendezvous,
        server_addr,
        false,
    )
    .await
    .unwrap();
    let client_bound = bind_test_endpoint(
        &client_key,
        "drive-control-reset-test",
        rendezvous,
        client_addr,
        false,
    )
    .await
    .unwrap();
    connect_endpoint_pair(
        &server_bound,
        &server_key,
        server_addr,
        &client_bound,
        &client_key,
        client_addr,
    )
    .await;

    let mut server = DriveControlRuntime::bind(
        server_bound.native_endpoint.clone(),
        BTreeSet::new(),
        BTreeSet::from([APP_KEY_LINK_REQUEST_APP_TOPIC]),
    )
    .await
    .unwrap();
    let mut deliveries = server.subscribe();
    let mut client = FipsTcpEndpoint::bind(
        client_bound.native_endpoint.clone(),
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
    let server_peer = PeerIdentity::from_npub(&server_key.pubkey_bech32()).unwrap();

    let disallowed = client.connect(server_peer, now_ms()).await.unwrap();
    wait_for_tcp_state(
        &mut client,
        disallowed,
        Some(State::Established),
        Duration::from_secs(5),
    )
    .await;
    let record = encode_record(DIRECT_ROOT_APP_TOPIC, b"unauthorized root").unwrap();
    client.write(disallowed, &record, now_ms()).await.unwrap();
    wait_for_tcp_state(&mut client, disallowed, None, Duration::from_secs(5)).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(300), deliveries.recv())
            .await
            .is_err(),
        "an unauthorized peer delivered a protected Drive control topic"
    );

    let malformed = client.connect(server_peer, now_ms()).await.unwrap();
    wait_for_tcp_state(
        &mut client,
        malformed,
        Some(State::Established),
        Duration::from_secs(5),
    )
    .await;
    client
        .write(malformed, &[0, 0, 0, 0], now_ms())
        .await
        .unwrap();
    wait_for_tcp_state(&mut client, malformed, None, Duration::from_secs(5)).await;

    let partial = client.connect(server_peer, now_ms()).await.unwrap();
    wait_for_tcp_state(
        &mut client,
        partial,
        Some(State::Established),
        Duration::from_secs(5),
    )
    .await;
    let record = encode_record(APP_KEY_LINK_REQUEST_APP_TOPIC, b"partial").unwrap();
    client
        .write(partial, &record[..record.len() - 1], now_ms())
        .await
        .unwrap();
    wait_for_tcp_state(
        &mut client,
        partial,
        None,
        Duration::from_millis(BOOTSTRAP_STREAM_LIFETIME_MS + 2_000),
    )
    .await;

    let valid = client.connect(server_peer, now_ms()).await.unwrap();
    wait_for_tcp_state(
        &mut client,
        valid,
        Some(State::Established),
        Duration::from_secs(5),
    )
    .await;
    let record = encode_record(APP_KEY_LINK_REQUEST_APP_TOPIC, b"valid").unwrap();
    client.write(valid, &record, now_ms()).await.unwrap();
    let delivered = tokio::time::timeout(Duration::from_secs(5), deliveries.recv())
        .await
        .expect("valid bootstrap record was not admitted after reset slots were released")
        .unwrap();
    assert_eq!(delivered.peer_id, client_key.pubkey_bech32());
    assert_eq!(delivered.data, b"valid");

    drop(client);
    server.shutdown().await.unwrap();
    client_bound.native_endpoint.shutdown().await.unwrap();
    server_bound.native_endpoint.shutdown().await.unwrap();
}

async fn connect_endpoint_pair(
    left: &BoundFipsEndpoint,
    left_key: &AppKey,
    left_addr: std::net::SocketAddrV4,
    right: &BoundFipsEndpoint,
    right_key: &AppKey,
    right_addr: std::net::SocketAddrV4,
) {
    set_fips_peer_configs(
        left.native_endpoint.as_ref(),
        vec![FipsPeerConfig {
            npub: right_key.pubkey_bech32(),
            udp_addresses: vec![right_addr.to_string()],
        }],
    )
    .await
    .unwrap();
    set_fips_peer_configs(
        right.native_endpoint.as_ref(),
        vec![FipsPeerConfig {
            npub: left_key.pubkey_bech32(),
            udp_addresses: vec![left_addr.to_string()],
        }],
    )
    .await
    .unwrap();
    wait_for_peer_connection(left, &right_key.pubkey_bech32()).await;
    wait_for_peer_connection(right, &left_key.pubkey_bech32()).await;
}

async fn wait_for_connected_endpoint(endpoint: &FipsEndpoint, peer: &str) {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if endpoint
                .peers()
                .await
                .unwrap()
                .iter()
                .any(|candidate| candidate.npub == peer && candidate.connected)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("FIPS endpoint peer did not connect");
}

async fn wait_for_tcp_state(
    tcp: &mut FipsTcpEndpoint,
    id: ConnectionId,
    expected: Option<State>,
    timeout: Duration,
) {
    tokio::time::timeout(timeout, async {
        loop {
            let now = now_ms();
            let _ = tokio::time::timeout(Duration::from_millis(20), tcp.receive_report(now)).await;
            tcp.poll(now).await.unwrap();
            if tcp.state(id) == expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("TCP/FIPS stream did not reach {expected:?}"));
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
