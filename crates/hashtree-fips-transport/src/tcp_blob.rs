use super::{FipsEndpointIo, FipsTransportError};
use fips_tcp::wire::Segment;
use fips_tcp::{Config, ConnectionId, Outbound, Stack, State};
use hashtree_core::{Hash, Store};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio::time::{timeout, Duration, Instant};

const SERVICE_PORT: u16 = 39_018;
const MAGIC: u8 = 0x48;
const VERSION: u8 = 1;
const GET: u8 = 1;
const FOUND: u8 = 1;
const REQUEST_BYTES: usize = 35;
const HEADER_BYTES: usize = 7;
const MAX_BLOB_BYTES: usize = 16 * 1024 * 1024;

pub(super) struct TcpBlobTransport<S> {
    endpoint: Arc<dyn FipsEndpointIo>,
    store: Arc<S>,
    state: Mutex<TcpBlobState>,
    epoch: Instant,
    cache_responses: bool,
}

struct TcpBlobState {
    stack: Stack<String>,
    connections: HashMap<ConnectionId, Connection>,
}

enum Connection {
    Client {
        hash: Hash,
        request_sent: bool,
        buffer: Vec<u8>,
        expected: Option<usize>,
        result: mpsc::UnboundedSender<Option<Vec<u8>>>,
    },
    Server {
        buffer: Vec<u8>,
        response: Option<Vec<u8>>,
        written: usize,
        closing: bool,
    },
}

impl<S: Store + Send + Sync + 'static> TcpBlobTransport<S> {
    pub(super) fn new(endpoint: Arc<dyn FipsEndpointIo>, store: Arc<S>) -> Self {
        let config = Config {
            send_buffer: MAX_BLOB_BYTES + HEADER_BYTES,
            initial_rto_ms: 50,
            min_rto_ms: 20,
            max_rto_ms: 5_000,
            ..Config::default()
        };
        let mut stack = Stack::new(config, 1);
        stack
            .listen(SERVICE_PORT)
            .expect("non-zero TCP/FIPS blob service");
        Self {
            endpoint,
            store,
            state: Mutex::new(TcpBlobState {
                stack,
                connections: HashMap::new(),
            }),
            epoch: Instant::now(),
            cache_responses: true,
        }
    }

    pub(super) fn with_cache_responses(mut self, cache_responses: bool) -> Self {
        self.cache_responses = cache_responses;
        self
    }

    pub(super) async fn get(
        &self,
        hash: &Hash,
        peers: &[String],
        request_timeout: Duration,
    ) -> Result<Option<Vec<u8>>, FipsTransportError> {
        if let Some(data) = self.verified_get(hash).await? {
            return Ok(Some(data));
        }
        if peers.is_empty() {
            return Ok(None);
        }
        let attempt_timeout = (request_timeout / 2).max(Duration::from_millis(1));
        for _ in 0..2 {
            match self.get_once(hash, peers, attempt_timeout).await? {
                AttemptResult::Found(data) => return Ok(Some(data)),
                AttemptResult::Miss => return Ok(None),
                AttemptResult::Retry => {}
            }
        }
        Ok(None)
    }

    async fn get_once(
        &self,
        hash: &Hash,
        peers: &[String],
        attempt_timeout: Duration,
    ) -> Result<AttemptResult, FipsTransportError> {
        let (result, mut results) = mpsc::unbounded_channel();
        let now_ms = self.now_ms();
        let (ids, outbound) = {
            let mut state = self.state.lock().await;
            let mut ids = Vec::with_capacity(peers.len());
            for peer in peers {
                let id = state
                    .stack
                    .connect(peer.clone(), SERVICE_PORT, now_ms)
                    .map_err(tcp_error)?;
                state.connections.insert(
                    id,
                    Connection::Client {
                        hash: *hash,
                        request_sent: false,
                        buffer: Vec::new(),
                        expected: None,
                        result: result.clone(),
                    },
                );
                ids.push(id);
            }
            (ids, state.stack.drain_outbound())
        };
        let sent = self.send_outbound(outbound).await?;
        drop(result);

        let wait = async {
            let mut completed = 0;
            while let Some(candidate) = results.recv().await {
                completed += 1;
                if let Some(data) = candidate {
                    return AttemptResult::Found(data);
                }
            }
            if completed == peers.len() {
                AttemptResult::Miss
            } else {
                AttemptResult::Retry
            }
        };
        let outcome = if sent == 0 {
            AttemptResult::Retry
        } else {
            timeout(attempt_timeout, wait)
                .await
                .unwrap_or(AttemptResult::Retry)
        };
        let outbound = {
            let mut state = self.state.lock().await;
            for id in ids {
                let _ = state.stack.close(id, self.now_ms());
                state.connections.remove(&id);
            }
            state.stack.drain_outbound()
        };
        let _ = self.send_outbound(outbound).await?;
        Ok(outcome)
    }

    pub(super) fn handles(&self, data: &[u8]) -> bool {
        Segment::decode(data).is_ok_and(|segment| {
            segment.src_port == SERVICE_PORT || segment.dst_port == SERVICE_PORT
        })
    }

    pub(super) async fn input(&self, peer: String, data: &[u8]) -> Result<(), FipsTransportError> {
        let actions = {
            let mut state = self.state.lock().await;
            let now_ms = self.now_ms();
            state.stack.input(peer, data, now_ms).map_err(tcp_error)?;
            process_connections(&mut state, now_ms)?
        };
        self.finish_actions(actions).await
    }

    pub(super) async fn poll(&self) -> Result<(), FipsTransportError> {
        let actions = {
            let mut state = self.state.lock().await;
            let now_ms = self.now_ms();
            state.stack.poll(now_ms);
            process_connections(&mut state, now_ms)?
        };
        self.finish_actions(actions).await
    }

    async fn finish_actions(&self, actions: Actions) -> Result<(), FipsTransportError> {
        let _ = self.send_outbound(actions.outbound).await?;
        for (id, hash) in actions.server_requests {
            let data = self.verified_get(&hash).await?;
            let size = data.as_ref().map_or(0, Vec::len);
            let mut response = encode_response_header(data.is_some(), size)?.to_vec();
            if let Some(data) = data {
                response.extend_from_slice(&data);
            }
            let outbound = {
                let mut state = self.state.lock().await;
                if let Some(Connection::Server { response: slot, .. }) =
                    state.connections.get_mut(&id)
                {
                    *slot = Some(response);
                }
                process_connections(&mut state, self.now_ms())?.outbound
            };
            let _ = self.send_outbound(outbound).await?;
        }
        for ClientResult {
            id,
            hash,
            data,
            result,
        } in actions.client_results
        {
            let verified = match data {
                Some(data) if verify_hash(&data, &hash) => {
                    if self.cache_responses {
                        let _ = self.store.put(hash, data.clone()).await;
                    }
                    Some(data)
                }
                _ => None,
            };
            let _ = result.send(verified);
            let outbound = {
                let mut state = self.state.lock().await;
                let _ = state.stack.close(id, self.now_ms());
                state.connections.remove(&id);
                state.stack.drain_outbound()
            };
            let _ = self.send_outbound(outbound).await?;
        }
        Ok(())
    }

    async fn verified_get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, FipsTransportError> {
        Ok(self
            .store
            .get(hash)
            .await?
            .filter(|data| verify_hash(data, hash)))
    }

    async fn send_outbound(
        &self,
        outbound: Vec<Outbound<String>>,
    ) -> Result<usize, FipsTransportError> {
        let mut sent = 0;
        for segment in outbound {
            match self.endpoint.send(&segment.peer, segment.bytes).await {
                Ok(()) => sent += 1,
                Err(error) => {
                    tracing::debug!(peer = %segment.peer, %error, "TCP/FIPS segment send deferred to retry")
                }
            }
        }
        Ok(sent)
    }

    fn now_ms(&self) -> u64 {
        self.epoch.elapsed().as_millis() as u64
    }
}

#[derive(Default)]
struct Actions {
    outbound: Vec<Outbound<String>>,
    server_requests: Vec<(ConnectionId, Hash)>,
    client_results: Vec<ClientResult>,
}

struct ClientResult {
    id: ConnectionId,
    hash: Hash,
    data: Option<Vec<u8>>,
    result: mpsc::UnboundedSender<Option<Vec<u8>>>,
}

enum AttemptResult {
    Found(Vec<u8>),
    Miss,
    Retry,
}

fn process_connections(
    state: &mut TcpBlobState,
    now_ms: u64,
) -> Result<Actions, FipsTransportError> {
    while let Some(id) = state.stack.accept(SERVICE_PORT) {
        state.connections.insert(
            id,
            Connection::Server {
                buffer: Vec::new(),
                response: None,
                written: 0,
                closing: false,
            },
        );
    }
    let ids = state.connections.keys().copied().collect::<Vec<_>>();
    let mut actions = Actions::default();
    for id in ids {
        let tcp_state = state.stack.state(id);
        if tcp_state.is_none() {
            state.connections.remove(&id);
            continue;
        }
        if !matches!(tcp_state, Some(State::Established | State::CloseWait)) {
            continue;
        }
        if let Some(Connection::Client {
            hash, request_sent, ..
        }) = state.connections.get_mut(&id)
        {
            if !*request_sent {
                let request = encode_request(hash);
                let accepted = state.stack.write(id, &request, now_ms).map_err(tcp_error)?;
                if accepted != request.len() {
                    return Err(FipsTransportError::Wire(
                        "TCP/FIPS request send buffer full".into(),
                    ));
                }
                *request_sent = true;
            }
        }
        loop {
            let data = state
                .stack
                .read(id, u16::MAX as usize, now_ms)
                .map_err(tcp_error)?;
            if data.is_empty() {
                break;
            }
            match state.connections.get_mut(&id) {
                Some(Connection::Client { buffer, .. })
                | Some(Connection::Server { buffer, .. }) => buffer.extend_from_slice(&data),
                None => break,
            }
        }
        process_frame(state, id, &mut actions, now_ms)?;
    }
    actions.outbound.extend(state.stack.drain_outbound());
    Ok(actions)
}

fn process_frame(
    state: &mut TcpBlobState,
    id: ConnectionId,
    actions: &mut Actions,
    now_ms: u64,
) -> Result<(), FipsTransportError> {
    let Some(connection) = state.connections.get_mut(&id) else {
        return Ok(());
    };
    match connection {
        Connection::Client {
            hash,
            buffer,
            expected,
            result,
            ..
        } => {
            if expected.is_none() && buffer.len() >= HEADER_BYTES {
                if buffer[0..2] != [MAGIC, VERSION] {
                    return Err(FipsTransportError::Wire(
                        "invalid TCP/FIPS blob response".into(),
                    ));
                }
                let size = u32::from_be_bytes(buffer[3..7].try_into().expect("header")) as usize;
                if size > MAX_BLOB_BYTES {
                    return Err(FipsTransportError::Wire(
                        "TCP/FIPS blob exceeds size limit".into(),
                    ));
                }
                *expected = Some(if buffer[2] == FOUND {
                    HEADER_BYTES + size
                } else {
                    HEADER_BYTES
                });
            }
            if let Some(size) = *expected {
                if buffer.len() >= size {
                    actions.client_results.push(ClientResult {
                        id,
                        hash: *hash,
                        data: (buffer[2] == FOUND).then(|| buffer[HEADER_BYTES..size].to_vec()),
                        result: result.clone(),
                    });
                }
            }
        }
        Connection::Server {
            buffer,
            response,
            written,
            closing,
        } => {
            if response.is_none() && buffer.len() >= REQUEST_BYTES {
                if buffer[0..3] != [MAGIC, VERSION, GET] {
                    return Err(FipsTransportError::Wire(
                        "invalid TCP/FIPS blob request".into(),
                    ));
                }
                let hash: Hash = buffer[3..REQUEST_BYTES]
                    .try_into()
                    .map_err(|_| FipsTransportError::Wire("invalid blob hash".into()))?;
                actions.server_requests.push((id, hash));
                *response = Some(Vec::new());
            }
            if let Some(data) = response.as_ref().filter(|data| !data.is_empty()) {
                if *written < data.len() {
                    *written += state
                        .stack
                        .write(id, &data[*written..], now_ms)
                        .map_err(tcp_error)?;
                }
                if *written == data.len() && !*closing {
                    state.stack.close(id, now_ms).map_err(tcp_error)?;
                    *closing = true;
                }
            }
        }
    }
    Ok(())
}

fn verify_hash(data: &[u8], expected: &Hash) -> bool {
    Sha256::digest(data).as_slice() == expected.as_ref()
}

fn encode_request(hash: &Hash) -> [u8; REQUEST_BYTES] {
    let mut request = [0; REQUEST_BYTES];
    request[..3].copy_from_slice(&[MAGIC, VERSION, GET]);
    request[3..].copy_from_slice(hash.as_ref());
    request
}

fn encode_response_header(
    found: bool,
    size: usize,
) -> Result<[u8; HEADER_BYTES], FipsTransportError> {
    let size = u32::try_from(size)
        .ok()
        .filter(|size| *size as usize <= MAX_BLOB_BYTES)
        .ok_or_else(|| FipsTransportError::Wire("TCP/FIPS blob exceeds size limit".into()))?;
    let mut header = [MAGIC, VERSION, u8::from(found) * FOUND, 0, 0, 0, 0];
    header[3..].copy_from_slice(&size.to_be_bytes());
    Ok(header)
}

fn tcp_error(error: impl std::fmt::Display) -> FipsTransportError {
    FipsTransportError::Wire(format!("TCP/FIPS: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_typescript_blob_v1_vectors() {
        let hash: Hash = std::array::from_fn(|index| index as u8);
        assert_eq!(
            hex::encode(encode_request(&hash)),
            "480101000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
        );
        assert_eq!(
            hex::encode(encode_response_header(true, 3).unwrap()),
            "48010100000003"
        );
        assert_eq!(SERVICE_PORT, 39_018);
        assert_eq!(MAX_BLOB_BYTES, 16 * 1024 * 1024);
    }
}
