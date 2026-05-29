use super::*;
use tempfile::tempdir;

#[tokio::test]
async fn device_link_app_message_records_inbound_request_for_owner_admin() {
    let config_dir = tempdir().unwrap();
    let account = Account::create(config_dir.path(), Some("admin".into())).unwrap();
    let mut config = AppConfig {
        account: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(&account.state.owner_pubkey));
    config.save(config_path_in(config_dir.path())).unwrap();

    let linked_device = nostr_sdk::Keys::generate().public_key().to_hex();
    let link_secret = account.state.device_link_secret.clone();
    let frame = DeviceLinkRequestFrame {
        schema: 1,
        owner_pubkey: account.state.owner_pubkey.clone(),
        device_pubkey: linked_device.clone(),
        link_secret: link_secret.clone(),
        label: Some(" phone ".into()),
        requested_at: 123,
        url: encode_device_approval_request(
            &account.state.owner_pubkey,
            &linked_device,
            &link_secret,
            Some(" phone "),
        ),
    };
    let message = iris_drive_core::FipsAppMessage {
        peer_id: account_npub(&linked_device),
        topic: DEVICE_LINK_REQUEST_APP_TOPIC.to_string(),
        data: serde_json::to_vec(&frame).unwrap(),
    };

    assert!(
        handle_device_link_app_message(config_dir.path(), &message, None, &mut BTreeSet::new())
            .await
            .unwrap()
    );

    let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    let inbound = &saved.account.unwrap().inbound_device_link_requests;
    assert_eq!(inbound.len(), 1);
    assert_eq!(inbound[0].device_pubkey, linked_device);
    assert_eq!(inbound[0].label.as_deref(), Some("phone"));
    assert_eq!(inbound[0].requested_at, 123);
}

#[tokio::test]
async fn device_link_app_message_ignores_wrong_link_secret() {
    let config_dir = tempdir().unwrap();
    let account = Account::create(config_dir.path(), Some("admin".into())).unwrap();
    let mut config = AppConfig {
        account: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(&account.state.owner_pubkey));
    config.save(config_path_in(config_dir.path())).unwrap();

    let linked_device = nostr_sdk::Keys::generate().public_key().to_hex();
    let frame = DeviceLinkRequestFrame {
        schema: 1,
        owner_pubkey: account.state.owner_pubkey.clone(),
        device_pubkey: linked_device.clone(),
        link_secret: "wrong-secret".into(),
        label: Some("phone".into()),
        requested_at: 123,
        url: encode_device_approval_request(
            &account.state.owner_pubkey,
            &linked_device,
            "wrong-secret",
            Some("phone"),
        ),
    };
    let message = iris_drive_core::FipsAppMessage {
        peer_id: account_npub(&linked_device),
        topic: DEVICE_LINK_REQUEST_APP_TOPIC.to_string(),
        data: serde_json::to_vec(&frame).unwrap(),
    };

    assert!(
        handle_device_link_app_message(config_dir.path(), &message, None, &mut BTreeSet::new())
            .await
            .unwrap()
    );

    let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    assert!(
        saved
            .account
            .unwrap()
            .inbound_device_link_requests
            .is_empty()
    );
}

#[tokio::test]
async fn device_link_roster_message_authorizes_only_after_local_request() {
    let admin_dir = tempdir().unwrap();
    let mut admin = Account::create(admin_dir.path(), Some("admin".into())).unwrap();
    let joiner_dir = tempdir().unwrap();
    let joiner = Account::link(
        joiner_dir.path(),
        admin.state.owner_pubkey.clone(),
        Some("laptop".into()),
    )
    .unwrap();
    let joiner_pubkey = joiner.state.device_pubkey.clone();
    admin
        .approve_device(&joiner_pubkey, Some("laptop".into()))
        .unwrap();
    let roster_event = iris_drive_core::nostr_events::build_app_keys_event(
        admin.device.keys(),
        admin.state.app_keys.as_ref().unwrap(),
    )
    .unwrap();

    let frame = DeviceLinkRosterFrame {
        schema: 1,
        owner_pubkey: admin.state.owner_pubkey.clone(),
        admin_device_pubkey: admin.state.device_pubkey.clone(),
        app_keys: admin.state.app_keys.clone().unwrap(),
        app_keys_event_id: roster_event.id.to_hex(),
        app_keys_event_json: roster_event.as_json(),
        sent_at: 456,
    };
    let message = iris_drive_core::FipsAppMessage {
        peer_id: account_npub(&admin.state.device_pubkey),
        topic: DEVICE_LINK_ROSTER_APP_TOPIC.to_string(),
        data: serde_json::to_vec(&frame).unwrap(),
    };

    let mut config = AppConfig {
        account: Some(joiner.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(&admin.state.owner_pubkey));
    config.save(config_path_in(joiner_dir.path())).unwrap();

    assert!(
        handle_device_link_app_message(joiner_dir.path(), &message, None, &mut BTreeSet::new())
            .await
            .unwrap()
    );
    let saved = AppConfig::load_or_default(config_path_in(joiner_dir.path())).unwrap();
    let state = saved.account.unwrap();
    assert_eq!(
        state.authorization_state,
        iris_drive_core::DeviceAuthorizationState::AwaitingApproval
    );
    assert!(state.app_keys.is_none());

    let mut requested = joiner.state.clone();
    requested
        .queue_outbound_device_link_request(
            admin.state.device_pubkey.clone(),
            admin.state.device_link_secret.clone(),
            123,
        )
        .unwrap();
    let mut config = AppConfig {
        account: Some(requested),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(&admin.state.owner_pubkey));
    config.save(config_path_in(joiner_dir.path())).unwrap();

    assert!(
        handle_device_link_app_message(joiner_dir.path(), &message, None, &mut BTreeSet::new())
            .await
            .unwrap()
    );
    let saved = AppConfig::load_or_default(config_path_in(joiner_dir.path())).unwrap();
    let state = saved.account.unwrap();
    assert_eq!(
        state.authorization_state,
        iris_drive_core::DeviceAuthorizationState::Authorized
    );
    assert!(state.outbound_device_link_request.is_none());
    assert!(state.app_keys.as_ref().unwrap().contains(&joiner_pubkey));
}

#[tokio::test]
async fn device_link_roster_ack_marks_delivery_for_admin() {
    let admin_dir = tempdir().unwrap();
    let mut admin = Account::create(admin_dir.path(), Some("admin".into())).unwrap();
    let joiner_pubkey = nostr_sdk::Keys::generate().public_key().to_hex();
    admin
        .approve_device(&joiner_pubkey, Some("laptop".into()))
        .unwrap();
    let mut config = AppConfig {
        account: Some(admin.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(&admin.state.owner_pubkey));
    config.save(config_path_in(admin_dir.path())).unwrap();

    let app_keys = admin.state.app_keys.as_ref().unwrap();
    let roster_event =
        iris_drive_core::nostr_events::build_app_keys_event(admin.device.keys(), app_keys).unwrap();
    let frame = DeviceLinkRosterAckFrame {
        schema: 1,
        owner_pubkey: admin.state.owner_pubkey.clone(),
        admin_device_pubkey: admin.state.device_pubkey.clone(),
        device_pubkey: joiner_pubkey.clone(),
        app_keys_event_id: roster_event.id.to_hex(),
        app_keys_created_at: app_keys.created_at,
        dck_generation: app_keys.dck_generation,
        acknowledged_at: 789,
    };
    let message = iris_drive_core::FipsAppMessage {
        peer_id: account_npub(&joiner_pubkey),
        topic: DEVICE_LINK_ROSTER_ACK_APP_TOPIC.to_string(),
        data: serde_json::to_vec(&frame).unwrap(),
    };
    let mut acked = BTreeSet::new();

    assert!(
        handle_device_link_app_message(admin_dir.path(), &message, None, &mut acked)
            .await
            .unwrap()
    );

    assert!(acked.contains(&device_link_roster_fingerprint(&joiner_pubkey, app_keys)));
}
