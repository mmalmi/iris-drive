//! Environment-derived FIPS carrier settings and endpoint construction.

use fips_core::config::WebSocketConfig;
use hashtree_fips_transport::FipsEndpointOptions;

use crate::config::AppConfig;

use super::{DEFAULT_FIPS_WEBSOCKET_SEED_URLS, default_fips_bootstrap_peer_hints};

pub(super) const FIPS_PACKET_CHANNEL_CAPACITY: usize = 8192;
// Three default STUN servers reserve twelve candidate sockets per link. Eight
// links fit fips-core's 96-socket endpoint budget; nine do not.
const FIPS_WEBRTC_MAX_CONNECTIONS: usize = 8;
const FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING: usize = 0;
pub const IRIS_DRIVE_FIPS_DISCOVERY_SCOPE: &str = "fips-overlay-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FipsTransportSettings {
    pub enable_udp: bool,
    pub enable_webrtc: bool,
    pub enable_lan_discovery: bool,
    pub enable_mesh_pubsub: bool,
    pub enable_local_rendezvous: bool,
    pub websocket_bind_addr: Option<String>,
    pub websocket_seed_urls: Vec<String>,
    pub udp_bind_addr: Option<String>,
    pub udp_public: bool,
    pub udp_external_addr: Option<String>,
    pub share_local_candidates: bool,
    pub static_peer_hints: Vec<(String, Vec<String>)>,
    pub bootstrap_peer_hints: Vec<(String, Vec<String>)>,
    pub webrtc_max_connections: usize,
    pub open_discovery_max_pending: usize,
}

impl Default for FipsTransportSettings {
    fn default() -> Self {
        Self {
            enable_udp: true,
            enable_webrtc: target_allows_default_desktop_fips(std::env::consts::OS),
            enable_lan_discovery: target_allows_default_lan_discovery(std::env::consts::OS),
            enable_mesh_pubsub: true,
            enable_local_rendezvous: true,
            websocket_bind_addr: None,
            websocket_seed_urls: DEFAULT_FIPS_WEBSOCKET_SEED_URLS
                .iter()
                .map(|url| (*url).to_string())
                .collect(),
            udp_bind_addr: None,
            udp_public: false,
            udp_external_addr: None,
            share_local_candidates: target_allows_default_desktop_fips(std::env::consts::OS),
            static_peer_hints: Vec::new(),
            bootstrap_peer_hints: Vec::new(),
            webrtc_max_connections: FIPS_WEBRTC_MAX_CONNECTIONS,
            open_discovery_max_pending: FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING,
        }
    }
}

impl FipsTransportSettings {
    #[must_use]
    pub fn from_env() -> Self {
        let defaults = Self::default();
        let udp_bind_addr = non_empty_env("IRIS_DRIVE_FIPS_UDP_BIND_ADDR");
        let udp_external_addr = non_empty_env("IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR");
        let udp_public =
            bool_env("IRIS_DRIVE_FIPS_UDP_PUBLIC").unwrap_or_else(|| udp_external_addr.is_some());
        let bootstrap_enabled = bool_env("IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP").unwrap_or(false);
        let bootstrap_peer_hints = if !bootstrap_enabled {
            Vec::new()
        } else if let Ok(value) = std::env::var("IRIS_DRIVE_FIPS_BOOTSTRAP_PEERS") {
            parse_static_peer_hints(&value)
        } else {
            default_fips_bootstrap_peer_hints()
        };
        Self {
            enable_udp: bool_env("IRIS_DRIVE_FIPS_ENABLE_UDP").unwrap_or(defaults.enable_udp),
            enable_webrtc: bool_env("IRIS_DRIVE_FIPS_ENABLE_WEBRTC")
                .unwrap_or(defaults.enable_webrtc),
            enable_lan_discovery: bool_env("IRIS_DRIVE_FIPS_ENABLE_LAN_DISCOVERY")
                .unwrap_or(defaults.enable_lan_discovery),
            enable_mesh_pubsub: bool_env("IRIS_DRIVE_FIPS_ENABLE_MESH_PUBSUB")
                .unwrap_or(defaults.enable_mesh_pubsub),
            enable_local_rendezvous: bool_env("IRIS_DRIVE_FIPS_ENABLE_LOCAL_RENDEZVOUS")
                .unwrap_or(defaults.enable_local_rendezvous),
            websocket_bind_addr: non_empty_env("IRIS_FIPS_WEBSOCKET_BIND_ADDR"),
            websocket_seed_urls: std::env::var("IRIS_FIPS_WEBSOCKET_SEED_URLS").map_or_else(
                |_| defaults.websocket_seed_urls.clone(),
                |value| parse_list_env_value(&value),
            ),
            udp_bind_addr,
            udp_public,
            udp_external_addr,
            share_local_candidates: bool_env("IRIS_DRIVE_FIPS_SHARE_LOCAL_CANDIDATES")
                .unwrap_or(defaults.share_local_candidates),
            static_peer_hints: parse_static_peer_hints(
                &std::env::var("IRIS_DRIVE_FIPS_STATIC_PEERS").unwrap_or_default(),
            ),
            bootstrap_peer_hints,
            webrtc_max_connections: bounded_webrtc_max_connections(usize_env(
                "IRIS_DRIVE_FIPS_WEBRTC_MAX_CONNECTIONS",
            )),
            open_discovery_max_pending: usize_env("IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING")
                .unwrap_or(FIPS_NOSTR_OPEN_DISCOVERY_MAX_PENDING),
        }
    }
}

pub(super) fn target_allows_default_desktop_fips(target_os: &str) -> bool {
    !matches!(target_os, "android" | "ios")
}

pub(super) fn target_allows_default_lan_discovery(target_os: &str) -> bool {
    target_os != "android"
}

pub(super) fn fips_endpoint_options(
    identity_nsec: String,
    discovery_scope: String,
    relays: Vec<String>,
    _config: &AppConfig,
    settings: &FipsTransportSettings,
) -> FipsEndpointOptions {
    FipsEndpointOptions {
        identity_nsec,
        discovery_scope,
        relays,
        enable_udp: settings.enable_udp,
        enable_webrtc: settings.enable_webrtc,
        websocket: (settings.websocket_bind_addr.is_some()
            || !settings.websocket_seed_urls.is_empty())
        .then(|| WebSocketConfig {
            bind_addr: settings.websocket_bind_addr.clone(),
            seed_urls: settings.websocket_seed_urls.clone(),
            ..WebSocketConfig::default()
        }),
        enable_local_rendezvous: settings.enable_local_rendezvous,
        ethernet_interfaces: Vec::new(),
        enable_lan_discovery: settings.enable_lan_discovery,
        udp_bind_addr: settings.udp_bind_addr.clone(),
        udp_public: settings.udp_public,
        udp_external_addr: settings.udp_external_addr.clone(),
        share_local_candidates: settings.share_local_candidates,
        webrtc_auto_connect: true,
        webrtc_max_connections: bounded_webrtc_max_connections(Some(
            settings.webrtc_max_connections,
        )),
        open_discovery_max_pending: settings.open_discovery_max_pending,
        packet_channel_capacity: FIPS_PACKET_CHANNEL_CAPACITY,
    }
}

pub(super) fn bounded_webrtc_max_connections(configured: Option<usize>) -> usize {
    configured
        .unwrap_or(FIPS_WEBRTC_MAX_CONNECTIONS)
        .clamp(1, FIPS_WEBRTC_MAX_CONNECTIONS)
}

pub(super) fn parse_bool_env_value(value: &str) -> Option<bool> {
    match value.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

pub(super) fn parse_list_env_value(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

pub(super) fn parse_static_peer_hints(value: &str) -> Vec<(String, Vec<String>)> {
    value
        .split([',', ';'])
        .filter_map(|entry| {
            let (key, addresses) = entry.split_once('=')?;
            let key = key.trim();
            if key.is_empty() {
                return None;
            }
            let addresses = addresses
                .split('|')
                .map(str::trim)
                .filter(|address| !address.is_empty())
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            (!addresses.is_empty()).then(|| (key.to_string(), addresses))
        })
        .collect()
}

fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn bool_env(name: &str) -> Option<bool> {
    parse_bool_env_value(std::env::var(name).ok()?.trim())
}

fn usize_env(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.trim().parse().ok()
}
