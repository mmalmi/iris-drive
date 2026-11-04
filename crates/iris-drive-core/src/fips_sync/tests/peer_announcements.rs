use super::control::connect_endpoint_pair;
use super::mesh_fallback::{bind_test_endpoint, reserve_udp_address};
use super::*;
use fips_core::discovery::nostr::{ADVERT_IDENTIFIER, ADVERT_KIND};
use hashtree_fips_transport::BoundFipsEndpoint;
use nostr_pubsub::{EventBus, EventSource, Filter, QueryOptions, VerifiedEvent};
use nostr_pubsub_fips::{FipsPubsubClient, FipsPubsubClientOptions};
use nostr_sdk::{Event, Tag, TagKind, Timestamp};
use std::net::SocketAddrV4;
use std::time::Duration;
use tokio::time::timeout;

const SCOPE: &str = "iris-drive:announcement-restart";

async fn endpoint(key: &AppKey, address: SocketAddrV4) -> BoundFipsEndpoint {
    let bound = bind_test_endpoint(key, SCOPE, reserve_udp_address(), address, false)
        .await
        .unwrap();
    assert!(
        bound
            .native_endpoint
            .relay_statuses()
            .await
            .unwrap()
            .is_empty()
    );
    bound
}

async fn drive(bound: &BoundFipsEndpoint, key: &AppKey) -> FipsBlockSync<MemoryStore> {
    FipsBlockSync::start_with_bound_endpoint_and_recent_peers(
        BoundFipsEndpoint {
            native_endpoint: bound.native_endpoint.clone(),
            local_peer_id: bound.local_peer_id.clone(),
            discovery_scope: bound.discovery_scope.clone(),
        },
        Arc::new(MemoryStore::new()),
        &AppConfig::default(),
        FipsTransportSettings {
            enable_mesh_pubsub: true,
            static_peer_hints: Vec::new(),
            bootstrap_peer_hints: Vec::new(),
            ..Default::default()
        },
        None,
        Some(recent_peers_file_path(key.path())),
    )
    .await
    .unwrap()
}

fn advert(keys: &Keys, scope: &str, content: &str, created_at: Timestamp) -> Event {
    EventBuilder::new(Kind::Custom(ADVERT_KIND), content)
        .tags([
            Tag::identifier(scope),
            Tag::custom(TagKind::custom("protocol"), [scope]),
            Tag::custom(TagKind::custom("version"), ["1"]),
            Tag::expiration(Timestamp::from(created_at.as_secs() + 3600)),
        ])
        .custom_created_at(created_at)
        .sign_with_keys(keys)
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn restarted_drive_serves_valid_peer_announcements_without_relays_or_original_peer() {
    let device_dir = tempfile::tempdir().unwrap();
    let key = AppKey::load_or_generate(device_dir.path().join("key")).unwrap();
    let original = AppKey::generate("announcement-carrier");
    let addr = reserve_udp_address();
    let author_addr = reserve_udp_address();
    let bound = endpoint(&key, addr).await;
    let author_bound = endpoint(&original, author_addr).await;
    let first = drive(&bound, &key).await;
    let author = FipsPubsubClient::start(author_bound.native_endpoint.clone(), Default::default())
        .await
        .unwrap();
    connect_endpoint_pair(&bound, &key, addr, &author_bound, &original, author_addr).await;

    let content = serde_json::json!({
        "identifier": ADVERT_IDENTIFIER, "version": 1,
        "endpoints": [{"transport": "tcp", "addr": "absent-peer.invalid:2121"}]
    })
    .to_string();
    let keys = Keys::generate();
    let now = Timestamp::now();
    let older = advert(&keys, SCOPE, &content, Timestamp::from(now.as_secs() - 1));
    let valid = advert(&keys, SCOPE, &content, now);
    let malformed = advert(&Keys::generate(), SCOPE, "{}", now);
    let other_scope = advert(&Keys::generate(), "iris-drive:other", &content, now);
    assert!(
        bound
            .native_endpoint
            .ingest_nostr_discovery_event(older.clone())
            .await
            .unwrap()
    );
    for event in [&valid, &malformed, &other_scope] {
        author
            .publish(
                VerifiedEvent::try_from(event.clone()).unwrap(),
                EventSource::local_index("fixture"),
            )
            .await
            .unwrap();
        timeout(Duration::from_secs(10), async {
            while first.recv_nostr_pubsub_event().await.event.id != event.id {}
        })
        .await
        .expect("Drive processed the signed event from its peer");
    }
    author.shutdown().await;
    author_bound.native_endpoint.shutdown().await.unwrap();
    first.shutdown().await.unwrap();
    drop(bound);

    // A fresh Drive endpoint and a reader that has never met the author.
    let key = AppKey::load_or_generate(device_dir.path().join("key")).unwrap();
    let reader_key = AppKey::generate("announcement-reader");
    let new_addr = reserve_udp_address();
    let reader_addr = reserve_udp_address();
    let bound = endpoint(&key, new_addr).await;
    let reader_bound = endpoint(&reader_key, reader_addr).await;
    let reopened = drive(&bound, &key).await;
    // Before connecting the new reader, FIPS already knows the newer advert.
    assert!(
        !bound
            .native_endpoint
            .ingest_nostr_discovery_event(older)
            .await
            .unwrap()
    );
    let reader = FipsPubsubClient::start(
        reader_bound.native_endpoint.clone(),
        FipsPubsubClientOptions {
            query_timeout: Duration::from_secs(2),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    connect_endpoint_pair(
        &bound,
        &key,
        new_addr,
        &reader_bound,
        &reader_key,
        reader_addr,
    )
    .await;
    timeout(Duration::from_secs(10), async {
        while reader.connected_peer_count().unwrap() != 1 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("fresh reader connected to Drive");
    let events = reader
        .query(
            vec![Filter::new().id(valid.id)],
            QueryOptions { limit: Some(1) },
        )
        .await
        .unwrap()
        .events;
    assert_eq!(events.len(), 1, "valid announcement survived restart");
    assert_eq!(events[0].event.as_event(), &valid);
    assert_eq!(events[0].source.id.as_str(), key.pubkey_bech32());
    let rejected = reader
        .query(
            vec![Filter::new().ids([malformed.id, other_scope.id])],
            QueryOptions { limit: Some(2) },
        )
        .await
        .unwrap()
        .events;
    assert!(
        rejected.is_empty(),
        "rejected advertisements must not be replayed"
    );
    reader.shutdown().await;
    reader_bound.native_endpoint.shutdown().await.unwrap();
    reopened.shutdown().await.unwrap();
}
