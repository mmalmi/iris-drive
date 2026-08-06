#[allow(clippy::wildcard_imports)]
use super::*;
use iris_drive_core::nostr_identity::{
    NOSTR_IDENTITY_DEVICE_APPROVAL_APPLIED_ACK_SCHEMA, NostrIdentityDeviceApprovalAppliedAck,
    build_nostr_identity_device_approval_applied_ack_event,
};
use nostr_sdk::{Client, Event, EventBuilder, JsonUtil, Keys, Kind, Tag};

fn pending_request(
    config_dir: &std::path::Path,
) -> iris_drive_core::app_key_link_transport::AppKeyApprovalBootstrap {
    let config = AppConfig::load_or_default(config_path_in(config_dir)).unwrap();
    let pending = config
        .profile
        .as_ref()
        .and_then(|profile| profile.outbound_app_key_link_request.as_ref())
        .expect("pending request");
    iris_drive_core::app_key_link_transport::parse_pending_app_key_approval_bootstrap(pending)
        .unwrap()
        .0
}

fn event_has_type(event: &serde_json::Value, type_name: &str) -> bool {
    event["tags"].as_array().is_some_and(|tags| {
        tags.iter().any(|tag| {
            tag.as_array().is_some_and(|values| {
                values.first().and_then(serde_json::Value::as_str) == Some("type")
                    && values.get(1).and_then(serde_json::Value::as_str) == Some(type_name)
            })
        })
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approval_and_one_shot_sync_complete_the_durable_relay_handshake() {
    let relay = LocalNostrRelay::spawn().await;
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();

    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    let mut owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    owner_config.relays = vec![relay.url.clone()];
    owner_config.save(config_path_in(owner_dir.path())).unwrap();
    run_json(
        linked_dir.path(),
        &[
            "link",
            app_key_link_invite_url(&owner),
            "--label",
            "relay-device",
        ],
    );
    let request_url = relay.pending_approval_request_url(linked_dir.path()).await;
    let request = pending_request(linked_dir.path());
    assert_eq!(request.label.as_deref(), Some("relay-device"));

    let approved = run_json(owner_dir.path(), &["approve", &request_url]);
    assert_eq!(approved["roster_size"], 2);

    let saved_owner = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let owner_state = saved_owner.profile.as_ref().unwrap();
    let approval_events = relay.events().await;
    assert_eq!(
        approval_events
            .iter()
            .filter(|event| { event_has_type(event, "nostr_identity_device_approval_receipt") })
            .count(),
        1
    );
    assert_eq!(
        approval_events.len(),
        owner_state.profile_roster_ops.len() + 1,
        "the relay must receive the complete roster and encrypted receipt"
    );
    let receipt = Event::from_json(approval_events.last().unwrap().to_string()).unwrap();

    let hostile_client = Client::default();
    hostile_client.add_relay(&relay.url).await.unwrap();
    hostile_client.connect().await;
    let request_pubkey = nostr_sdk::PublicKey::parse(&request.request_npub).unwrap();
    for _ in 0..8 {
        let malformed_receipt = EventBuilder::new(
            Kind::from(iris_drive_core::KIND_NOSTR_IDENTITY_ROSTER_OP),
            "malformed receipt",
        )
        .tag(Tag::parse(["type", "nostr_identity_device_approval_receipt"]).unwrap())
        .tag(Tag::parse(["p", request_pubkey.to_hex().as_str()]).unwrap())
        .sign_with_keys(&Keys::generate())
        .unwrap();
        hostile_client.send_event(&malformed_receipt).await.unwrap();
    }

    let linked_app_key =
        iris_drive_core::AppKey::load(iris_drive_core::paths::key_path_in(linked_dir.path()))
            .unwrap();
    let malformed_ack = EventBuilder::new(
        Kind::from(iris_drive_core::KIND_NOSTR_IDENTITY_ROSTER_OP),
        "malformed acknowledgment",
    )
    .tag(Tag::parse(["type", "nostr_identity_device_approval_applied_ack"]).unwrap())
    .tag(Tag::parse(["p", owner_state.app_key_pubkey.as_str()]).unwrap())
    .tag(Tag::parse(["e", receipt.id.to_hex().as_str()]).unwrap())
    .sign_with_keys(linked_app_key.keys())
    .unwrap();
    hostile_client.send_event(&malformed_ack).await.unwrap();
    for applied_at in 1..=64 {
        let outsider = Keys::generate();
        let ack = build_nostr_identity_device_approval_applied_ack_event(
            &outsider,
            NostrIdentityDeviceApprovalAppliedAck {
                schema: NOSTR_IDENTITY_DEVICE_APPROVAL_APPLIED_ACK_SCHEMA,
                request_pubkey: Keys::generate().public_key().to_hex(),
                device_app_key_pubkey: outsider.public_key().to_hex(),
                approval_event_id: receipt.id.to_hex(),
                approved_by_pubkey: owner_state.app_key_pubkey.clone(),
                applied_at,
            },
        )
        .unwrap();
        hostile_client.send_event(&ack).await.unwrap();
    }

    let handshake_started = std::time::Instant::now();
    let synced = run_json(
        linked_dir.path(),
        &["sync", "--relay", &relay.url, "--timeout", "1"],
    );
    assert_eq!(synced["device_approval_receipts_applied"], 1);
    assert_eq!(synced["device_approval_applied_acks_published"], 1);
    assert_eq!(
        synced["device_approval_applied_ack_publish_errors"],
        serde_json::json!([])
    );
    assert_eq!(
        run_json(linked_dir.path(), &["status"])["profile"]["authorization_state"],
        "authorized"
    );

    let acknowledged = run_json(
        owner_dir.path(),
        &["sync", "--relay", &relay.url, "--timeout", "1"],
    );
    assert_eq!(acknowledged["device_approval_applied_acks_seen"], 1);
    assert_eq!(acknowledged["device_approval_applied_acks_applied"], 1);
    assert_eq!(
        run_json(owner_dir.path(), &["status"])["profile"]["pending_device_approval_receipt_count"],
        0
    );
    assert!(
        handshake_started.elapsed() <= std::time::Duration::from_secs(8),
        "durable one-shot approval handshake took {:?}",
        handshake_started.elapsed()
    );
}
