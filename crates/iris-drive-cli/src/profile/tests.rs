use super::*;
use tempfile::tempdir;

#[test]
fn config_fingerprint_changes_when_profile_roster_sidecar_changes() {
    let dir = tempdir().unwrap();
    let config_path = config_path_in(dir.path());
    std::fs::write(&config_path, "schema_version = 1\n").unwrap();
    let initial = config_file_fingerprint(&config_path).unwrap();

    std::fs::write(
        dir.path().join("profile-roster-events.json"),
        r#"{"schema_version":1,"events":["event-a"]}"#,
    )
    .unwrap();
    let changed = config_file_fingerprint(&config_path).unwrap();

    assert_ne!(initial, changed);
    assert!(changed.profile_roster_events_len > 0);
}

#[test]
fn config_fingerprint_hashes_same_length_config_and_roster_changes() {
    let dir = tempdir().unwrap();
    let config_path = config_path_in(dir.path());
    let roster_path = dir.path().join("profile-roster-events.json");
    std::fs::write(
        &config_path,
        "schema_version = 1\nrelays = [\"wss://one.example\"]\n",
    )
    .unwrap();
    std::fs::write(&roster_path, r#"{"schema_version":1,"events":["event-a"]}"#).unwrap();
    let initial = config_file_fingerprint(&config_path).unwrap();

    std::fs::write(
        &config_path,
        "schema_version = 1\nrelays = [\"wss://two.example\"]\n",
    )
    .unwrap();
    std::fs::write(&roster_path, r#"{"schema_version":1,"events":["event-b"]}"#).unwrap();
    let changed = config_file_fingerprint(&config_path).unwrap();

    assert_eq!(initial.len, changed.len);
    assert_eq!(
        initial.profile_roster_events_len,
        changed.profile_roster_events_len
    );
    assert_ne!(initial.content_hash, changed.content_hash);
    assert_ne!(
        initial.profile_roster_events_hash,
        changed.profile_roster_events_hash
    );
}

#[test]
fn load_app_config_cached_invalidates_same_length_config_change() {
    let dir = tempdir().unwrap();
    let config_path = config_path_in(dir.path());
    std::fs::write(
        &config_path,
        "schema_version = 1\nrelays = [\"wss://one.example\"]\n",
    )
    .unwrap();
    let mut cache = AppConfigLoadCache::default();

    let first = load_app_config_cached(&config_path, &mut cache).unwrap();
    assert_eq!(first.relays, vec!["wss://one.example"]);

    std::fs::write(
        &config_path,
        "schema_version = 1\nrelays = [\"wss://two.example\"]\n",
    )
    .unwrap();
    let changed = load_app_config_cached(&config_path, &mut cache).unwrap();

    assert_eq!(changed.relays, vec!["wss://two.example"]);
}

#[test]
fn recover_app_key_command_uses_saved_phrase_after_profile_log_sync() {
    let owner_dir = tempdir().unwrap();
    let phrase = iris_drive_core::recovery_phrase::generate_recovery_phrase().unwrap();
    let mut owner = Profile::restore(owner_dir.path(), &phrase, Some("native".into())).unwrap();
    let owner_dck = owner.current_dck().unwrap();

    let recovered_dir = tempdir().unwrap();
    let recovered = Profile::restore(recovered_dir.path(), &phrase, Some("browser".into()))
        .expect("restore from recovery phrase");
    let recovered_pubkey = recovered.state.app_key_pubkey.clone();
    let mut awaiting_state = recovered.state.clone();
    awaiting_state.profile_roster_ops = owner.state.profile_roster_ops.clone();
    awaiting_state.app_keys = None;
    awaiting_state.authorization_state =
        iris_drive_core::AppKeyAuthorizationState::AwaitingApproval;
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
        iris_drive_core::AppKeyAuthorizationState::Authorized
    );
    assert!(state.can_admin_profile());
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

    let recovered_account = Profile::load(state.clone(), recovered_dir.path()).unwrap();
    let recovered_dck = recovered_account.current_dck().unwrap();
    assert_eq!(recovered_dck, owner_dck);
    owner.state.profile_roster_ops = state.profile_roster_ops;
    owner.state.sync_app_keys_from_profile();
    assert_eq!(owner.current_dck().unwrap(), recovered_dck);
}

#[test]
fn revoke_command_can_use_recovery_secret() {
    let dir = tempdir().unwrap();
    let phrase = iris_drive_core::recovery_phrase::generate_recovery_phrase().unwrap();
    let mut profile = Profile::restore(dir.path(), &phrase, Some("native".into())).unwrap();
    let linked_app_key = nostr_sdk::Keys::generate().public_key().to_hex();
    profile
        .approve_app_key(&linked_app_key, Some("phone".into()))
        .unwrap();
    let gen_before = profile.state.app_keys.as_ref().unwrap().dck_generation;
    let mut config = AppConfig {
        profile: Some(profile.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(profile.state.profile_id.to_string()));
    config.save(config_path_in(dir.path())).unwrap();
    let recovery_nsec = iris_drive_core::recovery_phrase::recovery_phrase_to_nsec(&phrase)
        .expect("recovery phrase derives nsec");

    cmd_revoke(dir.path(), &linked_app_key, Some(&recovery_nsec)).unwrap();

    let saved = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    let state = saved.profile.expect("profile saved");
    let snap = state.app_keys.as_ref().expect("app keys projection");
    assert!(snap.dck_generation > gen_before);
    assert!(!snap.contains(&linked_app_key));
    assert!(!snap.wrapped_dck.contains_key(&linked_app_key));
    assert!(state.can_admin_profile());
}

#[test]
fn app_keys_rename_command_updates_device_label() {
    let dir = tempdir().unwrap();
    let mut profile = Profile::create(dir.path(), Some("native".into())).unwrap();
    let linked_app_key = nostr_sdk::Keys::generate().public_key().to_hex();
    profile
        .approve_app_key(&linked_app_key, Some("phone".into()))
        .unwrap();
    let mut config = AppConfig {
        profile: Some(profile.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(profile.state.profile_id.to_string()));
    config.save(config_path_in(dir.path())).unwrap();

    cmd_rename_app_key(dir.path(), &linked_app_key, "iPhone").unwrap();

    let saved = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    let state = saved.profile.expect("profile saved");
    let profile = Profile::load(state, dir.path()).unwrap();
    assert_eq!(
        profile
            .state
            .app_keys
            .as_ref()
            .unwrap()
            .app_actor(&linked_app_key)
            .and_then(|device| device.label.as_deref()),
        Some("iPhone")
    );
}

#[tokio::test]
async fn app_key_link_app_message_records_inbound_request_for_owner_admin() {
    let config_dir = tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("admin".into())).unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(account.state.root_scope_id()));
    config.save(config_path_in(config_dir.path())).unwrap();

    let linked_device = nostr_sdk::Keys::generate();
    let linked_device_hex = linked_device.public_key().to_hex();
    let invite_pubkey =
        iris_drive_core::app_key_link_invite_pubkey(&account.state.app_key_link_secret).unwrap();
    let approval_request =
        iris_drive_core::app_key_link_transport::create_app_key_approval_bootstrap(
            &linked_device,
            Some(" phone "),
        )
        .unwrap();
    let frame = AppKeyLinkRequestFrame {
        schema: 1,
        invite_pubkey: invite_pubkey.clone(),
        label: Some(" phone ".into()),
        request_npub: approval_request.bootstrap.request_npub,
        request_secret: approval_request.bootstrap.request_secret,
    };
    let message = iris_drive_core::FipsAppMessage {
        peer_id: pubkey_npub(&linked_device_hex),
        topic: APP_KEY_LINK_REQUEST_APP_TOPIC.to_string(),
        data: serde_json::to_vec(&frame).unwrap(),
    };

    assert!(
        handle_app_key_link_app_message(config_dir.path(), &message, None, &mut BTreeSet::new())
            .await
            .unwrap()
    );

    let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    let inbound = &saved.profile.unwrap().inbound_app_key_link_requests;
    assert_eq!(inbound.len(), 1);
    assert_eq!(inbound[0].app_key_pubkey, linked_device_hex);
    assert_eq!(inbound[0].label.as_deref(), Some("phone"));
    assert!(inbound[0].requested_at > 0);
}

#[tokio::test]
async fn app_key_link_app_message_ignores_wrong_link_secret() {
    let config_dir = tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("admin".into())).unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(account.state.root_scope_id()));
    config.save(config_path_in(config_dir.path())).unwrap();

    let linked_device = nostr_sdk::Keys::generate();
    let linked_device_hex = linked_device.public_key().to_hex();
    let wrong_invite_pubkey = nostr_sdk::Keys::generate().public_key().to_hex();
    let approval_request =
        iris_drive_core::app_key_link_transport::create_app_key_approval_bootstrap(
            &linked_device,
            Some("phone"),
        )
        .unwrap();
    let frame = AppKeyLinkRequestFrame {
        schema: 1,
        invite_pubkey: wrong_invite_pubkey,
        label: Some("phone".into()),
        request_npub: approval_request.bootstrap.request_npub,
        request_secret: approval_request.bootstrap.request_secret,
    };
    let message = iris_drive_core::FipsAppMessage {
        peer_id: pubkey_npub(&linked_device_hex),
        topic: APP_KEY_LINK_REQUEST_APP_TOPIC.to_string(),
        data: serde_json::to_vec(&frame).unwrap(),
    };

    assert!(
        handle_app_key_link_app_message(config_dir.path(), &message, None, &mut BTreeSet::new())
            .await
            .unwrap()
    );

    let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    assert!(
        saved
            .profile
            .unwrap()
            .inbound_app_key_link_requests
            .is_empty()
    );
}

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn app_key_link_roster_message_authorizes_only_after_local_request() {
    let admin_dir = tempdir().unwrap();
    let mut admin = Profile::create(admin_dir.path(), Some("admin".into())).unwrap();
    let joiner_dir = tempdir().unwrap();
    let joiner = Profile::link_to_profile(
        joiner_dir.path(),
        admin.state.profile_id,
        admin.state.app_key_pubkey.clone(),
        Some("laptop".into()),
    )
    .unwrap();
    let joiner_pubkey = joiner.state.app_key_pubkey.clone();
    let approval_request =
        iris_drive_core::app_key_link_transport::create_app_key_approval_bootstrap(
            joiner.app_key.keys(),
            joiner.state.app_key_label.as_deref(),
        )
        .unwrap();
    admin
        .approve_device_bootstrap(&approval_request.bootstrap, Some("laptop".into()))
        .unwrap();

    let frame = AppKeyLinkRosterFrame {
        schema: 1,
        profile_id: admin.state.profile_id,
        admin_app_key_pubkey: admin.state.app_key_pubkey.clone(),
        profile_roster_ops: admin.state.profile_roster_ops.clone(),
        sent_at: 456,
    };
    let message = iris_drive_core::FipsAppMessage {
        peer_id: pubkey_npub(&admin.state.app_key_pubkey),
        topic: APP_KEY_LINK_ROSTER_APP_TOPIC.to_string(),
        data: serde_json::to_vec(&frame).unwrap(),
    };
    let receipt_message = iris_drive_core::FipsAppMessage {
        peer_id: pubkey_npub(&admin.state.app_key_pubkey),
        topic: APP_KEY_APPROVAL_RECEIPT_APP_TOPIC.to_string(),
        data: admin
            .state
            .pending_device_approval_receipts
            .last()
            .expect("approval receipt")
            .event_json
            .as_bytes()
            .to_vec(),
    };

    let mut config = AppConfig {
        profile: Some(joiner.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(admin.state.root_scope_id()));
    config.save(config_path_in(joiner_dir.path())).unwrap();

    assert!(
        handle_app_key_link_app_message(joiner_dir.path(), &message, None, &mut BTreeSet::new())
            .await
            .unwrap()
    );
    let saved = AppConfig::load_or_default(config_path_in(joiner_dir.path())).unwrap();
    let state = saved.profile.unwrap();
    assert_eq!(
        state.authorization_state,
        iris_drive_core::AppKeyAuthorizationState::AwaitingApproval
    );
    assert!(state.app_keys.is_none());

    let mut requested = joiner.state.clone();
    requested
        .queue_outbound_app_key_link_request(
            admin.state.app_key_pubkey.clone(),
            &iris_drive_core::app_key_link_invite_pubkey(&admin.state.app_key_link_secret).unwrap(),
            123,
            approval_request.url,
            approval_request.request_keys.secret_key().to_secret_hex(),
        )
        .unwrap();
    let mut config = AppConfig {
        profile: Some(requested),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(admin.state.root_scope_id()));
    config.save(config_path_in(joiner_dir.path())).unwrap();

    assert!(
        handle_app_key_link_app_message(
            joiner_dir.path(),
            &receipt_message,
            None,
            &mut BTreeSet::new(),
        )
        .await
        .unwrap()
    );
    assert!(
        handle_app_key_link_app_message(joiner_dir.path(), &message, None, &mut BTreeSet::new())
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
        iris_drive_core::AppKeyAuthorizationState::Authorized
    );
    assert!(state.outbound_app_key_link_request.is_none());
    assert!(state.app_keys.as_ref().unwrap().contains(&joiner_pubkey));

    let third_device = nostr_sdk::Keys::generate().public_key().to_hex();
    admin
        .approve_app_key(&third_device, Some("tablet".into()))
        .unwrap();
    let updated_frame = AppKeyLinkRosterFrame {
        schema: 1,
        profile_id: admin.state.profile_id,
        admin_app_key_pubkey: admin.state.app_key_pubkey.clone(),
        profile_roster_ops: admin.state.profile_roster_ops.clone(),
        sent_at: 789,
    };
    let updated_message = iris_drive_core::FipsAppMessage {
        peer_id: pubkey_npub(&admin.state.app_key_pubkey),
        topic: APP_KEY_LINK_ROSTER_APP_TOPIC.to_string(),
        data: serde_json::to_vec(&updated_frame).unwrap(),
    };

    assert!(
        handle_app_key_link_app_message(
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
async fn app_key_link_roster_ack_marks_delivery_for_admin() {
    let admin_dir = tempdir().unwrap();
    let mut admin = Profile::create(admin_dir.path(), Some("admin".into())).unwrap();
    let joiner_pubkey = nostr_sdk::Keys::generate().public_key().to_hex();
    admin
        .approve_app_key(&joiner_pubkey, Some("laptop".into()))
        .unwrap();
    let mut config = AppConfig {
        profile: Some(admin.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(admin.state.root_scope_id()));
    config.save(config_path_in(admin_dir.path())).unwrap();

    let frame = AppKeyLinkRosterAckFrame {
        schema: 1,
        admin_app_key_pubkey: admin.state.app_key_pubkey.clone(),
        app_key_pubkey: joiner_pubkey.clone(),
        roster_fingerprint: app_key_link_roster_fingerprint(
            &joiner_pubkey,
            admin.state.profile_id,
            &admin.state.profile_roster_ops,
        ),
        acknowledged_at: 789,
    };
    let message = iris_drive_core::FipsAppMessage {
        peer_id: pubkey_npub(&joiner_pubkey),
        topic: APP_KEY_LINK_ROSTER_ACK_APP_TOPIC.to_string(),
        data: serde_json::to_vec(&frame).unwrap(),
    };
    let mut acked = BTreeSet::new();

    assert!(
        handle_app_key_link_app_message(admin_dir.path(), &message, None, &mut acked)
            .await
            .unwrap()
    );

    assert!(acked.contains(&app_key_link_roster_fingerprint(
        &joiner_pubkey,
        admin.state.profile_id,
        &admin.state.profile_roster_ops,
    )));
}

#[test]
fn app_key_link_request_retry_uses_startup_burst_before_steady_interval() {
    let now = std::time::Instant::now();
    assert!(app_key_link_request_send_due(None, now));

    let first = SentAppKeyLinkRequest {
        last_sent: now,
        attempts: 1,
    };
    assert!(!app_key_link_request_send_due(
        Some(first),
        now + std::time::Duration::from_millis(999)
    ));
    assert!(app_key_link_request_send_due(
        Some(first),
        now + std::time::Duration::from_secs(1)
    ));

    let steady = SentAppKeyLinkRequest {
        last_sent: now,
        attempts: 40,
    };
    assert!(!app_key_link_request_send_due(
        Some(steady),
        now + std::time::Duration::from_secs(9)
    ));
    assert!(app_key_link_request_send_due(
        Some(steady),
        now + std::time::Duration::from_secs(10)
    ));
}

#[test]
fn app_key_link_roster_retry_uses_short_burst_then_steady_interval() {
    let now = std::time::Instant::now();
    assert!(app_key_link_roster_send_due(None, now));

    let first = SentAppKeyLinkRoster {
        last_sent: now,
        attempts: 1,
    };
    assert!(!app_key_link_roster_send_due(
        Some(first),
        now + std::time::Duration::from_secs(1)
    ));
    assert!(app_key_link_roster_send_due(
        Some(first),
        now + std::time::Duration::from_secs(2)
    ));

    let steady = SentAppKeyLinkRoster {
        last_sent: now,
        attempts: APP_KEY_LINK_ROSTER_STARTUP_BURST_ATTEMPTS,
    };
    assert!(!app_key_link_roster_send_due(
        Some(steady),
        now + std::time::Duration::from_secs(59)
    ));
    assert!(app_key_link_roster_send_due(
        Some(steady),
        now + std::time::Duration::from_mins(1)
    ));
}

#[test]
fn app_key_link_roster_startup_burst_covers_authorization_window() {
    assert!(
        u64::from(APP_KEY_LINK_ROSTER_STARTUP_BURST_ATTEMPTS)
            * APP_KEY_LINK_ROSTER_STARTUP_RETRY_SECS
            >= 60
    );
}

#[test]
fn authorized_roster_snapshot_cache_reuses_unchanged_config_and_invalidates_on_save() {
    let admin_dir = tempdir().unwrap();
    let mut admin = Profile::create(admin_dir.path(), Some("admin".into())).unwrap();
    let first_joiner = nostr_sdk::Keys::generate().public_key().to_hex();
    admin
        .approve_app_key(&first_joiner, Some("laptop".into()))
        .unwrap();
    let mut config = AppConfig {
        profile: Some(admin.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(admin.state.root_scope_id()));
    config.save(config_path_in(admin_dir.path())).unwrap();

    let mut cache = AuthorizedAppKeyLinkRosterSendCache::default();
    let first = load_authorized_app_key_link_roster_snapshot(admin_dir.path(), &mut cache)
        .unwrap()
        .expect("admin with linked app key has a roster snapshot");
    assert_eq!(first.recipients.len(), 1);

    cache
        .snapshot
        .as_mut()
        .unwrap()
        .snapshot
        .as_mut()
        .unwrap()
        .frame_bytes = b"cached".to_vec();
    let cached = load_authorized_app_key_link_roster_snapshot(admin_dir.path(), &mut cache)
        .unwrap()
        .expect("unchanged config reuses snapshot");
    assert_eq!(cached.frame_bytes, b"cached");

    let second_joiner = nostr_sdk::Keys::generate().public_key().to_hex();
    admin
        .approve_app_key(&second_joiner, Some("tablet".into()))
        .unwrap();
    config.profile = Some(admin.state.clone());
    config.save(config_path_in(admin_dir.path())).unwrap();

    let refreshed = load_authorized_app_key_link_roster_snapshot(admin_dir.path(), &mut cache)
        .unwrap()
        .expect("changed config refreshes snapshot");
    assert_eq!(refreshed.recipients.len(), 2);
    assert_ne!(refreshed.frame_bytes, b"cached");
}
