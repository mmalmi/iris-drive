//! A Drive daemon keeps serving its newest verified release head to fresh
//! update readers after unrelated pubsub traffic leaves its live cache.

use super::control::connect_endpoint_pair;
use super::mesh_fallback::{bind_test_endpoint, reserve_udp_address};
use super::*;
use hashtree_resolver::RootResolver;
use hashtree_updater::{PubsubRootResolver, UpdateRef};
use nostr_pubsub::{EventBus, EventSource, VerifiedEvent};
use nostr_pubsub_fips::{FipsPubsubClient, FipsPubsubClientOptions};
use nostr_sdk::{Alphabet, SingleLetterTag, Tag, TagKind, Timestamp};
use std::time::Duration;

const SCOPE: &str = "drive-release-head-test";
const TREE: &str = "releases/iris-drive";

fn release_head(keys: &Keys, created_at: u64, byte: u8) -> nostr_sdk::Event {
    EventBuilder::new(Kind::Custom(30_064), "")
        .tags([
            Tag::identifier(TREE),
            Tag::custom(
                TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::L)),
                ["hashtree"],
            ),
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
async fn drive_serves_newest_release_head_after_live_cache_eviction() {
    let daemon = AppKey::generate("release-head-daemon");
    let reader = AppKey::generate("release-head-reader");
    let rendezvous = reserve_udp_address();
    let daemon_addr = reserve_udp_address();
    let reader_addr = reserve_udp_address();
    let daemon_bound = bind_test_endpoint(&daemon, SCOPE, rendezvous, daemon_addr, false)
        .await
        .unwrap();
    let reader_bound = bind_test_endpoint(&reader, SCOPE, rendezvous, reader_addr, false)
        .await
        .unwrap();

    let release_keys = Keys::generate();
    let reference = UpdateRef {
        npub: release_keys.public_key().to_bech32().unwrap(),
        tree_name: TREE.into(),
        path: Some("latest".into()),
    };
    let mut runtime = DriveNostrPubsubRuntime::bind(
        daemon_bound.native_endpoint.clone(),
        Vec::new(),
        Vec::new(),
        None,
        nostr_runtime::drive_subscription_filters(SCOPE, &reference).unwrap(),
    )
    .await
    .unwrap();

    // The update exchange republishes its verified head; an older head must
    // never replace the newer one it already serves.
    let now = Timestamp::now().as_secs();
    let newest = release_head(&release_keys, now - 60, 0x42);
    runtime.publish(newest.clone()).await.unwrap();
    runtime
        .publish(release_head(&release_keys, now - 120, 0x41))
        .await
        .unwrap();
    // Ordinary traffic then fills the bounded live cache.
    let client = runtime.stack_fixture_client().unwrap();
    for n in 0..40 {
        let note = EventBuilder::text_note(format!("traffic {n}"))
            .sign_with_keys(&Keys::generate())
            .unwrap();
        EventBus::publish(
            client,
            VerifiedEvent::try_from(note).unwrap(),
            EventSource::local_index("test"),
        )
        .await
        .unwrap();
    }

    // A fresh, empty update reader that meets this daemon only now.
    let reader_client = Arc::new(
        FipsPubsubClient::start(
            reader_bound.native_endpoint.clone(),
            FipsPubsubClientOptions::default(),
        )
        .await
        .unwrap(),
    );
    connect_endpoint_pair(
        &daemon_bound,
        &daemon,
        daemon_addr,
        &reader_bound,
        &reader,
        reader_addr,
    )
    .await;
    let resolver = PubsubRootResolver::new(
        Arc::new(reader_client.fresh_subscriber()),
        Duration::from_secs(3),
    );
    let observed = resolver.resolve(&reference.resolver_key()).await;

    drop(resolver);
    reader_client.shutdown_shared().await;
    runtime.shutdown().await;
    reader_bound.native_endpoint.shutdown().await.unwrap();
    daemon_bound.native_endpoint.shutdown().await.unwrap();
    let root = observed
        .expect("the daemon must serve its retained release head to a fresh reader")
        .expect("release head");
    assert_eq!(hex::encode(root.hash), "42".repeat(32));
}
