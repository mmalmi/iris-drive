//! A root learned directly from a peer is recorded once its blocks are here.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hashtree_core::{Cid, HashTree, HashTreeConfig, MemoryStore};
use hashtree_fs::FsBlobStore;

use super::{apply_direct_root_event, apply_direct_root_hint, drive_root_transport_key};
use crate::paths::config_path_in;
use crate::root_meta::DriveRootMeta;
use crate::{AppConfig, AppKey, AppKeyRootRef, Drive, FipsBlockSync, PRIMARY_DRIVE_ID, Profile};

const CHILD_ENV: &str = "IRIS_DRIVE_DIRECT_ROOT_DOWNLOAD_TEST_CHILD";

/// Run real FIPS endpoints in an isolated process so discovery settings cannot
/// affect concurrently running tests or contact the user's configured peers.
#[test]
fn a_direct_root_is_recorded_once_its_blocks_are_here() {
    if std::env::var_os(CHILD_ENV).is_some() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(60), Box::pin(record_after_download()))
                .await
                .expect("direct-root download fixture did not finish");
        });
        return;
    }

    let directory = tempfile::tempdir().unwrap();
    let rendezvous = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "direct_root_transport::download_tests::a_direct_root_is_recorded_once_its_blocks_are_here",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .env("HTREE_DATA_DIR", directory.path().join("shared"))
        .env("HTREE_CONFIG_DIR", directory.path().join("htree-config"))
        .env("IRIS_DRIVE_FIPS_ENABLE_UDP", "true")
        .env("IRIS_DRIVE_FIPS_UDP_BIND_ADDR", "127.0.0.1:0")
        .env("IRIS_DRIVE_FIPS_UDP_PUBLIC", "false")
        .env("IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR", "")
        .env("IRIS_DRIVE_FIPS_ENABLE_LOCAL_RENDEZVOUS", "true")
        .env(
            "IRIS_DRIVE_FIPS_LOCAL_RENDEZVOUS_ADDR",
            rendezvous.to_string(),
        )
        .env("IRIS_DRIVE_FIPS_ENABLE_WEBRTC", "false")
        .env("IRIS_DRIVE_FIPS_ENABLE_LAN_DISCOVERY", "false")
        .env("IRIS_DRIVE_FIPS_ENABLE_NOSTR_DISCOVERY", "false")
        .env("IRIS_DRIVE_FIPS_ENABLE_MESH_PUBSUB", "false")
        .env("IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP", "false")
        .env("IRIS_DRIVE_FIPS_STATIC_PEERS", "")
        .env("IRIS_FIPS_WEBSOCKET_BIND_ADDR", "")
        .env("IRIS_FIPS_WEBSOCKET_SEED_URLS", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "direct-root download fixture failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

async fn record_after_download() {
    let directory = tempfile::tempdir().unwrap();
    let reader_dir = directory.path().join("iphone");
    let mut profile = Profile::create(&reader_dir, Some("iPhone".into())).unwrap();
    let source_key = AppKey::generate(directory.path().join("pixel-key"));
    profile
        .approve_app_key(&source_key.pubkey_hex(), Some("Pixel".into()))
        .unwrap();
    let mut config = AppConfig {
        profile: Some(profile.state.clone()),
        relays: Vec::new(),
        ..AppConfig::default()
    };
    config.blossom_servers.clear();
    config.upsert_drive(Drive::primary(profile.state.root_scope_id()));
    config.save(config_path_in(&reader_dir)).unwrap();

    // The Pixel's newest root, made after it merged this device's Drive. Its
    // blocks exist only on the Pixel.
    let source_store = Arc::new(MemoryStore::new());
    let source_tree = HashTree::new(HashTreeConfig::new(source_store.clone()));
    let files = tempfile::tempdir().unwrap();
    std::fs::write(files.path().join("from-pixel.txt"), b"merged on the Pixel").unwrap();
    let meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.into(),
        app_key_pubkey: source_key.pubkey_hex(),
        app_key_seq: 1,
        dck_generation: 1,
        local_only: false,
        parents: vec![],
        observed: BTreeMap::new(),
        created_at: 101,
    };
    let root = crate::indexer::index_dir_with_history_and_meta(
        &source_tree,
        files.path(),
        None,
        101,
        Some(&meta),
    )
    .await
    .unwrap();
    let root_ref = AppKeyRootRef::from_meta(root.to_string(), 101, &meta);
    let event = crate::nostr_events::build_drive_root_event(
        source_key.keys(),
        &profile.state.root_scope_id(),
        PRIMARY_DRIVE_ID,
        &root_ref,
        &[
            profile.state.app_key_pubkey.clone(),
            source_key.pubkey_hex(),
        ],
    )
    .unwrap();

    let local = Arc::new(FsBlobStore::new(reader_dir.join("blocks")).unwrap());
    let reader = FipsBlockSync::start(&profile.app_key, local.clone(), &config)
        .await
        .unwrap();
    let recorded_root = || {
        AppConfig::load_or_default(config_path_in(&reader_dir))
            .unwrap()
            .drive(PRIMARY_DRIVE_ID)
            .unwrap()
            .app_key_roots
            .get(&source_key.pubkey_hex())
            .map(|root| root.root_cid.clone())
    };

    // The Pixel announced the root (key hint, then the signed event) and went
    // away before serving its blocks.
    let recipients = [
        profile.state.app_key_pubkey.clone(),
        source_key.pubkey_hex(),
    ];
    let hint = drive_root_transport_key(
        &source_key.pubkey_hex(),
        PRIMARY_DRIVE_ID,
        &root_ref,
        &recipients,
    )
    .unwrap();
    let unreachable =
        apply_direct_root_hint(&reader_dir, &reader, &hint, &source_key.pubkey_bech32()).await;
    assert!(
        unreachable.is_err(),
        "nothing could serve the hinted blocks"
    );
    assert_eq!(
        recorded_root(),
        None,
        "a hinted root this device cannot read must not be recorded"
    );
    let unreachable = apply_direct_root_event(&reader_dir, &event, Some(&reader)).await;
    assert!(unreachable.is_err(), "nothing could serve the blocks");
    assert_eq!(
        recorded_root(),
        None,
        "a root this device cannot read must not be recorded"
    );

    let mut source_config = config.clone();
    source_config.profile.as_mut().unwrap().app_key_pubkey = source_key.pubkey_hex();
    let source = FipsBlockSync::start(&source_key, source_store, &source_config)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if reader
                .fips_peer_statuses()
                .await
                .iter()
                .any(|peer| peer.npub == source_key.pubkey_bech32() && peer.connected)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("authorized local FIPS peer did not connect");

    let hinted =
        apply_direct_root_hint(&reader_dir, &reader, &hint, &source_key.pubkey_bech32()).await;
    let recorded_by_hint = recorded_root();
    let applied = apply_direct_root_event(&reader_dir, &event, Some(&reader)).await;
    reader.shutdown_endpoint().await.unwrap();
    source.shutdown_endpoint().await.unwrap();
    assert!(hinted.unwrap());
    assert_eq!(recorded_by_hint, Some(root.to_string()));
    applied.unwrap();
    assert_eq!(recorded_root(), Some(root.to_string()));
    let tree = HashTree::new(HashTreeConfig::new(local));
    let root = Cid::parse(&root.to_string()).unwrap();
    assert!(
        crate::merge::walk_app_key_tree(&tree, &root).await.is_ok(),
        "the recorded root is readable on this device"
    );
}
