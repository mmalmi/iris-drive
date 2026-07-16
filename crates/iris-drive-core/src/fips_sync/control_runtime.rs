//! Bounded Drive control records over one reliable TCP/FIPS service.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fips_core::discovery::local::LocalInstanceCapability;
use fips_core::{FipsEndpoint, PeerIdentity};
use fips_tcp::{Config as TcpConfig, ConnectionId, MarkerStatus, SendMarker, State};
use fips_tcp_endpoint::FipsTcpEndpoint;
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::task::JoinHandle;

use super::FipsSyncError;

pub const DRIVE_CONTROL_CAPABILITY: &str = "iris.drive.control/1";
pub const DRIVE_CONTROL_SERVICE_PORT: u16 = 39_019;
pub const DRIVE_CONTROL_MAX_PAYLOAD_BYTES: usize = 256 * 1024;

const WIRE_VERSION: u8 = 1;
const RECORD_ID_BYTES: usize = 16;
const MAX_TOPIC_BYTES: usize = 256;
const MAX_PEERS: usize = 64;
const MAX_BOOTSTRAP_CONNECTIONS: usize = 8;
const MAX_QUEUED_RECORDS_PER_PEER: usize = 32;
const MAX_QUEUED_BYTES_PER_PEER: usize = 512 * 1024;
const MAX_RECORDS_PER_TURN: usize = 64;
const IO_CHUNK_BYTES: usize = 16 * 1024;
const COMMAND_CAPACITY: usize = 256;
const DELIVERY_CAPACITY: usize = 1024;
const SEEN_RECORD_CAPACITY: usize = 1024;
pub(super) const BOOTSTRAP_STREAM_LIFETIME_MS: u64 = 5_000;
const QUEUED_RECORD_LIFETIME_MS: u64 = 60_000;
const RECONNECT_DELAY_MS: u64 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FipsAppMessage {
    pub peer_id: String,
    pub topic: String,
    pub data: Vec<u8>,
}

pub(super) struct DriveControlRuntime {
    commands: mpsc::Sender<Command>,
    deliveries: broadcast::Sender<FipsAppMessage>,
    task: Option<JoinHandle<()>>,
}

impl DriveControlRuntime {
    pub(super) async fn bind(
        endpoint: Arc<FipsEndpoint>,
        authorized_peers: BTreeSet<String>,
        bootstrap_topics: BTreeSet<&'static str>,
    ) -> Result<Self, FipsSyncError> {
        validate_peer_count(&authorized_peers)?;
        let tcp = FipsTcpEndpoint::bind_with_capability(
            endpoint.clone(),
            LocalInstanceCapability::service(DRIVE_CONTROL_CAPABILITY, DRIVE_CONTROL_SERVICE_PORT),
            TcpConfig {
                receive_buffer: u16::MAX as usize,
                send_buffer: MAX_QUEUED_BYTES_PER_PEER,
                max_connections: MAX_PEERS * 2,
                max_connections_per_peer: 2,
                ..TcpConfig::default()
            },
            unix_millis(),
        )
        .await
        .map_err(|error| FipsSyncError::Endpoint(error.to_string()))?;
        let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (deliveries, _) = broadcast::channel(DELIVERY_CAPACITY);
        let actor = ControlActor {
            local_npub: endpoint.npub().to_string(),
            tcp,
            authorized_peers,
            bootstrap_topics,
            connections: HashMap::new(),
            active: BTreeMap::new(),
            inputs: HashMap::new(),
            queues: BTreeMap::new(),
            seen_records: HashSet::new(),
            seen_record_order: VecDeque::new(),
            deliveries: deliveries.clone(),
        };
        let task = tokio::spawn(actor.run(command_rx));
        Ok(Self {
            commands,
            deliveries,
            task: Some(task),
        })
    }

    pub(super) fn subscribe(&self) -> broadcast::Receiver<FipsAppMessage> {
        self.deliveries.subscribe()
    }

    pub(super) async fn set_policy(
        &self,
        authorized_peers: BTreeSet<String>,
        bootstrap_topics: BTreeSet<&'static str>,
    ) -> Result<(), FipsSyncError> {
        validate_peer_count(&authorized_peers)?;
        self.request(|reply| Command::SetPolicy {
            authorized_peers,
            bootstrap_topics,
            reply,
        })
        .await
    }

    pub(super) async fn send(
        &self,
        peer_id: String,
        topic: String,
        data: Vec<u8>,
    ) -> Result<(), FipsSyncError> {
        self.request(|reply| Command::Send {
            peer_id,
            topic,
            data,
            reply,
        })
        .await
    }

    pub(super) async fn broadcast(
        &self,
        topic: String,
        data: Vec<u8>,
    ) -> Result<usize, FipsSyncError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(Command::Broadcast { topic, data, reply })
            .await
            .map_err(|_| closed())?;
        response.await.map_err(|_| closed())?
    }

    pub(super) async fn shutdown(&mut self) -> Result<(), FipsSyncError> {
        let (reply, response) = oneshot::channel();
        if self
            .commands
            .send(Command::Shutdown { reply })
            .await
            .is_ok()
        {
            let _ = response.await;
        }
        if let Some(task) = self.task.take() {
            task.await
                .map_err(|error| FipsSyncError::Endpoint(error.to_string()))?;
        }
        Ok(())
    }

    async fn request(
        &self,
        command: impl FnOnce(oneshot::Sender<Result<(), FipsSyncError>>) -> Command,
    ) -> Result<(), FipsSyncError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(command(reply))
            .await
            .map_err(|_| closed())?;
        response.await.map_err(|_| closed())?
    }
}

impl Drop for DriveControlRuntime {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

enum Command {
    SetPolicy {
        authorized_peers: BTreeSet<String>,
        bootstrap_topics: BTreeSet<&'static str>,
        reply: oneshot::Sender<Result<(), FipsSyncError>>,
    },
    Send {
        peer_id: String,
        topic: String,
        data: Vec<u8>,
        reply: oneshot::Sender<Result<(), FipsSyncError>>,
    },
    Broadcast {
        topic: String,
        data: Vec<u8>,
        reply: oneshot::Sender<Result<usize, FipsSyncError>>,
    },
    Shutdown {
        reply: oneshot::Sender<()>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Direction {
    Inbound,
    Outbound,
}

struct TrackedConnection {
    peer: String,
    direction: Direction,
    opened_at_ms: u64,
}

#[derive(Default)]
struct PeerQueue {
    records: VecDeque<QueuedRecord>,
    bytes: usize,
    next_attempt_ms: u64,
}

struct QueuedRecord {
    bytes: Vec<u8>,
    offset: usize,
    ack_marker: Option<SendMarker>,
    expires_at_ms: u64,
}

impl PeerQueue {
    fn rewind_after_stream_change(&mut self) {
        for record in &mut self.records {
            record.offset = 0;
            record.ack_marker = None;
        }
    }

    fn pop_front(&mut self) {
        if let Some(record) = self.records.pop_front() {
            self.bytes = self.bytes.saturating_sub(record.bytes.len());
        }
    }

    fn expire_unstarted(&mut self, now_ms: u64) {
        while self
            .records
            .front()
            .is_some_and(|record| record.offset == 0 && now_ms >= record.expires_at_ms)
        {
            self.pop_front();
        }
    }
}

#[derive(Default)]
struct RecordDecoder {
    bytes: Vec<u8>,
}

#[derive(Debug, PartialEq, Eq)]
struct DecodedRecord {
    id: [u8; RECORD_ID_BYTES],
    topic: String,
    data: Vec<u8>,
}

struct ControlActor {
    local_npub: String,
    tcp: FipsTcpEndpoint,
    authorized_peers: BTreeSet<String>,
    bootstrap_topics: BTreeSet<&'static str>,
    connections: HashMap<ConnectionId, TrackedConnection>,
    active: BTreeMap<String, ConnectionId>,
    inputs: HashMap<String, RecordDecoder>,
    queues: BTreeMap<String, PeerQueue>,
    seen_records: HashSet<(String, [u8; RECORD_ID_BYTES])>,
    seen_record_order: VecDeque<(String, [u8; RECORD_ID_BYTES])>,
    deliveries: broadcast::Sender<FipsAppMessage>,
}

impl ControlActor {
    async fn run(mut self, mut commands: mpsc::Receiver<Command>) {
        let mut tick = tokio::time::interval(Duration::from_millis(20));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            let now_ms = unix_millis();
            tokio::select! {
                command = commands.recv() => {
                    let Some(command) = command else { break };
                    if self.handle_command(command, now_ms).await { break; }
                }
                received = self.tcp.receive_report(now_ms) => {
                    if let Err(error) = received {
                        tracing::warn!(%error, "Drive control TCP/FIPS receiver stopped");
                        break;
                    }
                }
                _ = tick.tick() => {
                    if let Err(error) = self.tcp.poll(now_ms).await {
                        tracing::warn!(%error, "Drive control TCP/FIPS poll failed");
                    }
                }
            }
            if let Err(error) = self.drive(now_ms).await {
                tracing::warn!(%error, "Drive control TCP/FIPS turn failed");
            }
        }
    }

    async fn handle_command(&mut self, command: Command, now_ms: u64) -> bool {
        match command {
            Command::SetPolicy {
                authorized_peers,
                bootstrap_topics,
                reply,
            } => {
                self.authorized_peers = authorized_peers;
                self.bootstrap_topics = bootstrap_topics;
                self.drop_disallowed_queues();
                let result = self.close_disallowed_connections().await;
                let _ = reply.send(result);
            }
            Command::Send {
                peer_id,
                topic,
                data,
                reply,
            } => {
                let result = self.queue(&peer_id, &topic, &data, now_ms);
                let _ = reply.send(result);
            }
            Command::Broadcast { topic, data, reply } => {
                let peers = self.authorized_peers.iter().cloned().collect::<Vec<_>>();
                let mut sent = 0;
                let mut first_error = None;
                for peer in peers {
                    match self.queue(&peer, &topic, &data, now_ms) {
                        Ok(()) => sent += 1,
                        Err(error) if first_error.is_none() => first_error = Some(error),
                        Err(_) => {}
                    }
                }
                let result = if sent == 0 {
                    first_error.map_or(Ok(0), Err)
                } else {
                    Ok(sent)
                };
                let _ = reply.send(result);
            }
            Command::Shutdown { reply } => {
                let ids = self.connections.keys().copied().collect::<Vec<_>>();
                for id in ids {
                    let _ = self.tcp.close(id, now_ms).await;
                }
                let _ = self.tcp.poll(now_ms).await;
                let _ = reply.send(());
                return true;
            }
        }
        false
    }

    fn queue(
        &mut self,
        peer_id: &str,
        topic: &str,
        data: &[u8],
        now_ms: u64,
    ) -> Result<(), FipsSyncError> {
        if !self.authorized_peers.contains(peer_id) && !self.bootstrap_topics.contains(topic) {
            return Err(endpoint_error("Drive control peer/topic is not authorized"));
        }
        PeerIdentity::from_npub(peer_id)
            .map_err(|error| FipsSyncError::Identity(error.to_string()))?;
        let record = encode_record(topic, data)?;
        if !self.queues.contains_key(peer_id) && self.queues.len() >= MAX_PEERS {
            return Err(endpoint_error("Drive control queue peer limit reached"));
        }
        let queue = self.queues.entry(peer_id.to_string()).or_default();
        if queue
            .records
            .iter()
            .any(|queued| same_logical_record(&queued.bytes, &record))
        {
            queue.next_attempt_ms = queue.next_attempt_ms.min(now_ms);
            return Ok(());
        }
        if queue.records.len() >= MAX_QUEUED_RECORDS_PER_PEER
            || queue.bytes.saturating_add(record.len()) > MAX_QUEUED_BYTES_PER_PEER
        {
            return Err(endpoint_error("Drive control peer queue is full"));
        }
        queue.bytes = queue.bytes.saturating_add(record.len());
        queue.records.push_back(QueuedRecord {
            bytes: record,
            offset: 0,
            ack_marker: None,
            expires_at_ms: now_ms.saturating_add(QUEUED_RECORD_LIFETIME_MS),
        });
        queue.next_attempt_ms = queue.next_attempt_ms.min(now_ms);
        Ok(())
    }

    async fn ensure_connection(&mut self, peer_id: &str, now_ms: u64) -> Result<(), FipsSyncError> {
        if self.connections.iter().any(|(id, connection)| {
            connection.peer == peer_id
                && matches!(
                    self.tcp.state(*id),
                    Some(
                        State::SynSent | State::SynReceived | State::Established | State::CloseWait
                    )
                )
        }) {
            return Ok(());
        }
        let peer = PeerIdentity::from_npub(peer_id)
            .map_err(|error| FipsSyncError::Identity(error.to_string()))?;
        let id = self
            .tcp
            .connect(peer, now_ms)
            .await
            .map_err(|error| endpoint_error(error.to_string()))?;
        self.connections.insert(
            id,
            TrackedConnection {
                peer: peer_id.to_string(),
                direction: Direction::Outbound,
                opened_at_ms: now_ms,
            },
        );
        Ok(())
    }

    async fn drive(&mut self, now_ms: u64) -> Result<(), FipsSyncError> {
        self.accept(now_ms).await?;
        self.select_streams().await?;
        self.expire_bootstrap_connections(now_ms).await?;
        self.expire_queues(now_ms);
        self.reconnect_queued(now_ms).await;
        self.read(now_ms).await?;
        self.flush(now_ms).await?;
        Ok(())
    }

    async fn reconnect_queued(&mut self, now_ms: u64) {
        let peers = self
            .queues
            .iter()
            .filter(|(_, queue)| now_ms >= queue.next_attempt_ms)
            .map(|(peer, _)| peer.clone())
            .collect::<Vec<_>>();
        for peer in peers {
            if let Err(error) = self.ensure_connection(&peer, now_ms).await {
                if let Some(queue) = self.queues.get_mut(&peer) {
                    queue.next_attempt_ms = now_ms.saturating_add(RECONNECT_DELAY_MS);
                }
                tracing::debug!(%peer, %error, "Drive control reconnect deferred");
            }
        }
    }

    async fn accept(&mut self, now_ms: u64) -> Result<(), FipsSyncError> {
        while let Some(id) = self.tcp.accept() {
            let Some(peer) = self.tcp.peer(id).map(|peer| peer.npub()) else {
                self.tcp
                    .abort(id)
                    .await
                    .map_err(|error| endpoint_error(error.to_string()))?;
                continue;
            };
            let is_authorized = self.authorized_peers.contains(&peer);
            let bootstrap_connections = self
                .connections
                .values()
                .filter(|connection| !self.authorized_peers.contains(&connection.peer))
                .count();
            if !is_authorized
                && (self.bootstrap_topics.is_empty()
                    || bootstrap_connections >= MAX_BOOTSTRAP_CONNECTIONS)
            {
                self.tcp
                    .abort(id)
                    .await
                    .map_err(|error| endpoint_error(error.to_string()))?;
                continue;
            }
            let distinct = self
                .connections
                .values()
                .map(|connection| connection.peer.as_str())
                .collect::<BTreeSet<_>>();
            if !distinct.contains(peer.as_str()) && distinct.len() >= MAX_PEERS {
                self.tcp
                    .abort(id)
                    .await
                    .map_err(|error| endpoint_error(error.to_string()))?;
                continue;
            }
            self.connections.entry(id).or_insert(TrackedConnection {
                peer,
                direction: Direction::Inbound,
                opened_at_ms: now_ms,
            });
        }
        Ok(())
    }

    async fn select_streams(&mut self) -> Result<(), FipsSyncError> {
        self.connections
            .retain(|id, _| self.tcp.state(*id).is_some());
        let mut candidates = BTreeMap::<String, Vec<(ConnectionId, Direction)>>::new();
        for (id, connection) in &self.connections {
            if matches!(
                self.tcp.state(*id),
                Some(State::Established | State::CloseWait)
            ) {
                candidates
                    .entry(connection.peer.clone())
                    .or_default()
                    .push((*id, connection.direction));
            }
        }
        let mut next = BTreeMap::new();
        let mut extras = Vec::new();
        for (peer, mut streams) in candidates {
            let prefer_outbound = self.local_npub < peer;
            streams.sort_by_key(|(id, direction)| {
                let preferred = (*direction == Direction::Outbound) == prefer_outbound;
                (!preferred, id.get())
            });
            let (selected, _) = streams.remove(0);
            next.insert(peer, selected);
            extras.extend(streams.into_iter().map(|(id, _)| id));
        }
        for id in extras {
            self.tcp
                .abort(id)
                .await
                .map_err(|error| endpoint_error(error.to_string()))?;
            self.connections.remove(&id);
        }
        let changed = self
            .active
            .keys()
            .chain(next.keys())
            .filter(|peer| self.active.get(*peer) != next.get(*peer))
            .cloned()
            .collect::<BTreeSet<_>>();
        self.active = next;
        for peer in changed {
            self.inputs.remove(&peer);
            self.rewind_queue(&peer);
        }
        Ok(())
    }

    async fn read(&mut self, now_ms: u64) -> Result<(), FipsSyncError> {
        let streams = self
            .active
            .iter()
            .map(|(peer, id)| (peer.clone(), *id))
            .collect::<Vec<_>>();
        for (peer, id) in streams {
            for _ in 0..MAX_RECORDS_PER_TURN {
                let decoded = match self.decode_next(&peer) {
                    Ok(decoded) => decoded,
                    Err(error) => {
                        self.tcp
                            .abort(id)
                            .await
                            .map_err(|error| endpoint_error(error.to_string()))?;
                        self.inputs.remove(&peer);
                        tracing::debug!(%peer, %error, "rejected malformed Drive control record");
                        break;
                    }
                };
                if let Some(record) = decoded {
                    let authorized = self.authorized_peers.contains(&peer);
                    if (authorized || self.bootstrap_topics.contains(record.topic.as_str()))
                        && self.mark_record_seen(&peer, record.id)
                    {
                        let _ = self.deliveries.send(FipsAppMessage {
                            peer_id: peer.clone(),
                            topic: record.topic,
                            data: record.data,
                        });
                    }
                    if !authorized {
                        self.abort_connection(id, &peer).await?;
                        break;
                    }
                    continue;
                }
                let bytes = self
                    .tcp
                    .read(id, IO_CHUNK_BYTES, now_ms)
                    .await
                    .map_err(|error| endpoint_error(error.to_string()))?;
                if bytes.is_empty() {
                    break;
                }
                let input = self.inputs.entry(peer.clone()).or_default();
                if input.bytes.len().saturating_add(bytes.len())
                    > DRIVE_CONTROL_MAX_PAYLOAD_BYTES + MAX_TOPIC_BYTES + 32
                {
                    self.tcp
                        .abort(id)
                        .await
                        .map_err(|error| endpoint_error(error.to_string()))?;
                    self.inputs.remove(&peer);
                    break;
                }
                input.bytes.extend_from_slice(&bytes);
            }
        }
        Ok(())
    }

    fn decode_next(&mut self, peer: &str) -> Result<Option<DecodedRecord>, FipsSyncError> {
        let Some(input) = self.inputs.get_mut(peer) else {
            return Ok(None);
        };
        decode_record(&mut input.bytes)
    }

    async fn flush(&mut self, now_ms: u64) -> Result<(), FipsSyncError> {
        let streams = self
            .active
            .iter()
            .map(|(peer, id)| (peer.clone(), *id))
            .collect::<Vec<_>>();
        for (peer, id) in streams {
            loop {
                let ack_status = self
                    .queues
                    .get(&peer)
                    .and_then(|queue| queue.records.front())
                    .filter(|record| record.offset == record.bytes.len())
                    .and_then(|record| record.ack_marker.as_ref())
                    .map(|marker| self.tcp.marker_status(marker));
                match ack_status {
                    Some(MarkerStatus::Acked) => {
                        self.pop_front_record(&peer);
                        continue;
                    }
                    Some(MarkerStatus::Pending) => break,
                    Some(MarkerStatus::ConnectionGone) => self.rewind_queue(&peer),
                    None => {}
                }

                let Some(bytes) = self.queues.get(&peer).and_then(|queue| {
                    queue.records.front().map(|record| {
                        let end = record
                            .offset
                            .saturating_add(IO_CHUNK_BYTES)
                            .min(record.bytes.len());
                        record.bytes[record.offset..end].to_vec()
                    })
                }) else {
                    break;
                };
                let (accepted, marker) = self
                    .tcp
                    .write_with_marker(id, &bytes, now_ms)
                    .await
                    .map_err(|error| endpoint_error(error.to_string()))?;
                if accepted == 0 {
                    break;
                }
                let queue = self
                    .queues
                    .get_mut(&peer)
                    .expect("Drive control queue exists");
                let record = queue
                    .records
                    .front_mut()
                    .expect("Drive control record exists");
                record.offset += accepted;
                if record.offset == record.bytes.len() {
                    record.ack_marker = Some(marker);
                }
            }
            if self
                .queues
                .get(&peer)
                .is_some_and(|queue| queue.records.is_empty())
            {
                self.queues.remove(&peer);
            }
        }
        Ok(())
    }

    fn drop_disallowed_queues(&mut self) {
        self.queues
            .retain(|peer, _| self.authorized_peers.contains(peer));
    }

    async fn close_disallowed_connections(&mut self) -> Result<(), FipsSyncError> {
        let disallowed = self
            .connections
            .iter()
            .filter(|(_, connection)| !self.authorized_peers.contains(&connection.peer))
            .map(|(id, connection)| (*id, connection.peer.clone()))
            .collect::<Vec<_>>();
        for (id, peer) in disallowed {
            self.abort_connection(id, &peer).await?;
        }
        Ok(())
    }

    async fn expire_bootstrap_connections(&mut self, now_ms: u64) -> Result<(), FipsSyncError> {
        let expired = self
            .connections
            .iter()
            .filter(|(_, connection)| {
                !self.authorized_peers.contains(&connection.peer)
                    && now_ms.saturating_sub(connection.opened_at_ms)
                        >= BOOTSTRAP_STREAM_LIFETIME_MS
            })
            .map(|(id, connection)| (*id, connection.peer.clone()))
            .collect::<Vec<_>>();
        for (id, peer) in expired {
            self.abort_connection(id, &peer).await?;
            if let Some(queue) = self.queues.get_mut(&peer) {
                queue.next_attempt_ms = now_ms.saturating_add(RECONNECT_DELAY_MS);
            }
        }
        Ok(())
    }

    async fn abort_connection(
        &mut self,
        id: ConnectionId,
        peer: &str,
    ) -> Result<(), FipsSyncError> {
        if self.tcp.state(id).is_some() {
            self.tcp
                .abort(id)
                .await
                .map_err(|error| endpoint_error(error.to_string()))?;
        }
        self.connections.remove(&id);
        if self.active.get(peer) == Some(&id) {
            self.active.remove(peer);
            self.inputs.remove(peer);
            self.rewind_queue(peer);
        }
        Ok(())
    }

    fn expire_queues(&mut self, now_ms: u64) {
        self.queues.retain(|_, queue| {
            queue.expire_unstarted(now_ms);
            !queue.records.is_empty()
        });
    }

    fn rewind_queue(&mut self, peer: &str) {
        if let Some(queue) = self.queues.get_mut(peer) {
            queue.rewind_after_stream_change();
        }
    }

    fn pop_front_record(&mut self, peer: &str) {
        let Some(queue) = self.queues.get_mut(peer) else {
            return;
        };
        queue.pop_front();
    }

    fn mark_record_seen(&mut self, peer: &str, id: [u8; RECORD_ID_BYTES]) -> bool {
        let key = (peer.to_string(), id);
        if !self.seen_records.insert(key.clone()) {
            return false;
        }
        self.seen_record_order.push_back(key);
        while self.seen_record_order.len() > SEEN_RECORD_CAPACITY {
            if let Some(expired) = self.seen_record_order.pop_front() {
                self.seen_records.remove(&expired);
            }
        }
        true
    }
}

pub(super) fn encode_record(topic: &str, data: &[u8]) -> Result<Vec<u8>, FipsSyncError> {
    let topic = topic.trim();
    if topic.is_empty() || topic.len() > MAX_TOPIC_BYTES {
        return Err(endpoint_error("invalid Drive control topic length"));
    }
    if data.len() > DRIVE_CONTROL_MAX_PAYLOAD_BYTES {
        return Err(endpoint_error("Drive control payload exceeds limit"));
    }
    let topic_len = u16::try_from(topic.len())
        .map_err(|_| endpoint_error("Drive control topic exceeds wire limit"))?;
    let data_len = u32::try_from(data.len())
        .map_err(|_| endpoint_error("Drive control payload exceeds wire limit"))?;
    let body_len = 1usize
        .checked_add(RECORD_ID_BYTES)
        .and_then(|len| len.checked_add(2))
        .and_then(|len| len.checked_add(4))
        .and_then(|len| len.checked_add(topic.len()))
        .and_then(|len| len.checked_add(data.len()))
        .ok_or_else(|| endpoint_error("Drive control frame length overflow"))?;
    let body_len = u32::try_from(body_len)
        .map_err(|_| endpoint_error("Drive control frame exceeds wire limit"))?;
    let mut out = Vec::with_capacity(4 + body_len as usize);
    out.extend_from_slice(&body_len.to_be_bytes());
    out.push(WIRE_VERSION);
    out.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    out.extend_from_slice(&topic_len.to_be_bytes());
    out.extend_from_slice(&data_len.to_be_bytes());
    out.extend_from_slice(topic.as_bytes());
    out.extend_from_slice(data);
    Ok(out)
}

fn decode_record(input: &mut Vec<u8>) -> Result<Option<DecodedRecord>, FipsSyncError> {
    if input.len() < 4 {
        return Ok(None);
    }
    let body_len = u32::from_be_bytes(input[..4].try_into().expect("four bytes")) as usize;
    let header_len = 1 + RECORD_ID_BYTES + 2 + 4;
    if !(header_len..=DRIVE_CONTROL_MAX_PAYLOAD_BYTES + MAX_TOPIC_BYTES + header_len)
        .contains(&body_len)
    {
        return Err(endpoint_error("invalid Drive control record length"));
    }
    if input.len() < 4 + body_len {
        return Ok(None);
    }
    let body = &input[4..4 + body_len];
    if body[0] != WIRE_VERSION {
        return Err(endpoint_error("unsupported Drive control wire version"));
    }
    let id = body[1..=RECORD_ID_BYTES]
        .try_into()
        .expect("record ID bytes");
    let topic_len = u16::from_be_bytes(
        body[1 + RECORD_ID_BYTES..3 + RECORD_ID_BYTES]
            .try_into()
            .expect("two bytes"),
    ) as usize;
    let data_len = u32::from_be_bytes(
        body[3 + RECORD_ID_BYTES..header_len]
            .try_into()
            .expect("four bytes"),
    ) as usize;
    if topic_len == 0
        || topic_len > MAX_TOPIC_BYTES
        || data_len > DRIVE_CONTROL_MAX_PAYLOAD_BYTES
        || header_len + topic_len + data_len != body_len
    {
        return Err(endpoint_error("invalid Drive control record fields"));
    }
    let topic = std::str::from_utf8(&body[header_len..header_len + topic_len])
        .map_err(|_| endpoint_error("Drive control topic is not UTF-8"))?
        .to_string();
    let data = body[header_len + topic_len..].to_vec();
    input.drain(..4 + body_len);
    Ok(Some(DecodedRecord { id, topic, data }))
}

fn same_logical_record(left: &[u8], right: &[u8]) -> bool {
    const ID_START: usize = 4 + 1;
    const ID_END: usize = ID_START + RECORD_ID_BYTES;

    left.len() == right.len()
        && left.len() >= ID_END
        && left[..ID_START] == right[..ID_START]
        && left[ID_END..] == right[ID_END..]
}

fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn closed() -> FipsSyncError {
    endpoint_error("Drive control runtime is closed")
}

fn validate_peer_count(peers: &BTreeSet<String>) -> Result<(), FipsSyncError> {
    if peers.len() > MAX_PEERS {
        return Err(endpoint_error(
            "Drive control authorized peer limit reached",
        ));
    }
    Ok(())
}

fn endpoint_error(error: impl Into<String>) -> FipsSyncError {
    FipsSyncError::Endpoint(error.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_record_roundtrip_and_limit() {
        let record = encode_record("iris-drive/test", &[1, 2, 3]).unwrap();
        let mut input = record;
        let decoded = decode_record(&mut input).unwrap().unwrap();
        assert_eq!(decoded.topic, "iris-drive/test");
        assert_eq!(decoded.data, vec![1, 2, 3]);
        assert!(input.is_empty());
        let oversized = vec![0; DRIVE_CONTROL_MAX_PAYLOAD_BYTES + 1];
        assert!(encode_record("test", &oversized).is_err());
    }

    #[test]
    fn partial_record_waits_without_consuming() {
        let record = encode_record("test", &[1, 2, 3]).unwrap();
        let mut input = record[..record.len() - 1].to_vec();
        let before = input.clone();
        assert_eq!(decode_record(&mut input).unwrap(), None);
        assert_eq!(input, before);
    }

    #[test]
    fn retry_coalescing_ignores_only_the_random_record_id() {
        let first = encode_record("test", b"same payload").unwrap();
        let retry = encode_record("test", b"same payload").unwrap();
        let other_topic = encode_record("other", b"same payload").unwrap();
        let other_payload = encode_record("test", b"other payload").unwrap();

        assert_ne!(first, retry);
        assert!(same_logical_record(&first, &retry));
        assert!(!same_logical_record(&first, &other_topic));
        assert!(!same_logical_record(&first, &other_payload));
    }

    #[test]
    fn reconnect_rewinds_the_whole_queue_without_reordering() {
        let interrupted = encode_record("test", b"interrupted").unwrap();
        let waiting = encode_record("test", b"waiting").unwrap();
        let mut queue = PeerQueue {
            bytes: interrupted.len() + waiting.len(),
            records: VecDeque::from([
                QueuedRecord {
                    bytes: interrupted.clone(),
                    offset: 5,
                    ack_marker: None,
                    expires_at_ms: 100,
                },
                QueuedRecord {
                    bytes: waiting.clone(),
                    offset: 0,
                    ack_marker: None,
                    expires_at_ms: 100,
                },
            ]),
            next_attempt_ms: 0,
        };

        queue.rewind_after_stream_change();

        assert_eq!(queue.records.front().unwrap().offset, 0);
        assert_eq!(queue.records.front().unwrap().bytes, interrupted);
        assert_eq!(queue.records.back().unwrap().offset, 0);
        assert_eq!(queue.records.back().unwrap().bytes, waiting);
        assert_eq!(
            queue.bytes,
            queue
                .records
                .iter()
                .map(|record| record.bytes.len())
                .sum::<usize>()
        );
    }

    #[test]
    fn expiry_never_discards_a_partial_record_tail() {
        let bytes = encode_record("test", b"payload").unwrap();
        let mut queue = PeerQueue {
            bytes: bytes.len(),
            records: VecDeque::from([QueuedRecord {
                bytes,
                offset: 5,
                ack_marker: None,
                expires_at_ms: 100,
            }]),
            next_attempt_ms: 0,
        };

        queue.expire_unstarted(101);
        assert_eq!(queue.records.len(), 1);

        queue.rewind_after_stream_change();
        queue.expire_unstarted(101);
        assert!(queue.records.is_empty());
        assert_eq!(queue.bytes, 0);
    }
}
