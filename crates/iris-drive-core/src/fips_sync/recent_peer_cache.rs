use std::path::{Path, PathBuf};

use fips_core::config::{PeerAddress, PeerConfig};
use fips_core::endpoint::FipsEndpointPeer;
use fips_core::{RecentPeers, RecentPeersError};
use fips_endpoint::RecentPeersFileStore;
use hashtree_fips_transport::FipsPeerConfig;

pub(super) const RECENT_PEERS_TTL_MS: u64 = 7 * 24 * 60 * 60 * 1_000;
pub(super) const RECENT_PEERS_REFRESH_MS: u64 = 60 * 60 * 1_000;

pub(super) struct DriveRecentPeers {
    store: RecentPeersFileStore,
    recent: RecentPeers,
}

impl DriveRecentPeers {
    pub(super) fn load(
        path: impl Into<PathBuf>,
        local_npub: &str,
        scope: &str,
        now_ms: u64,
    ) -> Option<Self> {
        let path = path.into();
        let store = match RecentPeersFileStore::new(&path, local_npub, scope) {
            Ok(store) => store,
            Err(error) => {
                tracing::warn!(%error, path = %path.display(), "disabling Drive recent-peer cache");
                return None;
            }
        };
        let mut recent = match store.load() {
            Ok(recent) => recent,
            Err(error) => {
                tracing::warn!(
                    %error,
                    path = %path.display(),
                    "discarding unreadable or invalid Drive recent-peer cache"
                );
                empty_recent_peers(local_npub, scope)?
            }
        };
        recent.prune(now_ms, RECENT_PEERS_TTL_MS);
        Some(Self { store, recent })
    }

    pub(super) fn merge_into_peer_configs(&self, peer_configs: &mut [FipsPeerConfig]) -> usize {
        merge_recent_udp_addresses(&self.recent, peer_configs)
    }

    pub(super) fn observe(&mut self, peers: &[FipsEndpointPeer], now_ms: u64) -> bool {
        let mut changed = false;
        for peer in peers {
            if !self.observation_due(peer, now_ms) {
                continue;
            }
            match self.recent.observe_authenticated_peer(peer, now_ms) {
                Ok(observed_changed) => changed |= observed_changed,
                Err(error) => {
                    tracing::warn!(%error, npub = %peer.npub, "ignoring invalid Drive peer snapshot");
                }
            }
        }
        let before_counts = self.retained_counts();
        self.recent.prune(now_ms, RECENT_PEERS_TTL_MS);
        changed || self.retained_counts() != before_counts
    }

    fn observation_due(&self, peer: &FipsEndpointPeer, now_ms: u64) -> bool {
        if !peer.connected {
            return false;
        }
        let Some(recent) = self.recent.peers.get(&peer.npub) else {
            return true;
        };
        let Some(addr) = peer.authenticated_udp_restart_addr() else {
            return now_ms.saturating_sub(recent.last_authenticated_at_ms)
                >= RECENT_PEERS_REFRESH_MS;
        };
        recent
            .endpoints
            .iter()
            .find(|endpoint| endpoint.addr.parse().ok() == Some(addr))
            .is_none_or(|endpoint| {
                now_ms.saturating_sub(endpoint.last_authenticated_at_ms) >= RECENT_PEERS_REFRESH_MS
            })
    }

    fn retained_counts(&self) -> (usize, usize) {
        (
            self.recent.peers.len(),
            self.recent
                .peers
                .values()
                .map(|peer| peer.endpoints.len())
                .sum(),
        )
    }

    pub(super) fn save(&self) {
        if let Err(error) = self.store.save(&self.recent) {
            tracing::warn!(
                %error,
                path = %self.store.path().display(),
                "saving Drive recent-peer cache failed"
            );
        }
    }
}

pub(super) fn recent_peers_file_path(device_path: &Path) -> PathBuf {
    device_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("fips-recent-peers.json")
}

fn merge_recent_udp_addresses(recent: &RecentPeers, peer_configs: &mut [FipsPeerConfig]) -> usize {
    let mut core_configs = peer_configs
        .iter()
        .map(|peer| PeerConfig {
            npub: peer.npub.clone(),
            addresses: peer
                .udp_addresses
                .iter()
                .map(|addr| PeerAddress::new("udp", addr))
                .collect(),
            ..PeerConfig::default()
        })
        .collect::<Vec<_>>();
    let merged = recent.merge_into_peer_configs(&mut core_configs);
    for (target, source) in peer_configs.iter_mut().zip(core_configs) {
        target.udp_addresses = source
            .addresses
            .into_iter()
            .filter(|address| address.transport == "udp")
            .map(|address| address.addr)
            .collect();
    }
    merged
}

fn empty_recent_peers(local_npub: &str, scope: &str) -> Option<RecentPeers> {
    RecentPeers::new(local_npub, scope)
        .map_err(|error: RecentPeersError| {
            tracing::warn!(%error, "creating empty Drive recent-peer cache failed");
        })
        .ok()
}

pub(super) fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use fips_core::{Identity, PeerIdentity, RecentPeer, RecentPeerEndpoint, RecentPeerTransport};

    fn identity(seed: u8) -> Identity {
        Identity::from_secret_bytes(&[seed; 32]).unwrap()
    }

    fn recent_peer(addr: &str, authenticated_at_ms: u64) -> RecentPeer {
        RecentPeer {
            last_authenticated_at_ms: authenticated_at_ms,
            endpoints: vec![RecentPeerEndpoint {
                transport: RecentPeerTransport::Udp,
                addr: addr.to_string(),
                last_authenticated_at_ms: authenticated_at_ms,
            }],
        }
    }

    fn connected_udp_peer(npub: String, addr: &str) -> FipsEndpointPeer {
        let identity = PeerIdentity::from_npub(&npub).unwrap();
        FipsEndpointPeer {
            npub,
            node_addr: *identity.node_addr(),
            connected: true,
            transport_addr: Some(addr.to_string()),
            transport_type: Some("udp".to_string()),
            link_id: 1,
            srtt_ms: None,
            srtt_age_ms: None,
            packets_sent: 0,
            packets_recv: 0,
            bytes_sent: 0,
            bytes_recv: 0,
            rekey_in_progress: false,
            rekey_draining: false,
            current_k_bit: None,
            last_outbound_route: None,
            direct_probe_pending: false,
            direct_probe_after_ms: None,
            direct_probe_retry_count: 0,
            direct_probe_auto_reconnect: false,
            direct_probe_expires_at_ms: None,
            nostr_traversal_consecutive_failures: 0,
            nostr_traversal_in_cooldown: false,
            nostr_traversal_cooldown_until_ms: None,
            nostr_traversal_last_observed_skew_ms: None,
        }
    }

    #[test]
    fn cached_routes_seed_only_existing_peer_membership() {
        let local = identity(1).npub();
        let known = identity(2).npub();
        let unknown = identity(3).npub();
        let mut recent = RecentPeers::new(local, "iris-drive:test").unwrap();
        recent
            .peers
            .insert(known.clone(), recent_peer("198.51.100.20:32112", 1_000));
        recent
            .peers
            .insert(unknown.clone(), recent_peer("198.51.100.30:32112", 1_000));
        let mut configs = vec![FipsPeerConfig {
            npub: known.clone(),
            udp_addresses: vec!["192.0.2.20:32112".to_string()],
        }];

        assert_eq!(merge_recent_udp_addresses(&recent, &mut configs), 1);

        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].npub, known);
        assert_eq!(
            configs[0].udp_addresses,
            ["192.0.2.20:32112", "198.51.100.20:32112"]
        );
        assert!(configs.iter().all(|peer| peer.npub != unknown));
    }

    #[test]
    fn authenticated_snapshot_persists_for_the_bound_identity_and_scope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fips-recent-peers.json");
        let local = identity(4).npub();
        let remote = identity(5).npub();
        let mut cache = DriveRecentPeers::load(&path, &local, "iris-drive:test", 1_000).unwrap();

        assert!(cache.observe(
            &[connected_udp_peer(remote.clone(), "198.51.100.50:32112")],
            2_000,
        ));
        cache.save();

        let stored = RecentPeersFileStore::new(&path, &local, "iris-drive:test")
            .unwrap()
            .load()
            .unwrap();
        assert_eq!(
            stored.peers[&remote].endpoints[0].addr,
            "198.51.100.50:32112"
        );
        assert_eq!(stored.peers[&remote].last_authenticated_at_ms, 2_000);
    }

    #[test]
    fn unchanged_authenticated_route_is_not_rewritten_on_every_status_poll() {
        let dir = tempfile::tempdir().unwrap();
        let local = identity(9).npub();
        let remote = identity(10).npub();
        let mut cache = DriveRecentPeers::load(
            dir.path().join("fips-recent-peers.json"),
            &local,
            "iris-drive:test",
            1_000,
        )
        .unwrap();
        let snapshot = connected_udp_peer(remote, "198.51.100.60:32112");

        assert!(cache.observe(std::slice::from_ref(&snapshot), 2_000));
        assert!(!cache.observe(std::slice::from_ref(&snapshot), 2_001));
        assert!(cache.observe(
            std::slice::from_ref(&snapshot),
            2_000 + RECENT_PEERS_REFRESH_MS
        ));
    }

    #[test]
    fn load_discards_invalid_cache_and_replaces_it_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fips-recent-peers.json");
        let local = identity(6).npub();
        std::fs::write(&path, "{not-json").unwrap();

        let cache = DriveRecentPeers::load(&path, &local, "iris-drive:test", 1_000).unwrap();

        assert!(cache.recent.peers.is_empty());
        cache.save();
        assert!(
            RecentPeersFileStore::new(&path, &local, "iris-drive:test")
                .unwrap()
                .load()
                .unwrap()
                .peers
                .is_empty()
        );
    }

    #[test]
    fn load_prunes_entries_older_than_seven_days() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fips-recent-peers.json");
        let local = identity(7).npub();
        let remote = identity(8).npub();
        let store = RecentPeersFileStore::new(&path, &local, "iris-drive:test").unwrap();
        let mut recent = RecentPeers::new(&local, "iris-drive:test").unwrap();
        recent
            .peers
            .insert(remote, recent_peer("198.51.100.80:32112", 1_000));
        store.save(&recent).unwrap();

        let cache = DriveRecentPeers::load(
            &path,
            &local,
            "iris-drive:test",
            1_000 + RECENT_PEERS_TTL_MS + 1,
        )
        .unwrap();

        assert!(cache.recent.peers.is_empty());
    }
}
