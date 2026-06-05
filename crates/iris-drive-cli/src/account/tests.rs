use super::*;
use tempfile::tempdir;

#[test]
fn recover_app_key_command_uses_saved_phrase_after_profile_log_sync() {
    let owner_dir = tempdir().unwrap();
    let mut owner = Account::create(owner_dir.path(), Some("native".into())).unwrap();
    let phrase = iris_drive_core::recovery_phrase::load_recovery_phrase(
        iris_drive_core::paths::recovery_phrase_path_in(owner_dir.path()),
    )
    .unwrap();
    let owner_dck = owner.current_dck().unwrap();

    let recovered_dir = tempdir().unwrap();
    let recovered = Account::restore(recovered_dir.path(), &phrase, Some("browser".into()))
        .expect("restore from recovery phrase");
    let recovered_pubkey = recovered.state.device_pubkey.clone();
    let mut awaiting_state = recovered.state.clone();
    awaiting_state.profile_roster_ops = owner.state.profile_roster_ops.clone();
    awaiting_state.app_keys = None;
    awaiting_state.authorization_state =
        iris_drive_core::DeviceAuthorizationState::AwaitingApproval;
    let mut config = AppConfig {
        profile: Some(awaiting_state),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(owner.state.profile_id.to_string()));
    config.save(config_path_in(recovered_dir.path())).unwrap();

    cmd_recover_app_key(recovered_dir.path(), None, Some("Recovered browser".into())).unwrap();

    let saved = AppConfig::load_or_default(config_path_in(recovered_dir.path())).unwrap();
    let state = saved.profile.expect("recovered state saved");
    assert_eq!(
        state.authorization_state,
        iris_drive_core::DeviceAuthorizationState::Authorized
    );
    assert!(state.can_manage_devices());
    assert_eq!(
        state.profile_roster_ops.len(),
        owner.state.profile_roster_ops.len() + 2
    );
    assert_eq!(state.app_keys.as_ref().unwrap().dck_generation, 2);
    assert!(state.app_keys.as_ref().unwrap().is_admin(&recovered_pubkey));
    assert_eq!(
        state
            .app_keys
            .as_ref()
            .unwrap()
            .app_actor(&recovered_pubkey)
            .and_then(|device| device.label.as_deref()),
        Some("Recovered browser")
    );

    let recovered_account = Account::load(state.clone(), recovered_dir.path()).unwrap();
    let recovered_dck = recovered_account.current_dck().unwrap();
    assert_ne!(recovered_dck, owner_dck);
    owner.state.profile_roster_ops = state.profile_roster_ops;
    owner.state.sync_app_keys_from_profile();
    assert_eq!(owner.current_dck().unwrap(), recovered_dck);
}

#[tokio::test]
async fn device_link_app_message_records_inbound_request_for_owner_admin() {
    let config_dir = tempdir().unwrap();
    let account = Account::create(config_dir.path(), Some("admin".into())).unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(account.state.root_scope_id()));
    config.save(config_path_in(config_dir.path())).unwrap();

    let linked_device = nostr_sdk::Keys::generate().public_key().to_hex();
    let link_secret = account.state.device_link_secret.clone();
    let frame = DeviceLinkRequestFrame {
        schema: 1,
        profile_id: account.state.profile_id,
        device_pubkey: linked_device.clone(),
        link_secret: link_secret.clone(),
        label: Some(" phone ".into()),
        requested_at: 123,
        url: encode_device_approval_request(
            account.state.profile_id,
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
    let inbound = &saved.profile.unwrap().inbound_device_link_requests;
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
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(account.state.root_scope_id()));
    config.save(config_path_in(config_dir.path())).unwrap();

    let linked_device = nostr_sdk::Keys::generate().public_key().to_hex();
    let frame = DeviceLinkRequestFrame {
        schema: 1,
        profile_id: account.state.profile_id,
        device_pubkey: linked_device.clone(),
        link_secret: "wrong-secret".into(),
        label: Some("phone".into()),
        requested_at: 123,
        url: encode_device_approval_request(
            account.state.profile_id,
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
            .profile
            .unwrap()
            .inbound_device_link_requests
            .is_empty()
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn device_link_roster_message_authorizes_only_after_local_request() {
    let admin_dir = tempdir().unwrap();
    let mut admin = Account::create(admin_dir.path(), Some("admin".into())).unwrap();
    let joiner_dir = tempdir().unwrap();
    let joiner = Account::link_to_profile(
        joiner_dir.path(),
        admin.state.profile_id,
        admin.state.device_pubkey.clone(),
        Some("laptop".into()),
    )
    .unwrap();
    let joiner_pubkey = joiner.state.device_pubkey.clone();
    admin
        .approve_device(&joiner_pubkey, Some("laptop".into()))
        .unwrap();

    let frame = DeviceLinkRosterFrame {
        schema: 1,
        profile_id: admin.state.profile_id,
        admin_device_pubkey: admin.state.device_pubkey.clone(),
        profile_roster_ops: admin.state.profile_roster_ops.clone(),
        sent_at: 456,
    };
    let message = iris_drive_core::FipsAppMessage {
        peer_id: account_npub(&admin.state.device_pubkey),
        topic: DEVICE_LINK_ROSTER_APP_TOPIC.to_string(),
        data: serde_json::to_vec(&frame).unwrap(),
    };

    let mut config = AppConfig {
        profile: Some(joiner.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(admin.state.root_scope_id()));
    config.save(config_path_in(joiner_dir.path())).unwrap();

    assert!(
        handle_device_link_app_message(joiner_dir.path(), &message, None, &mut BTreeSet::new())
            .await
            .unwrap()
    );
    let saved = AppConfig::load_or_default(config_path_in(joiner_dir.path())).unwrap();
    let state = saved.profile.unwrap();
    assert_eq!(
        state.authorization_state,
        iris_drive_core::DeviceAuthorizationState::AwaitingApproval
    );
    assert!(state.app_keys.is_none());

    let mut requested = joiner.state.clone();
    requested
        .queue_outbound_device_link_request(
            admin.state.device_pubkey.clone(),
            &admin.state.device_link_secret,
            123,
        )
        .unwrap();
    let mut config = AppConfig {
        profile: Some(requested),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(admin.state.root_scope_id()));
    config.save(config_path_in(joiner_dir.path())).unwrap();

    assert!(
        handle_device_link_app_message(joiner_dir.path(), &message, None, &mut BTreeSet::new())
            .await
            .unwrap()
    );
    let saved = AppConfig::load_or_default(config_path_in(joiner_dir.path())).unwrap();
    assert_eq!(
        saved
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .unwrap()
            .root_scope_id,
        admin.state.profile_id.to_string()
    );
    let state = saved.profile.unwrap();
    assert_eq!(
        state.authorization_state,
        iris_drive_core::DeviceAuthorizationState::Authorized
    );
    assert!(state.outbound_device_link_request.is_none());
    assert!(state.app_keys.as_ref().unwrap().contains(&joiner_pubkey));

    let third_device = nostr_sdk::Keys::generate().public_key().to_hex();
    admin
        .approve_device(&third_device, Some("tablet".into()))
        .unwrap();
    let updated_frame = DeviceLinkRosterFrame {
        schema: 1,
        profile_id: admin.state.profile_id,
        admin_device_pubkey: admin.state.device_pubkey.clone(),
        profile_roster_ops: admin.state.profile_roster_ops.clone(),
        sent_at: 789,
    };
    let updated_message = iris_drive_core::FipsAppMessage {
        peer_id: account_npub(&admin.state.device_pubkey),
        topic: DEVICE_LINK_ROSTER_APP_TOPIC.to_string(),
        data: serde_json::to_vec(&updated_frame).unwrap(),
    };

    assert!(
        handle_device_link_app_message(
            joiner_dir.path(),
            &updated_message,
            None,
            &mut BTreeSet::new(),
        )
        .await
        .unwrap()
    );
    let saved = AppConfig::load_or_default(config_path_in(joiner_dir.path())).unwrap();
    let state = saved.profile.unwrap();
    assert!(state.app_keys.as_ref().unwrap().contains(&third_device));
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
        profile: Some(admin.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(admin.state.root_scope_id()));
    config.save(config_path_in(admin_dir.path())).unwrap();

    let frame = DeviceLinkRosterAckFrame {
        schema: 1,
        admin_device_pubkey: admin.state.device_pubkey.clone(),
        device_pubkey: joiner_pubkey.clone(),
        roster_fingerprint: device_link_roster_fingerprint(
            &joiner_pubkey,
            admin.state.profile_id,
            &admin.state.profile_roster_ops,
        ),
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

    assert!(acked.contains(&device_link_roster_fingerprint(
        &joiner_pubkey,
        admin.state.profile_id,
        &admin.state.profile_roster_ops,
    )));
}

#[test]
fn device_link_request_retry_uses_startup_burst_before_steady_interval() {
    let now = std::time::Instant::now();
    assert!(device_link_request_send_due(None, now));

    let first = SentDeviceLinkRequest {
        last_sent: now,
        attempts: 1,
    };
    assert!(!device_link_request_send_due(
        Some(first),
        now + std::time::Duration::from_millis(249)
    ));
    assert!(device_link_request_send_due(
        Some(first),
        now + std::time::Duration::from_millis(250)
    ));

    let steady = SentDeviceLinkRequest {
        last_sent: now,
        attempts: 40,
    };
    assert!(!device_link_request_send_due(
        Some(steady),
        now + std::time::Duration::from_secs(9)
    ));
    assert!(device_link_request_send_due(
        Some(steady),
        now + std::time::Duration::from_secs(10)
    ));
}
