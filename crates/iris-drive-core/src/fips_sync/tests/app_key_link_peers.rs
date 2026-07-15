use super::*;

#[test]
fn receipt_authorized_linked_device_keeps_admin_fips_peer_until_roster_arrives() {
    let admin_dir = tempfile::tempdir().unwrap();
    let linked_dir = tempfile::tempdir().unwrap();
    let mut admin = crate::Profile::create(admin_dir.path(), Some("admin".into())).unwrap();
    let mut linked = crate::Profile::link_to_profile(
        linked_dir.path(),
        admin.state.profile_id,
        admin.state.app_key_pubkey.clone(),
        Some("phone".into()),
    )
    .unwrap();
    let approval_request = crate::app_key_link_transport::create_app_key_approval_bootstrap(
        linked.app_key.keys(),
        linked.state.app_key_label.as_deref(),
    )
    .unwrap();
    linked
        .state
        .queue_outbound_app_key_link_request(
            admin.state.app_key_pubkey.clone(),
            &crate::profile::app_key_link_invite_pubkey(&admin.state.app_key_link_secret).unwrap(),
            123,
            approval_request.url,
            approval_request.request_keys.secret_key().to_secret_hex(),
        )
        .unwrap();
    admin
        .approve_device_bootstrap(
            &approval_request.bootstrap,
            linked.state.app_key_label.clone(),
        )
        .unwrap();
    linked.state.authorization_state = crate::AppKeyAuthorizationState::Authorized;
    linked
        .state
        .outbound_app_key_link_request
        .as_mut()
        .unwrap()
        .approval_receipt_event = Some(
        admin
            .state
            .pending_device_approval_receipts
            .last()
            .unwrap()
            .event_json
            .clone(),
    );
    let config = AppConfig {
        profile: Some(linked.state),
        ..Default::default()
    };
    let admin_npub = admin.app_key.keys().public_key().to_bech32().unwrap();

    let peers = authorized_device_fips_peers(&config, &FipsTransportSettings::default());

    assert_eq!(peers.len(), 1);
    assert_eq!(peers[0].npub, admin_npub);
}
