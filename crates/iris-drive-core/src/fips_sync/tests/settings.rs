use super::*;

#[test]
fn discovery_scope_is_profile_scoped() {
    let profile_id = crate::NostrIdentityId::new_v4();
    let config = AppConfig {
        profile: Some(crate::ProfileState {
            profile_id,
            app_key_pubkey: "bb".repeat(32),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".into(),
            authorization_state: crate::AppKeyAuthorizationState::AwaitingApproval,
            app_key_label: None,
            app_keys: None,
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
            pending_device_approval_receipts: Vec::new(),
        }),
        ..Default::default()
    };

    assert_eq!(discovery_scope(&config), format!("iris-drive:{profile_id}"));
}

#[test]
fn discovery_scope_uses_iris_drive_overlay_without_profile() {
    assert_eq!(
        discovery_scope(&AppConfig::default()),
        IRIS_DRIVE_FIPS_DISCOVERY_SCOPE
    );
}

#[test]
fn endpoint_options_can_advertise_native_udp_without_disabling_webrtc() {
    let settings = FipsTransportSettings {
        enable_udp: true,
        enable_webrtc: true,
        enable_lan_discovery: true,
        enable_mesh_pubsub: true,
        udp_bind_addr: Some("0.0.0.0:2121".to_string()),
        udp_public: true,
        udp_external_addr: Some("10.44.94.98:2121".to_string()),
        share_local_candidates: true,
        static_peer_hints: Vec::new(),
        bootstrap_peer_hints: Vec::new(),
        webrtc_max_connections: 12,
        open_discovery_max_pending: 8,
    };

    let options = fips_endpoint_options(
        "nsec1example".to_string(),
        IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string(),
        vec!["wss://relay.example".to_string()],
        &AppConfig::default(),
        &settings,
    );

    assert!(options.enable_udp);
    assert!(options.enable_webrtc);
    assert_eq!(options.udp_bind_addr.as_deref(), Some("0.0.0.0:2121"));
    assert!(options.udp_public);
    assert_eq!(
        options.udp_external_addr.as_deref(),
        Some("10.44.94.98:2121")
    );
    assert!(options.webrtc_auto_connect);
    assert_eq!(options.webrtc_max_connections, 12);
    assert_eq!(options.open_discovery_max_pending, 8);
}

#[test]
fn default_transport_settings_do_not_seed_fips_bootstrap_transit() {
    let settings = FipsTransportSettings::default();

    assert_eq!(settings.webrtc_max_connections, 16);
    assert_eq!(settings.open_discovery_max_pending, 0);
    assert!(settings.bootstrap_peer_hints.is_empty());
    assert!(settings.enable_lan_discovery);
}

#[test]
fn single_device_profile_does_not_route_to_bootstrap_fips_peers() {
    let current_pubkey = "dd".repeat(32);
    let profile_id = crate::NostrIdentityId::new_v4();
    let config = AppConfig {
        profile: Some(crate::ProfileState {
            profile_id,
            app_key_pubkey: current_pubkey.clone(),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".into(),
            authorization_state: crate::AppKeyAuthorizationState::Authorized,
            app_key_label: None,
            app_keys: Some(crate::app_keys::AppKeysProjection {
                profile_id: profile_id.to_string(),
                signed_by_pubkey: Some(current_pubkey.clone()),
                created_at: 1,
                app_actors: vec![crate::app_keys::AppActorEntry::admin(
                    current_pubkey,
                    1,
                    Some("iPhone".into()),
                )],
                dck_generation: 0,
                wrapped_dck: std::collections::BTreeMap::default(),
            }),
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),

            pending_device_approval_receipts: Vec::new(),
        }),
        ..Default::default()
    };

    assert!(authorized_device_fips_peers(&config, &FipsTransportSettings::default()).is_empty());
    assert!(routing_fips_peers(&config, &FipsTransportSettings::default()).is_empty());
}

#[test]
fn admin_inbound_app_key_link_request_configures_pending_fips_peer() {
    let current_pubkey = "dd".repeat(32);
    let pending_pubkey = "ee".repeat(32);
    let invite_pubkey = "ff".repeat(32);
    let profile_id = crate::NostrIdentityId::new_v4();
    let config = AppConfig {
        profile: Some(crate::ProfileState {
            profile_id,
            app_key_pubkey: current_pubkey.clone(),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".into(),
            authorization_state: crate::AppKeyAuthorizationState::Authorized,
            app_key_label: None,
            app_keys: Some(crate::app_keys::AppKeysProjection {
                profile_id: profile_id.to_string(),
                signed_by_pubkey: Some(current_pubkey.clone()),
                created_at: 1,
                app_actors: vec![crate::app_keys::AppActorEntry::admin(
                    current_pubkey,
                    1,
                    Some("Mac".into()),
                )],
                dck_generation: 0,
                wrapped_dck: std::collections::BTreeMap::default(),
            }),
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: vec![crate::profile::InboundAppKeyLinkRequest {
                app_key_pubkey: pending_pubkey.clone(),
                label: Some("iPhone".into()),
                invite_pubkey,
                request_url: String::new(),
                requested_at: 1,
            }],
            handled_app_key_link_requests: Vec::new(),

            pending_device_approval_receipts: Vec::new(),
        }),
        ..Default::default()
    };
    let pending_npub = PublicKey::from_hex(&pending_pubkey)
        .unwrap()
        .to_bech32()
        .unwrap();

    let peers = authorized_device_fips_peers(&config, &FipsTransportSettings::default());

    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].npub, pending_npub);
}

#[test]
fn admin_accepting_link_requests_promotes_connected_joiners_to_application_peers() {
    let mut application_peers = vec![FipsPeerConfig::new("already-app".to_string())];
    let routing_peers = vec![FipsPeerConfig::new("already-routing".to_string())];

    add_connected_app_key_link_application_peers(
        &mut application_peers,
        &routing_peers,
        "local",
        [
            "local".to_string(),
            "already-app".to_string(),
            "already-routing".to_string(),
            "pending-joiner".to_string(),
        ],
    );

    assert_eq!(
        application_peers
            .iter()
            .map(|peer| peer.npub.as_str())
            .collect::<Vec<_>>(),
        vec!["already-app", "pending-joiner"]
    );
}

#[test]
fn remote_authorized_device_does_not_seed_bootstrap_fips_routing_peers() {
    let current_pubkey = "dd".repeat(32);
    let remote_pubkey = "ee".repeat(32);
    let profile_id = crate::NostrIdentityId::new_v4();
    let config = AppConfig {
        profile: Some(crate::ProfileState {
            profile_id,
            app_key_pubkey: current_pubkey.clone(),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".into(),
            authorization_state: crate::AppKeyAuthorizationState::Authorized,
            app_key_label: None,
            app_keys: Some(crate::app_keys::AppKeysProjection {
                profile_id: profile_id.to_string(),
                signed_by_pubkey: Some(current_pubkey.clone()),
                created_at: 1,
                app_actors: vec![
                    crate::app_keys::AppActorEntry::admin(current_pubkey, 1, Some("Mac".into())),
                    crate::app_keys::AppActorEntry::member(remote_pubkey, 1, Some("iPhone".into())),
                ],
                dck_generation: 0,
                wrapped_dck: std::collections::BTreeMap::default(),
            }),
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),

            pending_device_approval_receipts: Vec::new(),
        }),
        ..Default::default()
    };

    assert!(routing_fips_peers(&config, &FipsTransportSettings::default()).is_empty());
}

#[test]
fn explicit_bootstrap_hints_add_fips_routing_peers() {
    let current_pubkey = "dd".repeat(32);
    let remote_pubkey = "ee".repeat(32);
    let profile_id = crate::NostrIdentityId::new_v4();
    let config = AppConfig {
        profile: Some(crate::ProfileState {
            profile_id,
            app_key_pubkey: current_pubkey.clone(),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".into(),
            authorization_state: crate::AppKeyAuthorizationState::Authorized,
            app_key_label: None,
            app_keys: Some(crate::app_keys::AppKeysProjection {
                profile_id: profile_id.to_string(),
                signed_by_pubkey: Some(current_pubkey.clone()),
                created_at: 1,
                app_actors: vec![
                    crate::app_keys::AppActorEntry::admin(current_pubkey, 1, Some("Mac".into())),
                    crate::app_keys::AppActorEntry::member(remote_pubkey, 1, Some("iPhone".into())),
                ],
                dck_generation: 0,
                wrapped_dck: std::collections::BTreeMap::default(),
            }),
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),

            pending_device_approval_receipts: Vec::new(),
        }),
        ..Default::default()
    };
    let settings = FipsTransportSettings {
        bootstrap_peer_hints: default_fips_bootstrap_peer_hints(),
        ..Default::default()
    };

    assert_eq!(
        routing_fips_peers(&config, &settings).len(),
        DEFAULT_FIPS_BOOTSTRAP_PEERS.len()
    );
}

#[test]
fn fips_peer_config_snapshot_matches_endpoint_peer_sanitizing() {
    let snapshot = fips_peer_config_snapshot(
        Some("local"),
        &[
            FipsPeerConfig {
                npub: " remote ".to_string(),
                udp_addresses: vec![
                    " 10.44.1.2:22121 ".to_string(),
                    " ".to_string(),
                    "udp:10.44.1.3:22121".to_string(),
                ],
            },
            FipsPeerConfig {
                npub: "local".to_string(),
                udp_addresses: vec!["10.44.1.1:22121".to_string()],
            },
            FipsPeerConfig {
                npub: "remote".to_string(),
                udp_addresses: vec!["10.44.1.4:22121".to_string()],
            },
            FipsPeerConfig {
                npub: " ".to_string(),
                udp_addresses: vec!["10.44.1.5:22121".to_string()],
            },
        ],
        &[
            FipsPeerConfig {
                npub: "remote".to_string(),
                udp_addresses: vec!["10.44.1.6:22121".to_string()],
            },
            FipsPeerConfig {
                npub: " bootstrap ".to_string(),
                udp_addresses: vec![" udp:203.0.113.7:2121 ".to_string()],
            },
        ],
    );

    assert_eq!(
        snapshot.application_peers,
        vec![FipsPeerConfig {
            npub: "remote".to_string(),
            udp_addresses: vec![
                "10.44.1.2:22121".to_string(),
                "udp:10.44.1.3:22121".to_string()
            ],
        }]
    );
    assert_eq!(
        snapshot.routing_peers,
        vec![FipsPeerConfig {
            npub: "bootstrap".to_string(),
            udp_addresses: vec!["udp:203.0.113.7:2121".to_string()],
        }]
    );
}

#[test]
fn legacy_drive_roots_do_not_seed_bootstrap_fips_routing_peers() {
    let current_keys = nostr_sdk::Keys::generate();
    let remote_keys = nostr_sdk::Keys::generate();
    let current_pubkey = current_keys.public_key().to_hex();
    let remote_pubkey = remote_keys.public_key().to_hex();
    let profile_id = crate::NostrIdentityId::new_v4();
    let mut drive = crate::Drive::primary(profile_id.to_string());
    drive.app_key_roots.insert(
        current_pubkey.clone(),
        crate::AppKeyRootRef::legacy("current-root", 10, 1),
    );
    drive.app_key_roots.insert(
        remote_pubkey.clone(),
        crate::AppKeyRootRef::legacy("remote-root", 11, 1),
    );
    let config = AppConfig {
        profile: Some(crate::ProfileState {
            profile_id,
            app_key_pubkey: current_pubkey,
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".into(),
            authorization_state: crate::AppKeyAuthorizationState::Authorized,
            app_key_label: None,
            app_keys: None,
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),

            pending_device_approval_receipts: Vec::new(),
        }),
        drives: vec![drive],
        ..Default::default()
    };

    let remote_npub = remote_keys.public_key().to_bech32().unwrap();
    let authorized = authorized_device_fips_peers(&config, &FipsTransportSettings::default());
    assert_eq!(authorized.len(), 1);
    assert_eq!(authorized[0].npub, remote_npub);
    assert!(routing_fips_peers(&config, &FipsTransportSettings::default()).is_empty());
}

#[test]
fn static_peer_hints_match_authorized_devices_by_label_or_npub() {
    let first_keys = nostr_sdk::Keys::generate();
    let second_keys = nostr_sdk::Keys::generate();
    let first_pubkey = first_keys.public_key().to_hex();
    let second_pubkey = second_keys.public_key().to_hex();
    let first_npub = first_keys.public_key().to_bech32().unwrap();
    let profile_id = crate::NostrIdentityId::new_v4();
    let settings = FipsTransportSettings {
        static_peer_hints: parse_static_peer_hints(&format!(
            "linux-peer=10.44.214.2:22121,{first_npub}=10.44.34.102:22121"
        )),
        ..Default::default()
    };
    let config = AppConfig {
        profile: Some(crate::ProfileState {
            profile_id,
            app_key_pubkey: "dd".repeat(32),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".into(),
            authorization_state: crate::AppKeyAuthorizationState::Authorized,
            app_key_label: None,
            app_keys: Some(crate::app_keys::AppKeysProjection {
                profile_id: profile_id.to_string(),
                signed_by_pubkey: Some("dd".repeat(32)),
                created_at: 1,
                app_actors: vec![
                    crate::app_keys::AppActorEntry::member(
                        first_pubkey,
                        1,
                        Some("macos-peer".into()),
                    ),
                    crate::app_keys::AppActorEntry::member(
                        second_pubkey,
                        1,
                        Some("linux-peer".into()),
                    ),
                ],
                dck_generation: 0,
                wrapped_dck: std::collections::BTreeMap::default(),
            }),
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),

            pending_device_approval_receipts: Vec::new(),
        }),
        ..Default::default()
    };

    let peers = authorized_device_fips_peers(&config, &settings);

    assert_eq!(peers.len(), 2);
    assert!(peers.iter().any(|peer| peer.npub == first_npub
        && peer.udp_addresses == vec!["10.44.34.102:22121".to_string()]));
    assert!(
        peers
            .iter()
            .any(|peer| peer.udp_addresses == vec!["10.44.214.2:22121".to_string()])
    );
}

#[test]
fn pending_app_key_link_admin_is_allowed_for_roster_app_messages() {
    let admin_keys = nostr_sdk::Keys::generate();
    let admin_pubkey = admin_keys.public_key().to_hex();
    let admin_npub = admin_keys.public_key().to_bech32().unwrap();
    let settings = FipsTransportSettings {
        bootstrap_peer_hints: Vec::new(),
        static_peer_hints: parse_static_peer_hints(&format!("{admin_npub}=10.44.1.9:22121")),
        ..Default::default()
    };
    let config = AppConfig {
        profile: Some(crate::ProfileState {
            profile_id: crate::NostrIdentityId::new_v4(),
            app_key_pubkey: "dd".repeat(32),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "link-secret".into(),
            authorization_state: crate::AppKeyAuthorizationState::AwaitingApproval,
            app_key_label: None,
            app_keys: None,
            profile_roster_projection: None,
            outbound_app_key_link_request: Some(crate::profile::PendingAppKeyLinkRequest {
                admin_app_key_pubkey: admin_pubkey,
                invite_pubkey: "ee".repeat(32),
                request_url: String::new(),
                request_key_secret: "ff".repeat(32),
                approval_receipt_event: None,
                requested_at: 42,
            }),
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),

            pending_device_approval_receipts: Vec::new(),
        }),
        ..Default::default()
    };

    let authorized = authorized_device_fips_peers(&config, &settings);
    assert_eq!(authorized.len(), 1);
    assert_eq!(authorized[0].npub, admin_npub);
    assert_eq!(authorized[0].udp_addresses, vec!["10.44.1.9:22121"]);
    let routing = routing_fips_peers(&config, &settings);
    assert_eq!(routing.len(), 1);
    assert_eq!(routing[0].npub, admin_npub);
    assert_eq!(routing[0].udp_addresses, vec!["10.44.1.9:22121"]);
}

#[test]
fn endpoint_options_keep_native_udp_private_by_default() {
    let settings = FipsTransportSettings::default();

    let options = fips_endpoint_options(
        "nsec1example".to_string(),
        IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string(),
        Vec::new(),
        &AppConfig::default(),
        &settings,
    );

    assert!(options.enable_udp);
    assert!(options.enable_webrtc);
    assert!(!options.udp_public);
    assert!(options.udp_bind_addr.is_none());
    assert!(options.udp_external_addr.is_none());
}

#[test]
fn mobile_fips_defaults_keep_lan_without_local_candidate_sharing() {
    assert!(!target_allows_default_desktop_fips("android"));
    assert!(!target_allows_default_desktop_fips("ios"));
    assert!(target_allows_default_desktop_fips("macos"));
    assert!(target_allows_default_desktop_fips("linux"));
}

#[test]
fn endpoint_options_carry_ambient_discovery_settings() {
    let settings = FipsTransportSettings {
        enable_lan_discovery: false,
        enable_mesh_pubsub: false,
        share_local_candidates: false,
        ..Default::default()
    };

    let options = fips_endpoint_options(
        "nsec1example".to_string(),
        IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string(),
        Vec::new(),
        &AppConfig::default(),
        &settings,
    );

    assert!(!options.enable_lan_discovery);
    assert!(!options.share_local_candidates);
    assert!(!settings.enable_mesh_pubsub);
}

#[test]
fn endpoint_options_keep_packet_queue_large_enough_for_root_bursts() {
    let settings = FipsTransportSettings::default();

    let options = fips_endpoint_options(
        "nsec1example".to_string(),
        IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string(),
        Vec::new(),
        &AppConfig::default(),
        &settings,
    );

    assert!(
        options.packet_channel_capacity >= 8192,
        "direct root hints share this queue with block traffic during file bursts"
    );
}

#[test]
fn admin_endpoint_options_keep_open_discovery_closed_by_default() {
    let settings = FipsTransportSettings::default();
    let dir = tempfile::tempdir().unwrap();
    let profile = crate::Profile::create(dir.path(), Some("admin".into())).unwrap();
    let config = AppConfig {
        profile: Some(profile.state),
        ..Default::default()
    };

    let options = fips_endpoint_options(
        "nsec1example".to_string(),
        IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string(),
        Vec::new(),
        &config,
        &settings,
    );

    assert_eq!(options.open_discovery_max_pending, 0);
}

#[test]
fn signed_control_topics_are_allowed_before_peer_is_configured() {
    assert_eq!(
        super::unconfigured_app_message_topics(),
        [
            crate::app_key_link_transport::APP_KEY_LINK_REQUEST_APP_TOPIC,
            crate::app_key_link_transport::APP_KEY_APPROVAL_RECEIPT_APP_TOPIC,
            crate::app_key_link_transport::APP_KEY_LINK_ROSTER_APP_TOPIC,
            crate::direct_root_transport::DIRECT_ROOT_APP_TOPIC,
        ]
    );
}

#[tokio::test]
async fn unconfigured_signed_control_app_messages_are_delivered() {
    let network = Arc::new(TokioMutex::new(std::collections::HashMap::new()));
    let admin_endpoint = FakeEndpoint::new("admin", network.clone()).await;
    let phone_endpoint = FakeEndpoint::new("phone", network).await;
    let admin_transport = Arc::new(HashtreeFipsTransport::new(
        admin_endpoint,
        Arc::new(MemoryStore::new()),
    ));
    let phone_transport = Arc::new(
        HashtreeFipsTransport::new(phone_endpoint, Arc::new(MemoryStore::new()))
            .with_unconfigured_app_message_topics(unconfigured_app_message_topics()),
    );
    phone_transport
        .set_peers(vec!["configured-but-not-admin".to_string()])
        .await;
    let mut app_messages = phone_transport.subscribe_app_messages();
    let admin_task = admin_transport.start();
    let phone_task = phone_transport.start();

    for (topic, data) in [
        (
            crate::app_key_link_transport::APP_KEY_LINK_ROSTER_APP_TOPIC,
            b"signed roster".as_slice(),
        ),
        (
            crate::app_key_link_transport::APP_KEY_APPROVAL_RECEIPT_APP_TOPIC,
            b"signed receipt".as_slice(),
        ),
        (
            crate::direct_root_transport::DIRECT_ROOT_APP_TOPIC,
            b"signed direct root".as_slice(),
        ),
    ] {
        admin_transport
            .send_app_message("phone", topic, data.to_vec())
            .await
            .unwrap();
        let message = tokio::time::timeout(Duration::from_millis(250), app_messages.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(message.peer_id, "admin");
        assert_eq!(message.topic, topic);
        assert_eq!(message.data, data);
    }

    admin_task.abort();
    phone_task.abort();
}

#[test]
fn bool_env_parser_accepts_common_spellings() {
    for value in ["1", "true", "TRUE", "yes", "on"] {
        assert_eq!(parse_bool_env_value(value), Some(true));
    }
    for value in ["0", "false", "FALSE", "no", "off"] {
        assert_eq!(parse_bool_env_value(value), Some(false));
    }
    assert_eq!(parse_bool_env_value("maybe"), None);
}
