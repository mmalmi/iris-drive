use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use fips_core::PeerIdentity;
use hashtree_core::Store;

use super::{FipsBlockSync, FipsPeerStatus};

// Native FSP receiver reports can be up to ten seconds apart.
const ROUTED_PRESENCE_FEEDBACK_WINDOW: Duration = Duration::from_secs(20);

pub struct FipsPeerPresence {
    pub direct: Vec<String>,
    pub mesh: Vec<String>,
    pub statuses: Vec<FipsPeerStatus>,
}

impl<L: Store + Send + Sync + 'static> FipsBlockSync<L> {
    /// One observation supplies the route labels and the per-device status rows.
    pub async fn peer_presence(&self) -> FipsPeerPresence {
        let mut direct = BTreeSet::new();
        let mut statuses = BTreeMap::new();
        for peer in self.endpoint_peers(false).await {
            if peer.connected {
                direct.insert(peer.npub.clone());
            }
            statuses.insert(
                peer.npub.clone(),
                FipsPeerStatus {
                    npub: peer.npub,
                    connected: peer.connected,
                    transport_addr: peer.transport_addr,
                    transport_type: peer.transport_type,
                    srtt_ms: peer.srtt_ms,
                    packets_sent: peer.packets_sent,
                    packets_recv: peer.packets_recv,
                    bytes_sent: peer.bytes_sent,
                    bytes_recv: peer.bytes_recv,
                },
            );
        }
        let mesh = self.routed_presence(&direct).await;
        for npub in &mesh {
            statuses.insert(
                npub.clone(),
                FipsPeerStatus {
                    npub: npub.clone(),
                    connected: true,
                    transport_addr: None,
                    transport_type: None,
                    srtt_ms: None,
                    packets_sent: 0,
                    packets_recv: 0,
                    bytes_sent: 0,
                    bytes_recv: 0,
                },
            );
        }
        FipsPeerPresence {
            direct: direct.into_iter().collect(),
            mesh: mesh.into_iter().collect(),
            statuses: statuses.into_values().collect(),
        }
    }

    async fn routed_presence(&self, direct: &BTreeSet<String>) -> BTreeSet<String> {
        let mut mesh = BTreeSet::new();
        if let Some(control) = &self.control_runtime {
            match control.connected_peer_ids().await {
                Ok(peers) => mesh.extend(peers.into_iter().filter(|peer| !direct.contains(peer))),
                Err(error) => tracing::warn!(%error, "failed to snapshot Drive FIPS control peers"),
            }
        }
        // Browsers use shared Nostr pubsub without the Drive control service.
        // FSP feedback is authenticated by the destination's session. Cached
        // advertisements, past handshakes and outgoing attempts are insufficient.
        for npub in self.authorized_peer_ids() {
            if direct.contains(&npub) || mesh.contains(&npub) {
                continue;
            }
            let Ok(identity) = PeerIdentity::from_npub(&npub) else {
                continue;
            };
            if self
                .endpoint
                .source_route_quality(identity, ROUTED_PRESENCE_FEEDBACK_WINDOW)
                .await
                .is_ok_and(|quality| {
                    quality.has_recent_delivery_feedback && !quality.delivery_feedback_timed_out
                })
            {
                mesh.insert(npub);
            }
        }
        mesh
    }

    pub async fn mesh_peer_ids(&self) -> Vec<String> {
        self.peer_presence().await.mesh
    }

    pub async fn fips_peer_statuses(&self) -> Vec<FipsPeerStatus> {
        self.peer_presence().await.statuses
    }
}
