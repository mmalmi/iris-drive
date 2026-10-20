#[allow(clippy::wildcard_imports)]
use super::*;
use iris_drive_core::nostr_identity::{
    NOSTR_IDENTITY_DEVICE_APPROVAL_APPLIED_ACK_SCHEMA, NostrIdentityDeviceApprovalAppliedAck,
    build_nostr_identity_device_approval_applied_ack_event,
};
use nostr_sdk::{Client, Event, EventBuilder, JsonUtil, Keys, Kind, Tag};
use tokio::net::TcpListener;

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

async fn unresponsive_relay() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            tokio::spawn(async move {
                let _socket = socket;
                std::future::pending::<()>().await;
            });
        }
    });
    (format!("ws://{address}"), task)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn app_keys_approve_hands_current_root_to_new_device_before_receipt() {
    let relay = LocalNostrRelay::spawn().await;
    let blossom = LocalBlossomServer::spawn().await;
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let owner = run_json(owner_dir.path(), &["init", "--label", "Native owner"]);
    let _source = import_one_file(
        owner_dir.path(),
        "before-approval.txt",
        b"bytes created before browser approval",
    );
    configure_local_blossom(owner_dir.path(), &blossom.url);
    add_config_relay(owner_dir.path(), &relay.url);

    let linked = run_json(
        linked_dir.path(),
        &[
            "app-keys",
            "request",
            app_key_link_invite_url(&owner),
            "--label",
            "Web browser",
        ],
    );
    let request = LocalNostrRelay::pending_approval_request_url(linked_dir.path());
    let approved = run_json(owner_dir.path(), &["app-keys", "approve", request.as_str()]);
    assert_eq!(
        approved["approved_app_key_npub"],
        linked["current_app_key_npub"]
    );
    assert_eq!(approved["published_drive_root"], true);

    let events = relay.events().await;
    let parsed = events
        .iter()
        .map(|value| Event::from_json(value.to_string()).unwrap())
        .collect::<Vec<_>>();
    let receipt_index = parsed
        .iter()
        .position(iris_drive_core::relay_sync::is_device_approval_receipt_event)
        .expect("approval receipt published");
    let (drive_root_index, drive_root) = parsed
        .iter()
        .enumerate()
        .find(|(_, event)| event.kind.as_u16() == iris_drive_core::nostr_events::KIND_DRIVE_ROOT)
        .expect("Drive root handoff published");
    assert!(
        drive_root_index < receipt_index,
        "the decryptable Drive root must be accepted before the approval receipt"
    );

    let linked_key =
        iris_drive_core::AppKey::load(iris_drive_core::paths::key_path_in(linked_dir.path()))
            .unwrap();
    let (_, _, _, handed_off_root) =
        iris_drive_core::nostr_events::parse_drive_root_event_for_device(
            drive_root,
            linked_key.keys(),
        )
        .expect("newly approved AppKey can decrypt the root-key wrap");
    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let owner_state = owner_config.profile.as_ref().unwrap();
    let expected_root = owner_config
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&owner_state.app_key_pubkey)
        .unwrap();
    assert_eq!(handed_off_root.root_cid, expected_root.root_cid);
    let owner_profile = iris_drive_core::Profile::load(owner_state.clone(), owner_dir.path())
        .expect("load approved owner profile");
    let dck = owner_profile.current_dck().expect("current approval DCK");
    let current_labels = parsed
        .iter()
        .flat_map(|event| {
            iris_drive_core::nostr_identity::encrypted_device_label_payloads_from_nostr_identity_roster_op_event(
                event,
            )
        })
        .filter_map(|ciphertext| {
            iris_drive_core::device_labels::decrypt_drive_device_labels_with_dck(&ciphertext, &dck)
                .ok()
        })
        .max_by_key(|payload| (payload.secret_epoch, payload.updated_at))
        .expect("relay publishes current-DCK device labels");
    assert_eq!(
        current_labels
            .labels
            .get(&owner_state.app_key_pubkey)
            .map(String::as_str),
        Some("Native owner")
    );
    assert_eq!(
        current_labels
            .labels
            .get(&linked_key.pubkey_hex())
            .map(String::as_str),
        Some("Web browser")
    );
    assert!(
        blossom.blob_count().await > 0,
        "root metadata must not outrun its blocks during approval"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approval_without_block_destination_publishes_no_authorization_or_receipt() {
    let relay = LocalNostrRelay::spawn().await;
    let blossom = LocalBlossomServer::spawn().await;
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let owner = run_json(owner_dir.path(), &["init", "--label", "iPhone"]);
    idrive(owner_dir.path())
        .args(["blossom-servers", "remove", "https://upload.iris.to"])
        .assert()
        .success();
    add_config_relay(owner_dir.path(), &relay.url);
    run_json(
        linked_dir.path(),
        &[
            "app-keys",
            "request",
            app_key_link_invite_url(&owner),
            "--label",
            "Web browser",
        ],
    );
    let request = LocalNostrRelay::pending_approval_request_url(linked_dir.path());

    let approved = run_json(owner_dir.path(), &["app-keys", "approve", &request]);

    assert_eq!(approved["roster_size"], 2, "local approval remains durable");
    assert_eq!(approved["published_approval_events"], 0);
    assert!(
        approved["approval_publish_error"]
            .as_str()
            .is_some_and(|error| error.contains("no Blossom servers configured"))
    );
    assert!(
        relay.events().await.is_empty(),
        "Blossom failure must happen before roster authorization reaches a relay"
    );

    configure_local_blossom(owner_dir.path(), &blossom.url);
    let retried = run_json(
        owner_dir.path(),
        &["publish", "--relay", &relay.url, "--timeout", "2"],
    );
    assert_eq!(retried["published_drive_root"], true);
    assert_eq!(retried["published_device_approval_receipts"], 1);
    let events = relay.events().await;
    let drive_root_index = events
        .iter()
        .position(|event| event["kind"] == iris_drive_core::nostr_events::KIND_DRIVE_ROOT)
        .expect("retry publishes the uploaded Drive root");
    let receipt_index = events
        .iter()
        .position(|event| {
            Event::from_json(event.to_string()).is_ok_and(|event| {
                iris_drive_core::relay_sync::is_device_approval_receipt_event(&event)
            })
        })
        .expect("retry publishes the approval receipt");
    assert!(
        drive_root_index < receipt_index,
        "retry must retain root-before-receipt ordering"
    );
    assert!(blossom.blob_count().await > 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn approval_and_one_shot_sync_complete_the_durable_relay_handshake() {
    let relay = LocalNostrRelay::spawn().await;
    let blossom = LocalBlossomServer::spawn().await;
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();

    let owner = run_json(owner_dir.path(), &["init", "--label", "admin"]);
    configure_local_blossom(owner_dir.path(), &blossom.url);
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
    configure_local_blossom(linked_dir.path(), &blossom.url);
    let request_url = LocalNostrRelay::pending_approval_request_url(linked_dir.path());
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
        owner_state.profile_roster_ops.len() + 2,
        "the relay must receive the complete roster, Drive root, and encrypted receipt"
    );
    assert_eq!(
        approval_events[approval_events.len() - 2]["kind"],
        iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        "the resolvable empty Drive root must immediately precede the receipt"
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
    let (slow_relay, slow_relay_task) = unresponsive_relay().await;
    let mut owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    owner_config.relays = vec![slow_relay, relay.url.clone()];
    owner_config.save(config_path_in(owner_dir.path())).unwrap();
    let delayed_ack = build_nostr_identity_device_approval_applied_ack_event(
        linked_app_key.keys(),
        NostrIdentityDeviceApprovalAppliedAck {
            schema: NOSTR_IDENTITY_DEVICE_APPROVAL_APPLIED_ACK_SCHEMA,
            request_pubkey: request_pubkey.to_hex(),
            device_app_key_pubkey: linked_app_key.keys().public_key().to_hex(),
            approval_event_id: receipt.id.to_hex(),
            approved_by_pubkey: owner_state.app_key_pubkey.clone(),
            applied_at: 1,
        },
    )
    .unwrap();
    let delayed_client = Client::default();
    delayed_client.add_relay(&relay.url).await.unwrap();
    delayed_client.connect().await;
    let delayed_ack_publish = tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        delayed_client.send_event(&delayed_ack).await.unwrap();
        delayed_client.shutdown().await;
    });
    let ack_started = std::time::Instant::now();
    let acknowledged = iris_drive_core::sync_pending_device_approval_acks(
        owner_dir.path(),
        &[],
        std::time::Duration::from_secs(2),
    )
    .await
    .unwrap();
    let ack_elapsed = ack_started.elapsed();
    delayed_ack_publish.await.unwrap();
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

    assert_eq!(acknowledged.device_approval_applied_acks_seen, 1);
    assert_eq!(acknowledged.device_approval_applied_acks_applied, 1);
    assert!(
        ack_elapsed < std::time::Duration::from_secs(1),
        "responsive relay ACK was queued behind a slow relay for {ack_elapsed:?}"
    );
    slow_relay_task.abort();
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
