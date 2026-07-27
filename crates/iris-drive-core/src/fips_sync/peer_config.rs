use std::collections::BTreeSet;

use fips_core::FipsEndpoint;
use fips_core::config::{ConnectPolicy, PeerAddress, PeerConfig as CoreFipsPeerConfig};
use hashtree_fips_transport::FipsPeerConfig;

pub(super) async fn set_drive_fips_peer_configs(
    endpoint: &FipsEndpoint,
    local_npub: &str,
    application_peers: &[FipsPeerConfig],
    peer_configs: Vec<FipsPeerConfig>,
) -> Result<(), fips_core::FipsEndpointError> {
    let application_peer_ids = peer_ids(application_peers);
    let peers = drive_core_peer_configs(local_npub, &application_peer_ids, peer_configs);
    let peer_count = peers.len();
    let outcome = endpoint.update_peers(peers).await?;
    tracing::info!(
        peer_count,
        added = outcome.added,
        removed = outcome.removed,
        updated = outcome.updated,
        unchanged = outcome.unchanged,
        "updated Drive FIPS endpoint peer configs"
    );
    Ok(())
}

pub(super) fn drive_core_peer_configs(
    local_npub: &str,
    application_peer_ids: &BTreeSet<String>,
    peer_configs: Vec<FipsPeerConfig>,
) -> Vec<CoreFipsPeerConfig> {
    peer_configs
        .into_iter()
        .map(|peer| {
            let application_peer = application_peer_ids.contains(&peer.npub);
            let connect_policy = if application_peer && local_npub > peer.npub.as_str() {
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
