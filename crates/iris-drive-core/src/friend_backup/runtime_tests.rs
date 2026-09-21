//! Exercises the production friend service over actual authenticated local FIPS.

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use fips_core::PeerIdentity;
use hashtree_core::{
    BlobReply, BlobRequest, BlobRoute, Cid, HashTree, HashTreeConfig, MemoryStore, Store,
};
use hashtree_fips_transport::{
    BoundFipsEndpoint, FipsEndpointOptions, FipsPeerConfig, TcpBlobTransport,
    bind_fips_endpoint_at_local_rendezvous, set_fips_peer_configs,
};
use hashtree_fs::FsBlobStore;
use nostr_sdk::{Keys, ToBech32, nips::nip44};

use super::exchange::{BACKUP_TOPIC, BackupMessage, fetch, parse_hash};
use super::routes::FriendReadRoute;
use super::runtime::{BACKUP_SCOPE, BackupService};
use super::{FriendBackupConfig, backup_keys, set_capacity, upsert_friend};
use crate::{AppConfig, AppKey, Daemon, Profile};

const ALLOCATION: u64 = 8 * 1024 * 1024;

async fn fixture(config_dir: &Path, file: &[u8]) -> (Daemon, Keys, Cid) {
    std::fs::create_dir_all(config_dir).unwrap();
    let profile =
        Profile::create(config_dir, Some("friend backup integration test".into())).unwrap();
    let mut config = AppConfig {
        profile: Some(profile.state.clone()),
        relays: Vec::new(),
        ..AppConfig::default()
    };
    config.upsert_drive(crate::config::Drive::primary(profile.state.root_scope_id()));
    config
        .save(crate::paths::config_path_in(config_dir))
        .unwrap();
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("personal.txt"), file).unwrap();
    let mut daemon = Daemon::open(config_dir).unwrap();
    daemon.import_source_dir(source.path()).await.unwrap();
    let root = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap()
        .root_cid;
    let device = AppKey::load(crate::paths::key_path_in(config_dir)).unwrap();
    (daemon, backup_keys(&device).unwrap(), root)
}

fn rendezvous_address() -> SocketAddrV4 {
    let socket = UdpSocket::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let SocketAddr::V4(address) = socket.local_addr().unwrap() else {
        unreachable!()
    };
    address
}

async fn endpoint(keys: &Keys, rendezvous: SocketAddrV4) -> BoundFipsEndpoint {
    Box::pin(bind_fips_endpoint_at_local_rendezvous(
        FipsEndpointOptions {
            identity_nsec: keys.secret_key().to_bech32().unwrap(),
            discovery_scope: BACKUP_SCOPE.into(),
            relays: Vec::new(),
            enable_udp: false,
            enable_webrtc: false,
            websocket: None,
            enable_local_rendezvous: true,
            ethernet_interfaces: Vec::new(),
            enable_lan_discovery: false,
            udp_bind_addr: None,
            udp_public: false,
            udp_external_addr: None,
            share_local_candidates: false,
            webrtc_auto_connect: false,
            webrtc_max_connections: 4,
            open_discovery_max_pending: 0,
            packet_channel_capacity: 1024,
        },
        rendezvous,
    ))
    .await
    .unwrap()
}

fn add_mutual(config_dir: &Path, friend: &str) -> FriendBackupConfig {
    set_capacity(config_dir, ALLOCATION).unwrap();
    upsert_friend(
        config_dir,
        friend,
        Some("Private friend".into()),
        ALLOCATION,
        true,
    )
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(clippy::too_many_lines)]
async fn mutual_friends_store_audit_and_restore_over_authenticated_fips() {
    tokio::time::timeout(Duration::from_mins(1), async {
        let temp = tempfile::tempdir().unwrap();
        let alice_dir = temp.path().join("alice");
        let bob_dir = temp.path().join("bob");
        let plaintext = (0_u8..=251).cycle().take(80 * 1024).collect::<Vec<_>>();
        let (alice_daemon, alice_keys, alice_root) = fixture(&alice_dir, &plaintext).await;
        let (_bob_daemon, bob_keys, bob_root) =
            fixture(&bob_dir, b"Bob's own private document").await;
        let alice_npub = alice_keys.public_key().to_bech32().unwrap();
        let bob_npub = bob_keys.public_key().to_bech32().unwrap();
        let alice_config = add_mutual(&alice_dir, &bob_npub);
        let bob_config = add_mutual(&bob_dir, &alice_npub);
        let rendezvous = rendezvous_address();
        let alice_bound = endpoint(&alice_keys, rendezvous).await;
        let bob_bound = endpoint(&bob_keys, rendezvous).await;
        let alice_reads = Arc::new(FriendReadRoute::default());
        let mut alice = BackupService::bind(
            &alice_dir,
            alice_keys.clone(),
            alice_bound,
            alice_reads.clone(),
        )
        .await
        .unwrap();
        let mut bob = BackupService::bind(
            &bob_dir,
            bob_keys.clone(),
            bob_bound,
            Arc::new(FriendReadRoute::default()),
        )
        .await
        .unwrap();
        let mut alice_messages = alice.control.subscribe();
        let mut bob_messages = bob.control.subscribe();

        // Both sides offer before receiving anything: neither a public follow nor
        // a privileged invitation is needed, and simultaneous offers must work.
        let (a, b) = tokio::join!(alice.tick(&alice_config), bob.tick(&bob_config));
        a.unwrap();
        b.unwrap();
        loop {
            let a_verified = alice
                .exchange
                .status
                .get(&bob_npub)
                .is_some_and(|status| status.last_checked_at.is_some());
            let b_verified = bob
                .exchange
                .status
                .get(&alice_npub)
                .is_some_and(|status| status.last_checked_at.is_some());
            if a_verified && b_verified {
                break;
            }
            tokio::select! {
                message = alice_messages.recv() => {
                    let message = message.unwrap();
                    assert_eq!(message.topic, BACKUP_TOPIC);
                    alice.handle(&message.peer_id, &message.data).await.unwrap();
                },
                message = bob_messages.recv() => {
                    let message = message.unwrap();
                    assert_eq!(message.topic, BACKUP_TOPIC);
                    bob.handle(&message.peer_id, &message.data).await.unwrap();
                },
            }
        }
        assert!(alice.exchange.status[&bob_npub].error.is_none());
        assert!(bob.exchange.status[&alice_npub].error.is_none());
        let manifest = bob
            .exchange
            .store
            .current_manifest(&alice_npub)
            .unwrap()
            .unwrap();
        assert_eq!(manifest.root_hash, hex::encode(alice_root.hash));
        assert_eq!(
            alice
                .exchange
                .store
                .current_manifest(&bob_npub)
                .unwrap()
                .unwrap()
                .root_hash,
            hex::encode(bob_root.hash)
        );
        alice
            .exchange
            .audit(&bob_npub, &alice.transport)
            .await
            .unwrap();
        bob.exchange
            .audit(&alice_npub, &bob.transport)
            .await
            .unwrap();

        // Bob stores enough ciphertext to restore Alice but has no reading key.
        let host_blocks = Arc::new(
            FsBlobStore::new(
                bob_dir
                    .join("friend-backups")
                    .join(alice_keys.public_key().to_hex())
                    .join("blocks"),
            )
            .unwrap(),
        );
        let host_tree = HashTree::new(HashTreeConfig::new(host_blocks));
        assert!(
            host_tree
                .list_directory_required(&Cid::public(alice_root.hash))
                .await
                .is_err()
        );
        assert!(
            nip44::decrypt(
                bob_keys.secret_key(),
                &bob_keys.public_key(),
                &manifest.encrypted_root_cid
            )
            .is_err()
        );

        // Remove every source snapshot blob and its transient export. Recovery
        // now has to traverse the production friend route to Bob's retained copy.
        let owner_store = alice_daemon.tree().get_store();
        for entry in &manifest.entries {
            assert!(
                owner_store
                    .delete(&parse_hash(&entry.hash).unwrap())
                    .await
                    .unwrap()
            );
        }
        alice.exchange.prepared = None;
        alice.exchange.local_route.set_routes(Vec::new());
        assert!(!owner_store.has(&alice_root.hash).await.unwrap());
        alice
            .control
            .send(
                bob_npub.clone(),
                BACKUP_TOPIC.into(),
                serde_json::to_vec(&BackupMessage::RequestHead).unwrap(),
            )
            .await
            .unwrap();
        let request = bob_messages.recv().await.unwrap();
        assert!(matches!(
            serde_json::from_slice::<BackupMessage>(&request.data).unwrap(),
            BackupMessage::RequestHead
        ));
        bob.handle(&request.peer_id, &request.data).await.unwrap();
        let response = alice_messages.recv().await.unwrap();
        let BackupMessage::Head { manifest_hash } = serde_json::from_slice(&response.data).unwrap()
        else {
            panic!("friend did not return the recovery manifest address")
        };
        let remote = alice
            .transport
            .route_to(PeerIdentity::from_npub(&bob_npub).unwrap());
        let manifest_bytes = fetch(&remote, parse_hash(&manifest_hash).unwrap())
            .await
            .unwrap();
        let recovered_manifest =
            super::storage::BackupManifest::from_bytes(&manifest_bytes).unwrap();
        assert_eq!(recovered_manifest, manifest);
        let manifest = recovered_manifest;

        // A replacement install has a different AppKey. The portable backup
        // identity restores into a new folder and preserves its existing files.
        let replacement_dir = temp.path().join("replacement");
        let (replacement_before, replacement_keys, _) =
            fixture(&replacement_dir, b"keep this replacement-install document").await;
        assert_ne!(replacement_keys.public_key(), alice_keys.public_key());
        let before_config = std::fs::read(crate::paths::config_path_in(&replacement_dir)).unwrap();
        let before_bytes = replacement_before
            .tree()
            .get_store()
            .as_ref()
            .stats()
            .unwrap()
            .total_bytes;
        assert!(
            super::recovery::restore_snapshot(&replacement_dir, &bob_keys, &manifest, &remote)
                .await
                .is_err()
        );
        assert_eq!(
            std::fs::read(crate::paths::config_path_in(&replacement_dir)).unwrap(),
            before_config
        );
        assert_eq!(
            replacement_before
                .tree()
                .get_store()
                .as_ref()
                .stats()
                .unwrap()
                .total_bytes,
            before_bytes
        );
        let folder =
            super::recovery::restore_snapshot(&replacement_dir, &alice_keys, &manifest, &remote)
                .await
                .unwrap();
        let replacement = Daemon::open(&replacement_dir).unwrap();
        let replacement_root = Cid::parse(replacement.primary_root().unwrap()).unwrap();
        let existing = replacement
            .tree()
            .resolve(&replacement_root, "personal.txt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            replacement
                .tree()
                .get(&existing, None)
                .await
                .unwrap()
                .unwrap(),
            b"keep this replacement-install document"
        );
        let imported = replacement
            .tree()
            .resolve(&replacement_root, &format!("{folder}/personal.txt"))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            replacement
                .tree()
                .get(&imported, None)
                .await
                .unwrap()
                .unwrap(),
            plaintext
        );

        for entry in &manifest.entries {
            let hash = parse_hash(&entry.hash).unwrap();
            let BlobReply::Data(bytes) = alice_reads
                .route(BlobRequest { hash, htl: 0 })
                .await
                .unwrap()
            else {
                panic!("friend did not return retained ciphertext")
            };
            assert_eq!(hashtree_core::sha256(&bytes), hash);
            owner_store.put(hash, bytes).await.unwrap();
        }
        let recovered = nip44::decrypt(
            alice_keys.secret_key(),
            &alice_keys.public_key(),
            &manifest.encrypted_root_cid,
        )
        .unwrap();
        let recovered_root = Cid::parse(&recovered).unwrap();
        assert_eq!(recovered_root, alice_root);
        let file = alice_daemon
            .tree()
            .resolve(&recovered_root, "personal.txt")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            alice_daemon.tree().get(&file, None).await.unwrap().unwrap(),
            plaintext
        );

        // A connected third identity still cannot read even a known backup hash.
        let stranger = Keys::generate();
        let stranger_bound = endpoint(&stranger, rendezvous).await;
        set_fips_peer_configs(
            &stranger_bound.native_endpoint,
            vec![FipsPeerConfig::new(bob_npub.clone())],
        )
        .await
        .unwrap();
        // Give the transport a path in both directions without adding this
        // identity to Bob's application-level friend authorization list.
        set_fips_peer_configs(
            &bob.endpoint,
            vec![
                FipsPeerConfig::new(alice_npub.clone()),
                FipsPeerConfig::new(stranger.public_key().to_bech32().unwrap()),
            ],
        )
        .await
        .unwrap();
        let stranger_transport = TcpBlobTransport::bind(
            stranger_bound.native_endpoint.clone(),
            Arc::new(MemoryStore::new()),
        )
        .await
        .unwrap();
        let denied = stranger_transport
            .fetch_from_peer(
                &alice_root.hash,
                PeerIdentity::from_npub(&bob_npub).unwrap(),
            )
            .await
            .expect_err("unknown identity obtained friend's encrypted backup");
        assert!(
            denied.to_string().contains("closed"),
            "expected authenticated service rejection, not routing unavailability: {denied}"
        );
        stranger_transport.shutdown().await.unwrap();
        stranger_bound.native_endpoint.shutdown().await.unwrap();
        alice.shutdown().await;
        bob.shutdown().await;
    })
    .await
    .expect("friend backup exchange, audit and recovery exceeded one minute");
}
