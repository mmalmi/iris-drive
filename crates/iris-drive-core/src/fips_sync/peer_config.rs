use std::collections::BTreeSet;

use fips_core::FipsEndpoint;
use fips_core::config::{ConnectPolicy, PeerAddress, PeerConfig as CoreFipsPeerConfig};
use fips_core::discovery::nostr::{
    ADVERT_IDENTIFIER, ADVERT_KIND, ADVERT_VERSION, OverlayAdvert, OverlayTransportKind,
};
use hashtree_fips_transport::FipsPeerConfig;
use nostr_sdk::{Event, PublicKey};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LocalInboundCapability {
    InboundRoutable,
    OutboundOnly,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(super) struct FipsPeerConfigSnapshot {
    pub(super) local_inbound_capability: LocalInboundCapability,
    pub(super) application: Vec<FipsPeerConfig>,
    pub(super) routing: Vec<FipsPeerConfig>,
    pub(super) blob: Vec<FipsPeerConfig>,
}

pub(super) fn fips_peer_config_snapshot(
    local: Option<&str>,
    local_inbound_capability: LocalInboundCapability,
    application_peers: &[FipsPeerConfig],
    routing_peers: &[FipsPeerConfig],
    blob_peers: &[FipsPeerConfig],
) -> FipsPeerConfigSnapshot {
    let mut seen = std::collections::HashSet::new();
    let mut blob_seen = std::collections::HashSet::new();
    FipsPeerConfigSnapshot {
        local_inbound_capability,
        application: normalize_fips_peer_configs(local, application_peers, &mut seen),
        routing: normalize_fips_peer_configs(local, routing_peers, &mut seen),
        blob: normalize_fips_peer_configs(local, blob_peers, &mut blob_seen),
    }
}

fn normalize_fips_peer_configs(
    local: Option<&str>,
    peers: &[FipsPeerConfig],
    seen: &mut std::collections::HashSet<String>,
) -> Vec<FipsPeerConfig> {
    let mut out = Vec::new();
    for peer in peers {
        let npub = peer.npub.trim().to_string();
        if npub.is_empty() || Some(npub.as_str()) == local || !seen.insert(npub.clone()) {
            continue;
        }
        let udp_addresses = peer
            .udp_addresses
            .iter()
            .map(|addr| addr.trim().to_string())
            .filter(|addr| !addr.is_empty())
            .collect();
        out.push(FipsPeerConfig {
            npub,
            udp_addresses,
        });
    }
    out
}

pub(super) async fn set_drive_fips_peer_configs(
    endpoint: &FipsEndpoint,
    local_npub: &str,
    application_peers: &[FipsPeerConfig],
    outbound_connect_peers: &[FipsPeerConfig],
    peer_configs: Vec<FipsPeerConfig>,
) -> Result<(), fips_core::FipsEndpointError> {
    let application_peer_ids = peer_ids(application_peers);
    let outbound_connect_peer_ids = peer_ids(outbound_connect_peers);
    let local_inbound_capability =
        authenticated_local_inbound_capability(endpoint, local_npub).await;
    let peers = drive_core_peer_configs(
        local_npub,
        local_inbound_capability,
        &application_peer_ids,
        &outbound_connect_peer_ids,
        peer_configs,
    );
    let peer_count = peers.len();
    let outcome = endpoint.update_peers(peers).await?;
    tracing::info!(
        peer_count,
        added = outcome.added,
        removed = outcome.removed,
        updated = outcome.updated,
        unchanged = outcome.unchanged,
        ?local_inbound_capability,
        "updated Drive FIPS endpoint peer configs"
    );
    Ok(())
}

pub(super) fn drive_core_peer_configs(
    local_npub: &str,
    local_inbound_capability: LocalInboundCapability,
    application_peer_ids: &BTreeSet<String>,
    outbound_connect_peer_ids: &BTreeSet<String>,
    peer_configs: Vec<FipsPeerConfig>,
) -> Vec<CoreFipsPeerConfig> {
    peer_configs
        .into_iter()
        .map(|peer| {
            let application_peer = application_peer_ids.contains(&peer.npub);
            let explicitly_outbound = outbound_connect_peer_ids.contains(&peer.npub);
            // Before approval, the joiner can be the only endpoint with a
            // route. It must initiate even when the normal two-authorized-peer
            // identity tie-break would elect the unaware owner. This changes
            // only dialing; application and blob ACLs remain independently
            // derived from `application_peers` and the approved roster.
            let connect_policy = if application_peer
                && !explicitly_outbound
                && local_inbound_capability == LocalInboundCapability::InboundRoutable
                && local_npub > peer.npub.as_str()
            {
                ConnectPolicy::Manual
            } else {
                ConnectPolicy::AutoConnect
            };
            CoreFipsPeerConfig {
                npub: peer.npub,
                addresses: peer
                    .udp_addresses
                    .iter()
                    .filter_map(|address| drive_peer_address(address))
                    .collect(),
                connect_policy,
                // Iris Drive's authenticated TCP control runtime owns
                // application-peer liveness and reconnects. Leaving the
                // endpoint's independent reconnect loop enabled makes a
                // healthy routed control session repeatedly hunt for a
                // redundant direct path.
                auto_reconnect: !application_peer,
                ..CoreFipsPeerConfig::default()
            }
        })
        .collect()
}

pub(super) async fn authenticated_local_inbound_capability(
    endpoint: &FipsEndpoint,
    local_npub: &str,
) -> LocalInboundCapability {
    let Ok(Some(event)) = endpoint.local_nostr_discovery_advert_event().await else {
        return LocalInboundCapability::OutboundOnly;
    };
    local_inbound_capability_from_event(&event, local_npub)
}

pub(super) fn local_inbound_capability_from_event(
    event: &Event,
    local_npub: &str,
) -> LocalInboundCapability {
    if event.kind.as_u16() != ADVERT_KIND
        || event.verify().is_err()
        || PublicKey::parse(local_npub).ok().as_ref() != Some(&event.pubkey)
    {
        return LocalInboundCapability::OutboundOnly;
    }
    let Ok(advert) = serde_json::from_str::<OverlayAdvert>(&event.content) else {
        return LocalInboundCapability::OutboundOnly;
    };
    if advert.identifier != ADVERT_IDENTIFIER || advert.version != ADVERT_VERSION {
        return LocalInboundCapability::OutboundOnly;
    }
    if advert.endpoints.iter().any(|endpoint| {
        endpoint.transport != OverlayTransportKind::Udp
            || !endpoint.addr.trim().eq_ignore_ascii_case("nat")
    }) {
        LocalInboundCapability::InboundRoutable
    } else {
        // A UDP NAT traversal advert is authenticated and dial-capable, but
        // it is not a stable inbound address. Keep this endpoint elected as
        // an initiator after a pending link becomes authorized so two
        // outbound-only devices cannot strand each other at the identity
        // tie-break boundary.
        LocalInboundCapability::OutboundOnly
    }
}

fn drive_peer_address(raw: &str) -> Option<PeerAddress> {
    let value = raw.trim();
    if value.is_empty() {
        return None;
    }
    if value
        .split_once(':')
        .is_some_and(|(transport, _)| transport.eq_ignore_ascii_case("nostr_relay"))
    {
        return None;
    }
    let (transport, address) = value.split_once(':').map_or(("udp", value), |parts| {
        match parts.0.to_ascii_lowercase().as_str() {
            "udp" | "tcp" | "webrtc" | "tor" | "ethernet" | "ble" => parts,
            _ => ("udp", value),
        }
    });
    if address.is_empty() {
        return None;
    }
    Some(PeerAddress::new(transport, address))
}

pub(super) fn peer_ids(peers: &[FipsPeerConfig]) -> BTreeSet<String> {
    peers
        .iter()
        .map(|peer| peer.npub.trim())
        .filter(|npub| !npub.is_empty())
        .map(str::to_string)
        .collect()
}
