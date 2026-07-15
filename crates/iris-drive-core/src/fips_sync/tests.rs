use super::*;
use hashtree_core::{DirEntry, LinkType, MemoryStore};
use hashtree_fips_transport::{FipsEndpointIo, FipsEndpointPacket, FipsTransportError};
use nostr_sdk::{Alphabet, EventBuilder, Keys, Kind, SingleLetterTag, Tag, TagKind, ToBech32};
use tokio::sync::{Mutex as TokioMutex, mpsc};

mod app_key_link_peers;
mod mesh_fallback;

type PacketSenderMap =
    Arc<TokioMutex<std::collections::HashMap<String, mpsc::UnboundedSender<FipsEndpointPacket>>>>;
type PeerLinkMap = Arc<TokioMutex<std::collections::BTreeMap<String, Vec<String>>>>;

struct FakeEndpoint {
    id: String,
    network: PacketSenderMap,
    links: Option<PeerLinkMap>,
    rx: TokioMutex<mpsc::UnboundedReceiver<FipsEndpointPacket>>,
}

impl FakeEndpoint {
    async fn new(id: &str, network: PacketSenderMap) -> Arc<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        network.lock().await.insert(id.to_string(), tx);
        Arc::new(Self {
            id: id.to_string(),
            network,
            links: None,
            rx: TokioMutex::new(rx),
        })
    }

    async fn new_linked(id: &str, network: PacketSenderMap, links: PeerLinkMap) -> Arc<Self> {
        let (tx, rx) = mpsc::unbounded_channel();
        network.lock().await.insert(id.to_string(), tx);
        Arc::new(Self {
            id: id.to_string(),
            network,
            links: Some(links),
            rx: TokioMutex::new(rx),
        })
    }

    async fn visible_peers(&self) -> Vec<String> {
        if let Some(links) = self.links.as_ref() {
            return links
                .lock()
                .await
                .get(&self.id)
                .cloned()
                .unwrap_or_default();
        }
        self.network
            .lock()
            .await
            .keys()
            .filter(|id| *id != &self.id)
            .cloned()
            .collect()
    }
}

#[async_trait]
impl FipsEndpointIo for FakeEndpoint {
    async fn send(&self, peer_id: &str, data: Vec<u8>) -> Result<(), FipsTransportError> {
        if !self
            .visible_peers()
            .await
            .iter()
            .any(|peer| peer == peer_id)
        {
            return Err(FipsTransportError::Send(format!(
                "peer {peer_id} is not linked from {}",
                self.id
            )));
        }
        let tx = self
            .network
            .lock()
            .await
            .get(peer_id)
            .cloned()
            .ok_or_else(|| FipsTransportError::Send(format!("unknown peer {peer_id}")))?;
        tx.send(FipsEndpointPacket {
            peer_id: self.id.clone(),
            data,
        })
        .map_err(|_| FipsTransportError::Send("receiver closed".to_string()))
    }

    async fn recv(&self) -> Option<FipsEndpointPacket> {
        self.rx.lock().await.recv().await
    }

    async fn peer_ids(&self) -> Vec<String> {
        self.visible_peers().await
    }

    fn local_peer_id(&self) -> Option<String> {
        Some(self.id.clone())
    }
}

async fn wait_for_mesh_neighbors(mesh: &FipsMeshPubsub<MemoryStore>, expected: &[&str]) -> bool {
    for _ in 0..50 {
        let peers = mesh.peer_ids().await;
        if expected
            .iter()
            .all(|expected_peer| peers.iter().any(|peer| peer == expected_peer))
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

mod settings;
mod transport;
