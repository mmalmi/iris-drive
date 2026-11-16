use std::time::Duration;

use nostr_sdk::{Event, Tag};

use super::control::connect_endpoint_pair;
use super::mesh_fallback::{bind_test_endpoint, reserve_udp_address};
use super::*;

fn drive_root(device: &AppKey, label: &str) -> Event {
    EventBuilder::new(Kind::Custom(30_078), label)
        .tags([Tag::identifier(format!(
            "iris-drive/feed-test/primary/{label}"
        ))])
        .sign_with_keys(device.keys())
        .unwrap()
}

async fn next_drive_event(
    received: &mut tokio::sync::broadcast::Receiver<FipsNostrPubsubEvent>,
    expected: &Event,
) -> FipsNostrPubsubEvent {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let delivery = received.recv().await.unwrap();
            if delivery.event.id == expected.id {
                break delivery;
            }
            assert!(
                !is_drive_feed_event(&delivery.event),
                "unexpected Drive event {:?}",
                delivery.event
            );
        }
    })
    .await
    .expect("Drive event over the FIPS feed")
}

#[tokio::test]
async fn own_devices_exchange_only_their_missing_drive_events_over_the_fips_feed() {
    let alice = AppKey::generate("feed-alice");
    let bob = AppKey::generate("feed-bob");
    let rendezvous = reserve_udp_address();
    let (alice_addr, bob_addr) = (reserve_udp_address(), reserve_udp_address());
    let alice_bound = bind_test_endpoint(&alice, "drive-feed-test", rendezvous, alice_addr, false)
        .await
        .unwrap();
    let bob_bound = bind_test_endpoint(&bob, "drive-feed-test", rendezvous, bob_addr, false)
        .await
        .unwrap();
    let reference = hashtree_updater::UpdateRef {
        npub: Keys::generate().public_key().to_bech32().unwrap(),
        tree_name: "releases/iris-drive".to_string(),
        path: Some("latest".to_string()),
    };
    let filters = nostr_runtime::drive_subscription_filters("drive-feed-test", &reference).unwrap();
    let bind = |endpoint| {
        DriveNostrPubsubRuntime::bind(endpoint, Vec::new(), Vec::new(), None, filters.clone())
    };
    let mut alice_runtime = bind(alice_bound.native_endpoint.clone()).await.unwrap();
    let mut bob_runtime = bind(bob_bound.native_endpoint.clone()).await.unwrap();
    let (mut alice_received, mut bob_received) =
        (alice_runtime.subscribe(), bob_runtime.subscribe());

    // Both devices already hold the shared root; each holds one the other lacks.
    let shared = drive_root(&alice, "shared");
    let alice_root = drive_root(&alice, "alice");
    let bob_root = drive_root(&bob, "bob");
    let unrelated = EventBuilder::text_note("not a Drive event")
        .sign_with_keys(alice.keys())
        .unwrap();
    alice_runtime
        .admit_drive_feed_peers(
            vec![bob.pubkey_bech32()],
            vec![shared.clone(), alice_root.clone(), unrelated],
        )
        .await;
    bob_runtime
        .admit_drive_feed_peers(vec![alice.pubkey_bech32()], vec![shared, bob_root.clone()])
        .await;
    connect_endpoint_pair(&alice_bound, &alice, alice_addr, &bob_bound, &bob, bob_addr).await;

    let delivery = next_drive_event(&mut bob_received, &alice_root).await;
    assert_eq!(delivery.origin_peer_id, alice.pubkey_bech32());
    next_drive_event(&mut alice_received, &bob_root).await;

    let live = drive_root(&alice, "live");
    alice_runtime
        .admit_drive_feed_peers(vec![bob.pubkey_bech32()], vec![live.clone()])
        .await;
    next_drive_event(&mut bob_received, &live).await;
    // Nothing else of the feed arrives: the shared root was never resent.
    tokio::time::sleep(Duration::from_millis(500)).await;
    while let Ok(delivery) = bob_received.try_recv() {
        assert!(
            !is_drive_feed_event(&delivery.event),
            "{:?}",
            delivery.event
        );
    }

    alice_runtime.shutdown().await;
    bob_runtime.shutdown().await;
    alice_bound.native_endpoint.shutdown().await.unwrap();
    bob_bound.native_endpoint.shutdown().await.unwrap();
}
