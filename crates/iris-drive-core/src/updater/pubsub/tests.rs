use super::*;
use std::time::Duration;

use fips_core::config::{PeerConfig, TransportInstances, UdpConfig};
use hashtree_resolver::RootResolver;
use hashtree_updater::{PubsubRootResolver, UpdateEventCache, UpdateRef};
use nostr_pubsub::{EventBus, EventSource, VerifiedEvent};
use nostr_sdk::{Event, EventBuilder, Kind, Tag, TagKind, Timestamp};

use crate::update_announcement::{load_update_event_cache, persist_update_event_cache};

async fn endpoint(peers: Vec<PeerConfig>) -> Arc<FipsEndpoint> {
    let mut config = fips_core::Config::new();
    config.node.control.enabled = false;
    config.node.discovery.nostr.enabled = false;
    config.node.discovery.local.enabled = false;
    config.node.discovery.lan.enabled = false;
    config.transports.udp = TransportInstances::Single(UdpConfig {
        bind_addr: Some("127.0.0.1:0".into()),
        advertise_on_nostr: Some(false),
        accept_connections: Some(true),
        ..Default::default()
    });
    config.peers = peers;
    Arc::new(
        Box::pin(
            FipsEndpoint::builder()
                .config(config)
                .identity_nsec(Keys::generate().secret_key().to_bech32().unwrap())
                .without_system_tun()
                .bind(),
        )
        .await
        .unwrap(),
    )
}

fn release(keys: &Keys, created_at: u64, byte: u8) -> Event {
    EventBuilder::new(Kind::Custom(30_064), "")
        .tags([
            Tag::identifier("releases/iris-drive"),
            Tag::custom(TagKind::Custom("l".into()), ["hashtree"]),
            Tag::custom(
                TagKind::Custom("hash".into()),
                [format!("{byte:02x}").repeat(32)],
            ),
        ])
        .custom_created_at(Timestamp::from(created_at))
        .sign_with_keys(keys)
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shared_fips_provider_refreshes_cached_release_and_fails_offline() {
    let publisher_endpoint = endpoint(Vec::new()).await;
    let publisher = FipsPubsubClient::start(publisher_endpoint.clone(), Default::default())
        .await
        .unwrap();
    let keys = Keys::generate();
    let reference = UpdateRef {
        npub: keys.public_key().to_bech32().unwrap(),
        tree_name: "releases/iris-drive".into(),
        path: Some("latest".into()),
    };
    let event = release(&keys, 10, 0x42);
    publisher
        .publish(
            VerifiedEvent::try_from(event.clone()).unwrap(),
            EventSource::local_index("release"),
        )
        .await
        .unwrap();
    publisher
        .publish(
            VerifiedEvent::try_from(release(&Keys::generate(), 30, 0x99)).unwrap(),
            EventSource::local_index("untrusted-release"),
        )
        .await
        .unwrap();
    let address = publisher_endpoint.bound_udp_listen_addrs().await.unwrap()[0].to_string();
    let app_endpoint = endpoint(vec![PeerConfig::new(
        publisher_endpoint.npub(),
        "udp",
        &address,
    )])
    .await;
    let app_client = Arc::new(
        FipsPubsubClient::start(app_endpoint.clone(), Default::default())
            .await
            .unwrap(),
    );
    let filter = UpdateEventCache::new(&reference).unwrap().filter().clone();
    let mut app_subscription = app_client.subscribe(vec![filter.clone()]).await.unwrap();
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(8), app_subscription.recv())
            .await
            .unwrap()
            .unwrap()
            .event
            .as_event()
            .id,
        event.id
    );

    let directory = tempfile::tempdir().unwrap();
    register(directory.path(), &app_client);
    let provider = UpdatePubsub::connect(&ProductUpdateConfig {
        config_dir: Some(directory.path().to_path_buf()),
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(
        provider.connections.owned_fips.is_none(),
        "must reuse the app endpoint"
    );
    assert!(
        provider.connections.relay.is_none(),
        "empty relays must stay empty"
    );
    let resolver = PubsubRootResolver::new(provider, Duration::from_millis(500));
    let key = reference.resolver_key();
    for _ in 0..2 {
        assert_eq!(
            resolver.resolve(&key).await.unwrap().unwrap().hash,
            [0x42; 32]
        );
    }

    publisher.shutdown_shared().await;
    publisher_endpoint.shutdown().await.unwrap();
    let mut cached = app_client.subscribe(vec![filter]).await.unwrap();
    assert_eq!(cached.recv().await.unwrap().event.as_event().id, event.id);
    drop(cached);
    assert!(
        resolver.resolve(&key).await.is_err(),
        "cached signed roots cannot confirm freshness"
    );
    drop(resolver);
    // Dropping an updater must leave the app's ordinary subscription intact.
    assert_eq!(app_client.active_subscription_count().unwrap(), 1);
    drop(app_subscription);
    app_client.shutdown_shared().await;
    app_endpoint.shutdown().await.unwrap();
}

#[test]
fn older_check_does_not_overwrite_newer_persisted_release() {
    let keys = Keys::generate();
    let reference = UpdateRef {
        npub: keys.public_key().to_bech32().unwrap(),
        tree_name: "releases/iris-drive".into(),
        path: None,
    };
    let directory = tempfile::tempdir().unwrap();
    let mut older = UpdateEventCache::new(&reference).unwrap();
    older.ingest_event(release(&keys, 10, 1)).unwrap();
    let mut newer = older.clone();
    newer.ingest_event(release(&keys, 20, 2)).unwrap();
    persist_update_event_cache(directory.path(), &newer).unwrap();
    persist_update_event_cache(directory.path(), &older).unwrap();
    assert_eq!(
        load_update_event_cache(directory.path(), &reference)
            .unwrap()
            .latest(),
        newer.latest()
    );
}
