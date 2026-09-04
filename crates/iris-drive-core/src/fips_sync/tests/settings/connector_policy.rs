use super::*;

use fips_core::discovery::nostr::{OverlayAdvert, OverlayEndpointAdvert, OverlayTransportKind};
use nostr_sdk::{Event, EventBuilder, Keys, Kind};

#[test]
fn outbound_only_authorized_peer_keeps_dialing_after_pending_transition() {
    let higher_local = "npub1zzz";
    let lower_peer = "npub1aaa";
    let peer = FipsPeerConfig {
        npub: lower_peer.to_string(),
        udp_addresses: Vec::new(),
    };
    let application_peer_ids = BTreeSet::from([lower_peer.to_string()]);

    let pending = drive_core_peer_configs(
        higher_local,
        LocalInboundCapability::OutboundOnly,
        &application_peer_ids,
        &BTreeSet::from([lower_peer.to_string()]),
        vec![peer.clone()],
    )
    .remove(0);
    let authorized = drive_core_peer_configs(
        higher_local,
        LocalInboundCapability::OutboundOnly,
        &application_peer_ids,
        &BTreeSet::new(),
        vec![peer],
    )
    .remove(0);
    let lower_outbound_only = drive_core_peer_configs(
        lower_peer,
        LocalInboundCapability::OutboundOnly,
        &BTreeSet::from([higher_local.to_string()]),
        &BTreeSet::new(),
        vec![FipsPeerConfig {
            npub: higher_local.to_string(),
            udp_addresses: Vec::new(),
        }],
    )
    .remove(0);

    assert_eq!(pending.connect_policy, ConnectPolicy::AutoConnect);
    assert_eq!(authorized.connect_policy, ConnectPolicy::AutoConnect);
    assert_eq!(
        lower_outbound_only.connect_policy,
        ConnectPolicy::AutoConnect,
        "both npub orderings must initiate when the local endpoint has no direct inbound advert"
    );
    assert!(
        !authorized.auto_reconnect,
        "the control runtime bounds retries"
    );
}

#[test]
fn revoked_peer_is_not_kept_as_an_outbound_only_application_dial_target() {
    let peer = FipsPeerConfig {
        npub: "npub1revoked".to_string(),
        udp_addresses: Vec::new(),
    };
    let config = drive_core_peer_configs(
        "npub1zzz",
        LocalInboundCapability::OutboundOnly,
        &BTreeSet::new(),
        &BTreeSet::new(),
        vec![peer],
    )
    .remove(0);

    assert_eq!(config.connect_policy, ConnectPolicy::AutoConnect);
    assert!(
        config.auto_reconnect,
        "a non-application routing peer must not inherit Drive roster ACL policy"
    );
}

#[test]
fn revoked_app_key_is_absent_from_application_and_routing_acl() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = crate::Profile::create(dir.path(), Some("owner".into())).unwrap();
    let revoked = Keys::generate().public_key().to_hex();
    owner
        .approve_app_key(&revoked, Some("old phone".into()))
        .unwrap();
    owner.revoke_app_key(&revoked).unwrap();
    let config = AppConfig {
        profile: Some(owner.state),
        ..AppConfig::default()
    };

    assert!(authorized_device_fips_peers(&config, &FipsTransportSettings::default()).is_empty());
    assert!(routing_fips_peers(&config, &FipsTransportSettings::default()).is_empty());
}

#[test]
fn connector_election_uses_authenticated_advertised_endpoint_capabilities() {
    let local = Keys::generate();
    let local_npub = local.public_key().to_bech32().unwrap();
    let nat_only = signed_capability_advert(
        &local,
        vec![OverlayEndpointAdvert {
            transport: OverlayTransportKind::Udp,
            addr: "nat".to_string(),
        }],
    );
    let inbound = signed_capability_advert(
        &local,
        vec![OverlayEndpointAdvert {
            transport: OverlayTransportKind::WebRtc,
            addr: format!("02{}", "11".repeat(32)),
        }],
    );
    let foreign = signed_capability_advert(
        &Keys::generate(),
        vec![OverlayEndpointAdvert {
            transport: OverlayTransportKind::WebSocket,
            addr: "wss://peer.example/fips".to_string(),
        }],
    );

    assert_eq!(
        local_inbound_capability_from_event(&nat_only, &local_npub),
        LocalInboundCapability::OutboundOnly
    );
    assert_eq!(
        local_inbound_capability_from_event(&inbound, &local_npub),
        LocalInboundCapability::InboundRoutable
    );
    assert_eq!(
        local_inbound_capability_from_event(&foreign, &local_npub),
        LocalInboundCapability::OutboundOnly,
        "a foreign signed advert must not steer local connector election"
    );
}

fn signed_capability_advert(keys: &Keys, endpoints: Vec<OverlayEndpointAdvert>) -> Event {
    let advert = OverlayAdvert {
        identifier: fips_core::discovery::nostr::ADVERT_IDENTIFIER.to_string(),
        version: fips_core::discovery::nostr::ADVERT_VERSION,
        endpoints,
        stun_servers: None,
    };
    EventBuilder::new(
        Kind::Custom(fips_core::discovery::nostr::ADVERT_KIND),
        serde_json::to_string(&advert).unwrap(),
    )
    .sign_with_keys(keys)
    .unwrap()
}
