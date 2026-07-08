use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use hashtree_core::Cid;
use nostr_sdk::{Event, JsonUtil};
use serde::{Deserialize, Serialize};

use crate::paths::{config_path_in, key_path_in};
use crate::{
    AppConfig, AppKey, AppKeyRootRef, FsFipsBlockSync, PRIMARY_DRIVE_ID, ProfileState,
    drive_root_recipient_app_key_pubkeys,
};

pub const DIRECT_ROOT_APP_TOPIC: &str = "iris-drive/root-events/v1/direct";
pub const DIRECT_ROOT_MESH_STREAM_PREFIX: &str = "iris-drive/root-events/v1";

const DIRECT_ROOT_EVENT_CACHE_CAP: usize = 128;
const DIRECT_ROOT_REPUBLISH_INTERVAL_SECS: u64 = 5;
const DIRECT_ROOT_STATE_REQUEST_INTERVAL_SECS: u64 = 10;
const DIRECT_ROOT_METADATA_REPUBLISH_INTERVAL_SECS: u64 = 300;
const DIRECT_ROOT_DOWNLOAD_TIMEOUT_SECS: u64 = 8;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectRootFrame {
    pub key: String,
    pub event_id: String,
    pub event_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectRootHintFrame {
    pub key: String,
    pub event_id: String,
    pub hint: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirectRootStateRequestFrame {
    pub root_scope_id: String,
    pub request: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirectRootWireFrame {
    Full(DirectRootFrame),
    Hint(DirectRootHintFrame),
    Request(DirectRootStateRequestFrame),
}

#[derive(Debug, Clone)]
pub struct DirectRootEvent {
    pub key: String,
    pub event_id: String,
    pub event_json: String,
}

#[derive(Debug, Clone)]
struct DirectRootPublishEvent {
    event: DirectRootEvent,
    source: DirectRootPublishSource,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum DirectRootPublishSource {
    LocalCurrent,
    LocalHeartbeat,
    CachedRelay,
    StateRequestReply,
    CachedStateRequestReply,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum DirectRootHintApply {
    Applied,
    AlreadyCurrent,
    NotRootKey,
    SenderMismatch,
    NoAccount,
    UnknownDrive,
    UnauthorizedAppKey,
    Stale,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum DirectRootHintScope {
    Drive,
    Share { share_id: String },
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DirectRootKeyHint {
    pub scope: DirectRootHintScope,
    pub app_key_pubkey: String,
    pub drive_id: String,
    pub app_key_seq: u64,
    pub root_cid: String,
    pub recipient_count: usize,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DirectRootHintApplyReport {
    pub outcome: DirectRootHintApply,
    pub root_cid: Option<String>,
}

#[derive(Default)]
pub struct DirectRootExchange {
    cached_events: BTreeMap<String, DirectRootEvent>,
    published_keys: BTreeMap<String, Instant>,
    seen_keys: BTreeSet<String>,
    subscribed_streams: BTreeSet<String>,
    known_mesh_peers: BTreeSet<String>,
    known_publish_peers: BTreeSet<String>,
    next_mesh_publish_seq: u64,
    state_request_times: BTreeMap<String, Instant>,
}

impl DirectRootExchange {
    pub async fn announce_current_state(
        &mut self,
        config_dir: &Path,
        sync: &FsFipsBlockSync,
    ) -> Result<(), String> {
        let config = AppConfig::load_or_default(config_path_in(config_dir))
            .map_err(|error| format!("loading config: {error}"))?;
        sync.refresh_authorized_peers(&config).await;
        let Some(state) = config.profile.as_ref() else {
            return Ok(());
        };
        let root_scope_id = state.root_scope_id();
        self.subscribe_profile_stream(&root_scope_id, sync).await;
        let stream = direct_root_mesh_stream(&root_scope_id);
        let events = self.events_for_publish(
            build_current_direct_root_events(config_dir, &config, state)
                .map_err(|error| format!("{error:#}"))?,
        );
        let now = Instant::now();
        for publish_event in events {
            self.publish_event(sync, &stream, publish_event, None, now)
                .await?;
        }
        self.prune_published_keys();
        Ok(())
    }

    pub async fn announce_local_root_heartbeat(
        &mut self,
        config_dir: &Path,
        sync: &FsFipsBlockSync,
    ) -> Result<(), String> {
        let config = AppConfig::load_or_default(config_path_in(config_dir))
            .map_err(|error| format!("loading config: {error}"))?;
        sync.refresh_authorized_peers(&config).await;
        let Some(state) = config.profile.as_ref() else {
            return Ok(());
        };
        let root_scope_id = state.root_scope_id();
        self.subscribe_profile_stream(&root_scope_id, sync).await;
        let stream = direct_root_mesh_stream(&root_scope_id);
        let local_events = build_current_direct_root_events(config_dir, &config, state)
            .map_err(|error| format!("{error:#}"))?;
        let now = Instant::now();
        for publish_event in self
            .local_root_events_for_publish(local_events, DirectRootPublishSource::LocalHeartbeat)
        {
            self.publish_event(sync, &stream, publish_event, None, now)
                .await?;
        }
        self.prune_published_keys();
        Ok(())
    }

    async fn announce_state_request_reply(
        &mut self,
        config_dir: &Path,
        sync: &FsFipsBlockSync,
        config: &AppConfig,
        state: &ProfileState,
        reply_peer: &str,
    ) -> Result<(), String> {
        let root_scope_id = state.root_scope_id();
        self.subscribe_profile_stream(&root_scope_id, sync).await;
        let stream = direct_root_mesh_stream(&root_scope_id);
        let events = self.state_request_events_for_publish(
            build_current_direct_root_events(config_dir, config, state)
                .map_err(|error| format!("{error:#}"))?,
        );
        let now = Instant::now();
        for publish_event in events {
            self.publish_event(sync, &stream, publish_event, Some(reply_peer), now)
                .await?;
        }
        self.prune_published_keys();
        Ok(())
    }

    pub async fn request_current_state_from_peers(
        &mut self,
        config_dir: &Path,
        sync: &FsFipsBlockSync,
    ) -> Result<(), String> {
        let config = AppConfig::load_or_default(config_path_in(config_dir))
            .map_err(|error| format!("loading config: {error}"))?;
        let Some(state) = config.profile.as_ref() else {
            return Ok(());
        };
        let root_scope_id = state.root_scope_id();
        let now = Instant::now();
        if !self.should_publish_state_request(&root_scope_id, now) {
            tracing::debug!(root_scope_id, "native direct-root state request throttled");
            return Ok(());
        }
        self.subscribe_profile_stream(&root_scope_id, sync).await;
        let bytes = encode_direct_root_state_request_frame(&root_scope_id)
            .map_err(|error| format!("encoding direct-root state request: {error}"))?;
        match sync
            .broadcast_app_message(DIRECT_ROOT_APP_TOPIC, bytes.clone())
            .await
        {
            Ok(sent_peers) => tracing::debug!(
                root_scope_id,
                sent_peers,
                "requested current direct-root state over FIPS"
            ),
            Err(error) => tracing::warn!(
                root_scope_id,
                error = %error,
                "requesting current direct-root state over FIPS failed"
            ),
        }
        let stream = direct_root_mesh_stream(&root_scope_id);
        let seq = self.next_mesh_publish_seq();
        let publish = sync.publish_mesh_pubsub(stream.clone(), seq, bytes).await;
        tracing::debug!(
            root_scope_id,
            stream,
            seq,
            sent_peers = publish.sent_peers,
            "requested current direct-root state over FIPS mesh"
        );
        Ok(())
    }

    async fn publish_event(
        &mut self,
        sync: &FsFipsBlockSync,
        stream: &str,
        publish_event: DirectRootPublishEvent,
        target_peer: Option<&str>,
        now: Instant,
    ) -> Result<(), String> {
        let DirectRootPublishEvent { event, source } = publish_event;
        if !self.should_publish_candidate_key_for_target(&event.key, source, target_peer, now) {
            return Ok(());
        }
        let frame = DirectRootFrame {
            key: event.key.clone(),
            event_id: event.event_id.clone(),
            event_json: event.event_json.clone(),
        };
        let bytes =
            serde_json::to_vec(&frame).map_err(|error| format!("encoding direct root: {error}"))?;
        let hint_bytes = should_publish_direct_root_hint(&event.key, source)
            .then(|| encode_direct_root_hint_frame(&event.key, &event.event_id))
            .transpose()
            .map_err(|error| format!("encoding direct-root hint: {error}"))?;
        let attempts = direct_root_publish_attempts_for_source(&event.key, source);
        let publish_targeted_reply_over_mesh =
            should_publish_targeted_direct_root_reply_over_mesh(source);
        for attempt in 0..attempts {
            let publish_full_frame =
                should_publish_direct_root_full_frame(&event.key, source, attempt);
            if let Some(hint_bytes) = hint_bytes.as_ref() {
                if let Some(target_peer) = target_peer {
                    match sync
                        .send_app_message(target_peer, DIRECT_ROOT_APP_TOPIC, hint_bytes.clone())
                        .await
                    {
                        Ok(()) => tracing::debug!(
                            root_key = event.key.as_str(),
                            target_peer,
                            "sent targeted direct-root hint over FIPS"
                        ),
                        Err(error) => tracing::warn!(
                            root_key = event.key.as_str(),
                            target_peer,
                            error = %error,
                            "sending targeted direct-root hint over FIPS failed"
                        ),
                    }
                } else {
                    match sync
                        .broadcast_app_message(DIRECT_ROOT_APP_TOPIC, hint_bytes.clone())
                        .await
                    {
                        Ok(sent_peers) => tracing::debug!(
                            root_key = event.key.as_str(),
                            sent_peers,
                            "sent direct-root hint over FIPS"
                        ),
                        Err(error) => tracing::warn!(
                            root_key = event.key.as_str(),
                            error = %error,
                            "sending direct-root hint over FIPS failed"
                        ),
                    }
                }
            }
            if publish_full_frame {
                if let Some(target_peer) = target_peer {
                    match sync
                        .send_app_message(target_peer, DIRECT_ROOT_APP_TOPIC, bytes.clone())
                        .await
                    {
                        Ok(()) => tracing::debug!(
                            root_key = event.key.as_str(),
                            target_peer,
                            "sent targeted direct-root event over FIPS"
                        ),
                        Err(error) => tracing::warn!(
                            root_key = event.key.as_str(),
                            target_peer,
                            error = %error,
                            "sending targeted direct-root event over FIPS failed"
                        ),
                    }
                } else {
                    match sync
                        .broadcast_app_message(DIRECT_ROOT_APP_TOPIC, bytes.clone())
                        .await
                    {
                        Ok(sent_peers) => tracing::debug!(
                            root_key = event.key.as_str(),
                            sent_peers,
                            "sent direct-root event over FIPS"
                        ),
                        Err(error) => tracing::warn!(
                            root_key = event.key.as_str(),
                            error = %error,
                            "sending direct-root event over FIPS failed"
                        ),
                    }
                }
            }
            if target_peer.is_some() && !publish_targeted_reply_over_mesh {
                continue;
            }
            if publish_full_frame {
                let seq = self.next_mesh_publish_seq();
                let publish = sync
                    .publish_mesh_pubsub(stream.to_string(), seq, bytes.clone())
                    .await;
                tracing::debug!(
                    root_key = event.key.as_str(),
                    sent_peers = publish.sent_peers,
                    seq,
                    "published direct-root mesh event over FIPS"
                );
            }
            if let Some(hint_bytes) = hint_bytes.as_ref() {
                let seq = self.next_mesh_publish_seq();
                let publish = sync
                    .publish_mesh_pubsub(stream.to_string(), seq, hint_bytes.clone())
                    .await;
                tracing::debug!(
                    root_key = event.key.as_str(),
                    sent_peers = publish.sent_peers,
                    seq,
                    "published direct-root mesh hint over FIPS"
                );
            }
        }
        Ok(())
    }

    pub async fn handle_app_message(
        &mut self,
        config_dir: &Path,
        sync: &FsFipsBlockSync,
        message: &crate::FipsAppMessage,
    ) -> Result<bool, String> {
        if message.topic != DIRECT_ROOT_APP_TOPIC {
            return Ok(false);
        }
        match decode_direct_root_wire_frame(&message.data)
            .map_err(|error| format!("parsing direct-root frame: {error}"))?
        {
            DirectRootWireFrame::Full(frame) => self.apply_frame(config_dir, sync, frame).await,
            DirectRootWireFrame::Hint(frame) => {
                self.apply_hint_frame(config_dir, sync, frame, &message.peer_id)
                    .await
            }
            DirectRootWireFrame::Request(frame) => {
                self.handle_state_request_frame(config_dir, sync, frame, &message.peer_id)
                    .await?;
                Ok(false)
            }
        }
    }

    pub async fn drain_mesh_events(
        &mut self,
        config_dir: &Path,
        sync: &FsFipsBlockSync,
    ) -> Result<(), String> {
        let messages = sync.drain_mesh_pubsub_events().await;
        let received_messages = messages.len();
        let (messages, skipped_roots) = coalesce_direct_root_mesh_events(messages);
        if skipped_roots > 0 {
            tracing::debug!(
                received_messages,
                applied_messages = messages.len(),
                skipped_roots,
                "coalesced native direct-root FIPS mesh events"
            );
        }
        for message in messages {
            self.handle_mesh_event(config_dir, sync, message).await?;
        }
        Ok(())
    }

    pub async fn handle_mesh_event(
        &mut self,
        config_dir: &Path,
        sync: &FsFipsBlockSync,
        message: crate::FipsMeshPubsubEvent,
    ) -> Result<(), String> {
        if !message
            .stream_id
            .starts_with(DIRECT_ROOT_MESH_STREAM_PREFIX)
        {
            return Ok(());
        }
        match decode_direct_root_wire_frame(&message.payload)
            .map_err(|error| format!("parsing direct-root mesh frame: {error}"))?
        {
            DirectRootWireFrame::Full(frame) => {
                self.apply_frame(config_dir, sync, frame).await?;
            }
            DirectRootWireFrame::Hint(frame) => {
                self.apply_hint_frame(config_dir, sync, frame, &message.origin_peer_id)
                    .await?;
            }
            DirectRootWireFrame::Request(frame) => {
                self.handle_state_request_frame(config_dir, sync, frame, &message.origin_peer_id)
                    .await?;
            }
        }
        Ok(())
    }

    async fn handle_state_request_frame(
        &mut self,
        config_dir: &Path,
        sync: &FsFipsBlockSync,
        frame: DirectRootStateRequestFrame,
        reply_peer: &str,
    ) -> Result<(), String> {
        if !frame.request {
            return Ok(());
        }
        let config = AppConfig::load_or_default(config_path_in(config_dir))
            .map_err(|error| format!("loading config: {error}"))?;
        let Some(state) = config.profile.as_ref() else {
            return Ok(());
        };
        if state.root_scope_id() != frame.root_scope_id {
            return Ok(());
        }
        self.announce_state_request_reply(config_dir, sync, &config, state, reply_peer)
            .await
    }

    async fn apply_frame(
        &mut self,
        config_dir: &Path,
        sync: &FsFipsBlockSync,
        frame: DirectRootFrame,
    ) -> Result<bool, String> {
        if self.seen_keys.contains(&frame.key) {
            return Ok(true);
        }
        let event: Event = serde_json::from_str(&frame.event_json)
            .map_err(|error| format!("parsing direct-root event: {error}"))?;
        if event.id.to_hex() != frame.event_id {
            return Err("direct-root event id mismatch".to_owned());
        }
        let direct_event = direct_root_event(frame.key.clone(), &event);
        self.remember_seen_key(frame.key.clone());
        match apply_direct_root_event(config_dir, &event, Some(sync)).await {
            Ok(changed) => {
                if changed {
                    self.cache_event(direct_event);
                } else {
                    self.seen_keys.remove(&frame.key);
                }
                Ok(true)
            }
            Err(error) => {
                self.seen_keys.remove(&frame.key);
                Err(format!("{error:#}"))
            }
        }
    }

    async fn apply_hint_frame(
        &mut self,
        config_dir: &Path,
        sync: &FsFipsBlockSync,
        frame: DirectRootHintFrame,
        source_peer: &str,
    ) -> Result<bool, String> {
        if !frame.hint || !self.should_cache_event_as_latest(&frame.key) {
            return Ok(false);
        }
        let changed = apply_direct_root_hint(config_dir, sync, &frame.key, source_peer)
            .await
            .map_err(|error| format!("{error:#}"))?;
        Ok(changed)
    }

    async fn subscribe_profile_stream(&mut self, root_scope_id: &str, sync: &FsFipsBlockSync) {
        let stream = direct_root_mesh_stream(root_scope_id);
        let peers_changed = self.refresh_known_root_peers(sync).await;
        if self.subscribed_streams.insert(stream.clone()) || peers_changed {
            let stats = sync.subscribe_mesh_pubsub(stream.clone()).await;
            tracing::debug!(
                stream,
                selected_peers = stats.selected_peers,
                sent_peers = stats.sent_peers,
                "subscribed direct-root mesh stream over FIPS"
            );
        }
    }

    async fn refresh_known_root_peers(&mut self, sync: &FsFipsBlockSync) -> bool {
        self.refresh_known_root_peer_sets(
            sync.authorized_peer_ids().await,
            sync.mesh_peer_ids().await,
        )
    }

    fn refresh_known_root_peer_sets(
        &mut self,
        authorized_peers: impl IntoIterator<Item = String>,
        mesh_peers: impl IntoIterator<Item = String>,
    ) -> bool {
        let publish_peers = authorized_peers.into_iter().collect::<BTreeSet<_>>();
        let mut root_peers = publish_peers.clone();
        root_peers.extend(mesh_peers);

        let peers_changed = root_peers != self.known_mesh_peers;
        if peers_changed {
            self.known_mesh_peers = root_peers;
        }
        let has_new_publish_peer = publish_peers
            .iter()
            .any(|peer| !self.known_publish_peers.contains(peer));
        if has_new_publish_peer {
            self.published_keys.clear();
        }
        self.known_publish_peers = publish_peers;
        peers_changed
    }

    fn next_mesh_publish_seq(&mut self) -> u64 {
        self.next_mesh_publish_seq = self.next_mesh_publish_seq.saturating_add(1).max(1);
        self.next_mesh_publish_seq
    }

    #[cfg(test)]
    fn should_publish_key(&mut self, key: &str, now: Instant) -> bool {
        self.should_publish_candidate_key(key, DirectRootPublishSource::LocalCurrent, now)
    }

    #[cfg(test)]
    fn should_publish_candidate_key(
        &mut self,
        key: &str,
        source: DirectRootPublishSource,
        now: Instant,
    ) -> bool {
        self.should_publish_candidate_key_for_target(key, source, None, now)
    }

    fn should_publish_candidate_key_for_target(
        &mut self,
        key: &str,
        source: DirectRootPublishSource,
        target_peer: Option<&str>,
        now: Instant,
    ) -> bool {
        let throttle_key = direct_root_publish_throttle_key(key, source, target_peer);
        if self.published_keys.get(&throttle_key).is_some_and(|last| {
            now.duration_since(*last)
                < Duration::from_secs(direct_root_republish_interval_secs_for_source(key, source))
        }) {
            return false;
        }
        self.published_keys.insert(throttle_key, now);
        true
    }

    fn should_publish_state_request(&mut self, root_scope_id: &str, now: Instant) -> bool {
        if self
            .state_request_times
            .get(root_scope_id)
            .is_some_and(|last| {
                now.duration_since(*last)
                    < Duration::from_secs(DIRECT_ROOT_STATE_REQUEST_INTERVAL_SECS)
            })
        {
            return false;
        }
        self.state_request_times
            .insert(root_scope_id.to_string(), now);
        while self.state_request_times.len() > DIRECT_ROOT_EVENT_CACHE_CAP {
            let Some(oldest) = self.state_request_times.keys().next().cloned() else {
                break;
            };
            self.state_request_times.remove(&oldest);
        }
        true
    }

    fn remember_seen_key(&mut self, key: String) {
        self.seen_keys.insert(key);
        while self.seen_keys.len() > DIRECT_ROOT_EVENT_CACHE_CAP {
            let Some(oldest) = self.seen_keys.iter().next().cloned() else {
                break;
            };
            self.seen_keys.remove(&oldest);
        }
    }

    fn cache_event(&mut self, event: DirectRootEvent) {
        self.remember_seen_key(event.key.clone());
        if !self.should_cache_event_as_latest(&event.key) {
            return;
        }
        for key in self.superseded_cached_event_keys(&event.key) {
            self.cached_events.remove(&key);
            self.published_keys.remove(&key);
        }
        self.cached_events.insert(event.key.clone(), event);
        while self.cached_events.len() > DIRECT_ROOT_EVENT_CACHE_CAP {
            let Some(key) = self.cached_events.keys().next().cloned() else {
                break;
            };
            self.cached_events.remove(&key);
            self.published_keys.remove(&key);
        }
    }

    fn event_for_publish(&self, event: DirectRootEvent) -> DirectRootEvent {
        self.cached_events.get(&event.key).cloned().unwrap_or(event)
    }

    fn events_for_publish(
        &self,
        local_events: Vec<DirectRootEvent>,
    ) -> Vec<DirectRootPublishEvent> {
        let mut events = Vec::with_capacity(local_events.len() + self.cached_events.len());
        let mut keys = BTreeSet::new();
        let mut local_slots = BTreeMap::new();
        for event in local_events {
            let event = self.event_for_publish(event);
            keys.insert(event.key.clone());
            if let Some(slot) = direct_root_cache_slot(&event.key) {
                local_slots.insert(slot.family.clone(), slot);
            }
            events.push(DirectRootPublishEvent {
                event,
                source: DirectRootPublishSource::LocalCurrent,
            });
        }
        events.extend(
            self.cached_events
                .values()
                .filter(|event| {
                    if keys.contains(&event.key) {
                        return false;
                    }
                    let Some(slot) = direct_root_cache_slot(&event.key) else {
                        return true;
                    };
                    local_slots
                        .get(&slot.family)
                        .is_none_or(|local_slot| direct_root_slot_is_newer(&slot, local_slot))
                })
                .cloned()
                .map(|event| DirectRootPublishEvent {
                    event,
                    source: DirectRootPublishSource::CachedRelay,
                }),
        );
        events
    }

    fn local_root_events_for_publish(
        &self,
        local_events: Vec<DirectRootEvent>,
        source: DirectRootPublishSource,
    ) -> Vec<DirectRootPublishEvent> {
        local_events
            .into_iter()
            .filter(|event| direct_root_cache_slot(&event.key).is_some())
            .map(|event| DirectRootPublishEvent {
                event: self.event_for_publish(event),
                source,
            })
            .collect()
    }

    fn state_request_events_for_publish(
        &self,
        local_events: Vec<DirectRootEvent>,
    ) -> Vec<DirectRootPublishEvent> {
        let mut events = Vec::with_capacity(local_events.len() + self.cached_events.len());
        let mut keys = BTreeSet::new();
        let mut local_slots = BTreeMap::new();
        for event in local_events {
            let event = self.event_for_publish(event);
            keys.insert(event.key.clone());
            if let Some(slot) = direct_root_cache_slot(&event.key) {
                local_slots.insert(slot.family.clone(), slot);
                events.push(DirectRootPublishEvent {
                    event,
                    source: DirectRootPublishSource::StateRequestReply,
                });
            }
        }
        events.extend(
            self.cached_events
                .values()
                .filter(|event| {
                    if keys.contains(&event.key) {
                        return false;
                    }
                    let Some(slot) = direct_root_cache_slot(&event.key) else {
                        return false;
                    };
                    local_slots
                        .get(&slot.family)
                        .is_none_or(|local_slot| direct_root_slot_is_newer(&slot, local_slot))
                })
                .cloned()
                .map(|event| DirectRootPublishEvent {
                    event,
                    source: DirectRootPublishSource::CachedStateRequestReply,
                }),
        );
        events
    }

    fn should_cache_event_as_latest(&self, incoming_key: &str) -> bool {
        let Some(incoming) = direct_root_cache_slot(incoming_key) else {
            return should_cache_unsequenced_direct_root_key(incoming_key);
        };
        !self.cached_events.keys().any(|key| {
            direct_root_cache_slot(key).is_some_and(|cached| {
                cached.family == incoming.family
                    && direct_root_slot_is_strictly_newer(&cached, &incoming)
            })
        })
    }

    fn superseded_cached_event_keys(&self, incoming_key: &str) -> Vec<String> {
        let Some(incoming) = direct_root_cache_slot(incoming_key) else {
            return Vec::new();
        };
        self.cached_events
            .keys()
            .filter(|key| {
                key.as_str() != incoming_key
                    && direct_root_cache_slot(key).is_some_and(|cached| {
                        cached.family == incoming.family
                            && !direct_root_slot_is_strictly_newer(&cached, &incoming)
                    })
            })
            .cloned()
            .collect()
    }

    fn prune_published_keys(&mut self) {
        while self.published_keys.len() > DIRECT_ROOT_EVENT_CACHE_CAP {
            let Some(oldest) = self.published_keys.keys().next().cloned() else {
                break;
            };
            self.published_keys.remove(&oldest);
        }
    }
}

pub fn coalesce_direct_root_app_messages(
    messages: Vec<crate::FipsAppMessage>,
) -> (Vec<crate::FipsAppMessage>, usize) {
    let mut passthrough = Vec::new();
    let mut latest_roots =
        BTreeMap::<String, (DirectRootCacheSlot, bool, crate::FipsAppMessage)>::new();
    let mut unsequenced_indices = BTreeMap::<String, usize>::new();
    let mut skipped = 0usize;

    for message in messages {
        let Some(frame) = direct_root_message_batch_frame(&message) else {
            passthrough.push(message);
            continue;
        };
        let Some(slot) = direct_root_cache_slot(&frame.key) else {
            if let Some(cache_key) = direct_root_unsequenced_batch_key(&frame.key) {
                if let Some(index) = unsequenced_indices.get(&cache_key).copied() {
                    passthrough[index] = message;
                    skipped = skipped.saturating_add(1);
                } else {
                    unsequenced_indices.insert(cache_key, passthrough.len());
                    passthrough.push(message);
                }
            } else {
                passthrough.push(message);
            }
            continue;
        };
        match latest_roots.entry(slot.family.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert((slot, frame.hint, message));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let (current_slot, current_is_hint, _) = entry.get();
                if direct_root_should_replace_batched_frame(
                    &slot,
                    frame.hint,
                    current_slot,
                    *current_is_hint,
                ) {
                    skipped = skipped.saturating_add(1);
                    entry.insert((slot, frame.hint, message));
                } else {
                    skipped = skipped.saturating_add(1);
                }
            }
        }
    }

    let mut coalesced = latest_roots
        .into_values()
        .map(|(_, _, message)| message)
        .collect::<Vec<_>>();
    coalesced.extend(passthrough);
    (coalesced, skipped)
}

pub fn coalesce_direct_root_mesh_events(
    messages: Vec<crate::FipsMeshPubsubEvent>,
) -> (Vec<crate::FipsMeshPubsubEvent>, usize) {
    let mut passthrough = Vec::new();
    let mut latest_roots =
        BTreeMap::<String, (DirectRootCacheSlot, bool, crate::FipsMeshPubsubEvent)>::new();
    let mut unsequenced_indices = BTreeMap::<String, usize>::new();
    let mut skipped = 0usize;

    for message in messages {
        let Some(frame) = direct_root_mesh_event_batch_frame(&message) else {
            passthrough.push(message);
            continue;
        };
        let Some(slot) = direct_root_cache_slot(&frame.key) else {
            if let Some(cache_key) = direct_root_unsequenced_batch_key(&frame.key) {
                if let Some(index) = unsequenced_indices.get(&cache_key).copied() {
                    passthrough[index] = message;
                    skipped = skipped.saturating_add(1);
                } else {
                    unsequenced_indices.insert(cache_key, passthrough.len());
                    passthrough.push(message);
                }
            } else {
                passthrough.push(message);
            }
            continue;
        };
        match latest_roots.entry(slot.family.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert((slot, frame.hint, message));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let (current_slot, current_is_hint, _) = entry.get();
                if direct_root_should_replace_batched_frame(
                    &slot,
                    frame.hint,
                    current_slot,
                    *current_is_hint,
                ) {
                    skipped = skipped.saturating_add(1);
                    entry.insert((slot, frame.hint, message));
                } else {
                    skipped = skipped.saturating_add(1);
                }
            }
        }
    }

    let mut coalesced = latest_roots
        .into_values()
        .map(|(_, _, message)| message)
        .collect::<Vec<_>>();
    coalesced.extend(passthrough);
    (coalesced, skipped)
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct DirectRootBatchFrame {
    key: String,
    hint: bool,
}

fn direct_root_message_batch_frame(
    message: &crate::FipsAppMessage,
) -> Option<DirectRootBatchFrame> {
    if message.topic != DIRECT_ROOT_APP_TOPIC {
        return None;
    }
    direct_root_batch_frame(&message.data)
}

fn direct_root_mesh_event_batch_frame(
    message: &crate::FipsMeshPubsubEvent,
) -> Option<DirectRootBatchFrame> {
    if !message
        .stream_id
        .starts_with(DIRECT_ROOT_MESH_STREAM_PREFIX)
    {
        return None;
    }
    direct_root_batch_frame(&message.payload)
}

fn direct_root_batch_frame(data: &[u8]) -> Option<DirectRootBatchFrame> {
    match decode_direct_root_wire_frame(data).ok()? {
        DirectRootWireFrame::Full(frame) => Some(DirectRootBatchFrame {
            key: frame.key,
            hint: false,
        }),
        DirectRootWireFrame::Hint(frame) => Some(DirectRootBatchFrame {
            key: frame.key,
            hint: true,
        }),
        DirectRootWireFrame::Request(_) => None,
    }
}

fn direct_root_should_replace_batched_frame(
    candidate: &DirectRootCacheSlot,
    candidate_is_hint: bool,
    current: &DirectRootCacheSlot,
    current_is_hint: bool,
) -> bool {
    direct_root_slot_is_strictly_newer(candidate, current)
        || (candidate.family == current.family
            && candidate.seq == current.seq
            && candidate.recipient_count == current.recipient_count
            && current_is_hint
            && !candidate_is_hint)
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct DirectRootCacheSlot {
    family: String,
    seq: u64,
    recipient_count: usize,
}

fn direct_root_cache_slot(key: &str) -> Option<DirectRootCacheSlot> {
    let (prefix, rest) = key.split_once(':')?;
    match prefix {
        "drive-root" => {
            let mut parts = rest.splitn(4, ':');
            let app_key = parts.next()?;
            let drive_id = parts.next()?;
            let seq = parts.next()?.parse().ok()?;
            let root_and_recipients = parts.next()?;
            let (_root_cid, recipients) = root_and_recipients.rsplit_once(':')?;
            Some(DirectRootCacheSlot {
                family: format!("drive-root:{app_key}:{drive_id}"),
                seq,
                recipient_count: direct_root_recipient_count(recipients),
            })
        }
        "share-root" => {
            let mut parts = rest.splitn(4, ':');
            let share_id = parts.next()?;
            let app_key = parts.next()?;
            let seq = parts.next()?.parse().ok()?;
            let root_and_recipients = parts.next()?;
            let (_root_cid, recipients) = root_and_recipients.rsplit_once(':')?;
            Some(DirectRootCacheSlot {
                family: format!("share-root:{share_id}:{app_key}"),
                seq,
                recipient_count: direct_root_recipient_count(recipients),
            })
        }
        _ => None,
    }
}

fn direct_root_recipient_count(recipients: &str) -> usize {
    recipients
        .split(',')
        .filter(|recipient| !recipient.is_empty())
        .count()
}

fn direct_root_slot_is_newer(
    candidate: &DirectRootCacheSlot,
    current: &DirectRootCacheSlot,
) -> bool {
    candidate.seq > current.seq
        || (candidate.seq == current.seq && candidate.recipient_count > current.recipient_count)
}

fn direct_root_slot_is_strictly_newer(
    candidate: &DirectRootCacheSlot,
    current: &DirectRootCacheSlot,
) -> bool {
    direct_root_slot_is_newer(candidate, current)
}

fn should_cache_unsequenced_direct_root_key(key: &str) -> bool {
    !key.starts_with("drive-root:")
        && !key.starts_with("share-root:")
        && !key.starts_with("files-root:")
}

fn direct_root_unsequenced_batch_key(key: &str) -> Option<String> {
    if should_cache_unsequenced_direct_root_key(key) || key.starts_with("files-root:") {
        return Some(key.to_string());
    }
    None
}

fn direct_root_republish_interval_secs_for_source(
    key: &str,
    source: DirectRootPublishSource,
) -> u64 {
    if matches!(
        source,
        DirectRootPublishSource::StateRequestReply
            | DirectRootPublishSource::CachedStateRequestReply
    ) && direct_root_cache_slot(key).is_some()
    {
        return 2;
    }
    if source == DirectRootPublishSource::CachedRelay && direct_root_cache_slot(key).is_some() {
        return DIRECT_ROOT_REPUBLISH_INTERVAL_SECS;
    }
    if direct_root_cache_slot(key).is_some() {
        DIRECT_ROOT_REPUBLISH_INTERVAL_SECS
    } else {
        DIRECT_ROOT_METADATA_REPUBLISH_INTERVAL_SECS
    }
}

fn direct_root_publish_attempts_for_source(key: &str, source: DirectRootPublishSource) -> usize {
    if matches!(
        source,
        DirectRootPublishSource::LocalHeartbeat | DirectRootPublishSource::StateRequestReply
    ) && direct_root_cache_slot(key).is_some()
    {
        return 4;
    }
    if matches!(
        source,
        DirectRootPublishSource::LocalHeartbeat | DirectRootPublishSource::StateRequestReply
    ) {
        return 1;
    }
    if source == DirectRootPublishSource::CachedStateRequestReply
        && direct_root_cache_slot(key).is_some()
    {
        return 4;
    }
    if source == DirectRootPublishSource::CachedRelay && direct_root_cache_slot(key).is_some() {
        return 2;
    }
    if direct_root_cache_slot(key).is_some() {
        4
    } else if key.starts_with("files-root:") {
        2
    } else {
        1
    }
}

fn direct_root_publish_throttle_key(
    key: &str,
    source: DirectRootPublishSource,
    target_peer: Option<&str>,
) -> String {
    if source == DirectRootPublishSource::StateRequestReply && direct_root_cache_slot(key).is_some()
    {
        return target_peer.map_or_else(
            || format!("state-request:{key}"),
            |peer| format!("state-request:{peer}:{key}"),
        );
    }
    if source == DirectRootPublishSource::CachedStateRequestReply
        && direct_root_cache_slot(key).is_some()
    {
        return target_peer.map_or_else(
            || format!("state-request-cached:{key}"),
            |peer| format!("state-request-cached:{peer}:{key}"),
        );
    }
    if source == DirectRootPublishSource::CachedRelay && direct_root_cache_slot(key).is_some() {
        return format!("cached-relay:{key}");
    }
    key.to_string()
}

fn should_publish_direct_root_hint(key: &str, source: DirectRootPublishSource) -> bool {
    matches!(
        source,
        DirectRootPublishSource::LocalCurrent
            | DirectRootPublishSource::LocalHeartbeat
            | DirectRootPublishSource::StateRequestReply
    ) && direct_root_cache_slot(key).is_some()
}

fn should_publish_direct_root_full_frame(
    key: &str,
    source: DirectRootPublishSource,
    attempt: usize,
) -> bool {
    if should_publish_direct_root_hint(key, source) {
        return match source {
            DirectRootPublishSource::LocalHeartbeat => false,
            DirectRootPublishSource::LocalCurrent | DirectRootPublishSource::StateRequestReply => {
                attempt == 0
            }
            DirectRootPublishSource::CachedRelay
            | DirectRootPublishSource::CachedStateRequestReply => true,
        };
    }
    true
}

fn should_publish_targeted_direct_root_reply_over_mesh(source: DirectRootPublishSource) -> bool {
    matches!(
        source,
        DirectRootPublishSource::StateRequestReply
            | DirectRootPublishSource::CachedStateRequestReply
    )
}

pub fn encode_direct_root_hint_frame(
    key: &str,
    event_id: &str,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&DirectRootHintFrame {
        key: key.to_string(),
        event_id: event_id.to_string(),
        hint: true,
    })
}

pub fn encode_direct_root_state_request_frame(
    root_scope_id: &str,
) -> Result<Vec<u8>, serde_json::Error> {
    serde_json::to_vec(&DirectRootStateRequestFrame {
        root_scope_id: root_scope_id.to_string(),
        request: true,
    })
}

pub fn decode_direct_root_wire_frame(
    data: &[u8],
) -> Result<DirectRootWireFrame, serde_json::Error> {
    match serde_json::from_slice::<DirectRootFrame>(data) {
        Ok(frame) => Ok(DirectRootWireFrame::Full(frame)),
        Err(full_error) => match serde_json::from_slice::<DirectRootHintFrame>(data) {
            Ok(frame) if frame.hint => Ok(DirectRootWireFrame::Hint(frame)),
            _ => match serde_json::from_slice::<DirectRootStateRequestFrame>(data) {
                Ok(frame) if frame.request => Ok(DirectRootWireFrame::Request(frame)),
                _ => Err(full_error),
            },
        },
    }
}

pub fn build_current_direct_root_events(
    config_dir: &Path,
    config: &AppConfig,
    state: &ProfileState,
) -> Result<Vec<DirectRootEvent>> {
    let mut events = Vec::new();
    if state.can_write_roots()
        && let Some(drive) = config.drive(PRIMARY_DRIVE_ID)
        && let Some(root) = drive.app_key_roots.get(&state.app_key_pubkey)
    {
        let device = AppKey::load(key_path_in(config_dir)).context("loading app key")?;
        let authorized_app_keys = drive_root_recipient_app_key_pubkeys(state, drive);
        if !authorized_app_keys.is_empty() {
            let event = crate::nostr_events::build_drive_root_event(
                device.keys(),
                &state.root_scope_id(),
                &drive.drive_id,
                root,
                &authorized_app_keys,
            )
            .context("building drive-root event")?;
            events.push(direct_root_event(
                format!(
                    "drive-root:{}:{}:{}:{}:{}",
                    state.app_key_pubkey,
                    drive.drive_id,
                    root.app_key_seq,
                    root.root_cid,
                    authorized_app_keys.join(",")
                ),
                &event,
            ));
        }
    }
    for op in &state.profile_roster_ops {
        let event =
            Event::from_json(&op.event_json).context("parsing NostrIdentity roster op event")?;
        events.push(direct_root_event(
            format!("profile-op:{}:{}", state.profile_id, op.op_id),
            &event,
        ));
    }
    Ok(events)
}

fn direct_root_event(key: String, event: &Event) -> DirectRootEvent {
    DirectRootEvent {
        key,
        event_id: event.id.to_hex(),
        event_json: event.as_json(),
    }
}

#[must_use]
pub fn direct_root_mesh_stream(root_scope_id: &str) -> String {
    format!("{DIRECT_ROOT_MESH_STREAM_PREFIX}/{root_scope_id}")
}

pub async fn apply_direct_root_event(
    config_dir: &Path,
    event: &Event,
    sync: Option<&FsFipsBlockSync>,
) -> Result<bool> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    if crate::is_nostr_identity_roster_op_event_coordinate(event) {
        let outcome =
            crate::relay_sync::apply_remote_nostr_identity_roster_op_event(&mut config, event)?;
        let changed = matches!(
            outcome,
            crate::relay_sync::NostrIdentityRosterOpApply::Applied
        );
        config.save(config_path_in(config_dir))?;
        if let Some(sync) = sync {
            sync.refresh_authorized_peers(&config).await;
        }
        return Ok(changed);
    }
    if crate::is_share_access_snapshot_event_coordinate(event) {
        let outcome =
            crate::relay_sync::apply_remote_share_access_snapshot_event(&mut config, event)?;
        let changed = matches!(
            outcome,
            crate::relay_sync::ShareAccessSnapshotApply::Applied
        );
        config.save(config_path_in(config_dir))?;
        if let Some(sync) = sync {
            sync.refresh_authorized_peers(&config).await;
        }
        return Ok(changed);
    }
    if crate::nostr_events::is_drive_root_event_coordinate(event) {
        let device = AppKey::load(key_path_in(config_dir)).context("loading app key")?;
        let parsed =
            crate::nostr_events::parse_drive_root_event_for_device(event, device.keys()).ok();
        let outcome = crate::relay_sync::apply_remote_drive_root_event(
            &mut config,
            event,
            Some(device.keys()),
        )?;
        let should_download = matches!(
            outcome,
            crate::relay_sync::DriveRootApply::Applied
                | crate::relay_sync::DriveRootApply::StaleTimestamp
        );
        let root_cid = parsed
            .as_ref()
            .filter(|_| should_download)
            .map(|(_, _, _, root_ref)| root_ref.root_cid.clone());
        let materialize_after_download = root_cid
            .as_deref()
            .is_some_and(|root_cid| root_cid_belongs_to_peer(&config, root_cid));
        let changed = matches!(outcome, crate::relay_sync::DriveRootApply::Applied);
        config.save(config_path_in(config_dir))?;
        if let Some(sync) = sync {
            sync.refresh_authorized_peers(&config).await;
            if let Some(root_cid) = root_cid {
                download_direct_root(sync, &root_cid).await?;
            }
        }
        let materialized = if sync.is_some() && materialize_after_download {
            let mut daemon =
                crate::Daemon::open(config_dir).context("opening daemon to materialize merge")?;
            daemon
                .materialize_primary_merged_root()
                .await
                .context("materializing merged root")?
                .is_some()
        } else {
            false
        };
        return Ok(changed || materialized);
    }
    Ok(false)
}

pub async fn apply_direct_root_hint(
    config_dir: &Path,
    sync: &FsFipsBlockSync,
    key: &str,
    source_peer: &str,
) -> Result<bool> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let report = apply_direct_root_key_hint_to_config(
        &mut config,
        key,
        source_peer,
        current_unix_seconds(),
    )?;
    let changed = matches!(report.outcome, DirectRootHintApply::Applied);
    if changed {
        config.save(config_path_in(config_dir))?;
        sync.refresh_authorized_peers(&config).await;
    }
    let should_pull = matches!(
        report.outcome,
        DirectRootHintApply::Applied | DirectRootHintApply::AlreadyCurrent
    );
    let materialized = if let Some(root_cid) = report.root_cid.filter(|_| should_pull) {
        download_direct_root(sync, &root_cid).await?;
        if root_cid_belongs_to_peer(&config, &root_cid) {
            let mut daemon =
                crate::Daemon::open(config_dir).context("opening daemon to materialize merge")?;
            daemon
                .materialize_primary_merged_root()
                .await
                .context("materializing merged root")?
                .is_some()
        } else {
            false
        }
    } else {
        false
    };
    Ok(changed || materialized)
}

pub fn apply_direct_root_key_hint_to_config(
    config: &mut AppConfig,
    key: &str,
    source_peer: &str,
    published_at: i64,
) -> Result<DirectRootHintApplyReport> {
    let Some(hint) = parse_direct_root_key_hint(key) else {
        return Ok(direct_root_hint_report(
            DirectRootHintApply::NotRootKey,
            None,
        ));
    };
    Cid::parse(&hint.root_cid)
        .with_context(|| format!("parsing direct-root hint root {}", hint.root_cid))?;
    if !direct_root_hint_sender_matches(&hint.app_key_pubkey, source_peer) {
        return Ok(direct_root_hint_report(
            DirectRootHintApply::SenderMismatch,
            Some(hint.root_cid),
        ));
    }
    let Some(account) = config.profile.as_ref() else {
        return Ok(direct_root_hint_report(
            DirectRootHintApply::NoAccount,
            Some(hint.root_cid),
        ));
    };

    match &hint.scope {
        DirectRootHintScope::Drive => {
            let Some(drive_index) = config
                .drives
                .iter()
                .position(|drive| drive.drive_id == hint.drive_id)
            else {
                return Ok(direct_root_hint_report(
                    DirectRootHintApply::UnknownDrive,
                    Some(hint.root_cid),
                ));
            };
            if !crate::drive_root_app_key_can_write_roots(
                account,
                &config.drives[drive_index],
                &hint.app_key_pubkey,
            ) {
                return Ok(direct_root_hint_report(
                    DirectRootHintApply::UnauthorizedAppKey,
                    Some(hint.root_cid),
                ));
            }
            Ok(apply_direct_root_hint_to_roots(
                &mut config.drives[drive_index].app_key_roots,
                &hint,
                published_at,
            ))
        }
        DirectRootHintScope::Share { share_id } => {
            if hint.drive_id != PRIMARY_DRIVE_ID {
                return Ok(direct_root_hint_report(
                    DirectRootHintApply::UnknownDrive,
                    Some(hint.root_cid),
                ));
            }
            let Ok(share_id) = share_id.parse::<crate::NostrIdentityId>() else {
                return Ok(direct_root_hint_report(
                    DirectRootHintApply::UnknownDrive,
                    Some(hint.root_cid),
                ));
            };
            let Some(folder_index) = config
                .shared_folders
                .iter()
                .position(|folder| folder.share_id == share_id)
            else {
                return Ok(direct_root_hint_report(
                    DirectRootHintApply::UnknownDrive,
                    Some(hint.root_cid),
                ));
            };
            if !crate::shared_folder_app_key_can_write_roots(
                &config.shared_folders[folder_index],
                &hint.app_key_pubkey,
            ) {
                return Ok(direct_root_hint_report(
                    DirectRootHintApply::UnauthorizedAppKey,
                    Some(hint.root_cid),
                ));
            }
            Ok(apply_direct_root_hint_to_roots(
                &mut config.shared_folders[folder_index].app_key_roots,
                &hint,
                published_at,
            ))
        }
    }
}

pub fn parse_direct_root_key_hint(key: &str) -> Option<DirectRootKeyHint> {
    let (prefix, rest) = key.split_once(':')?;
    match prefix {
        "drive-root" => {
            let mut parts = rest.splitn(4, ':');
            let app_key_pubkey = parts.next()?.to_ascii_lowercase();
            let drive_id = parts.next()?.to_string();
            let app_key_seq = parts.next()?.parse().ok()?;
            let root_and_recipients = parts.next()?;
            let (root_cid, recipients) = root_and_recipients.rsplit_once(':')?;
            Some(DirectRootKeyHint {
                scope: DirectRootHintScope::Drive,
                app_key_pubkey,
                drive_id,
                app_key_seq,
                root_cid: root_cid.to_string(),
                recipient_count: direct_root_recipient_count(recipients),
            })
        }
        "share-root" => {
            let mut parts = rest.splitn(4, ':');
            let share_id = parts.next()?.to_string();
            let app_key_pubkey = parts.next()?.to_ascii_lowercase();
            let app_key_seq = parts.next()?.parse().ok()?;
            let root_and_recipients = parts.next()?;
            let (root_cid, recipients) = root_and_recipients.rsplit_once(':')?;
            Some(DirectRootKeyHint {
                scope: DirectRootHintScope::Share { share_id },
                app_key_pubkey,
                drive_id: PRIMARY_DRIVE_ID.to_string(),
                app_key_seq,
                root_cid: root_cid.to_string(),
                recipient_count: direct_root_recipient_count(recipients),
            })
        }
        _ => None,
    }
}

fn apply_direct_root_hint_to_roots(
    app_key_roots: &mut BTreeMap<String, AppKeyRootRef>,
    hint: &DirectRootKeyHint,
    published_at: i64,
) -> DirectRootHintApplyReport {
    if let Some(existing) = app_key_roots.get(&hint.app_key_pubkey) {
        if existing.root_cid == hint.root_cid {
            return direct_root_hint_report(
                DirectRootHintApply::AlreadyCurrent,
                Some(hint.root_cid.clone()),
            );
        }
        if existing.app_key_seq > 0 || hint.app_key_seq > 0 {
            if existing.app_key_seq >= hint.app_key_seq {
                return direct_root_hint_report(
                    DirectRootHintApply::Stale,
                    Some(hint.root_cid.clone()),
                );
            }
        } else if existing.published_at >= published_at {
            return direct_root_hint_report(
                DirectRootHintApply::Stale,
                Some(hint.root_cid.clone()),
            );
        }
    }

    app_key_roots.insert(
        hint.app_key_pubkey.clone(),
        AppKeyRootRef {
            root_cid: hint.root_cid.clone(),
            published_at,
            dck_generation: 0,
            app_key_seq: hint.app_key_seq,
            parents: Vec::new(),
            observed: BTreeMap::new(),
            local_only: false,
        },
    );
    direct_root_hint_report(DirectRootHintApply::Applied, Some(hint.root_cid.clone()))
}

fn direct_root_hint_report(
    outcome: DirectRootHintApply,
    root_cid: Option<String>,
) -> DirectRootHintApplyReport {
    DirectRootHintApplyReport { outcome, root_cid }
}

fn direct_root_hint_sender_matches(app_key_pubkey: &str, source_peer: &str) -> bool {
    let Ok(app_key_hex) = crate::normalize_app_key_pubkey(app_key_pubkey) else {
        return false;
    };
    if let Ok(source_hex) = crate::normalize_app_key_pubkey(source_peer) {
        return source_hex == app_key_hex;
    }
    crate::app_key_summary::pubkey_npub(&app_key_hex) == source_peer
}

fn current_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_secs().try_into().unwrap_or(i64::MAX)
        })
}

fn root_cid_belongs_to_peer(config: &AppConfig, root_cid: &str) -> bool {
    let Some(account) = config.profile.as_ref() else {
        return false;
    };
    config.drive(PRIMARY_DRIVE_ID).is_some_and(|drive| {
        drive
            .app_key_roots
            .iter()
            .any(|(device, root)| device != &account.app_key_pubkey && root.root_cid == root_cid)
    })
}

async fn download_direct_root(sync: &FsFipsBlockSync, root_cid: &str) -> Result<()> {
    let cid = Cid::parse(root_cid).with_context(|| format!("parsing root cid {root_cid}"))?;
    tokio::time::timeout(
        Duration::from_secs(DIRECT_ROOT_DOWNLOAD_TIMEOUT_SECS),
        sync.download_tree(&cid),
    )
    .await
    .context("direct-root FIPS download timed out")?
    .with_context(|| format!("downloading direct root {root_cid} over FIPS"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppKeyRootRef, Drive, Profile};

    #[test]
    fn direct_root_events_fall_back_to_known_drive_roots_without_roster_cache() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Profile::create(dir.path(), Some("Windows".to_string())).unwrap();
        let mut state = owner.state.clone();
        state.profile_roster_ops.clear();
        state.app_keys = None;

        let mut drive = Drive::primary(state.root_scope_id());
        let other_app_key = "08de265ef1945219e2a23a9243f63b2698aa71e8cf0a72ecc6456030363ce282";
        let third_app_key = "e543d0df262839a24d6ca2b400ce50a285adb3a89219d3d702ff4c23e05a1dc4";
        drive.app_key_roots.insert(
            other_app_key.to_string(),
            AppKeyRootRef::legacy(
                "082c410c74dd61929c37875ca2d8f79f31e1d64b0aaeabf66672cf933cf37922:1e7814a66a87918eced61de58be46e8bc8e9790fbb554f62b096f82832aeed04",
                10,
                1,
            ),
        );
        drive.app_key_roots.insert(
            state.app_key_pubkey.clone(),
            AppKeyRootRef::legacy(
                "63c49f803c85645412e6fd431633b21f8ede15dd39150e709a087bc1b7cd960c:3e9a2656d97761cc705a80c405827bbd204e34fb83cb80ef59bcbad30bfec5e4",
                11,
                2,
            ),
        );
        drive.app_key_roots.insert(
            third_app_key.to_string(),
            AppKeyRootRef::legacy(
                "a04b615515eaa15ab861094d1d5cdfaf1e90b525017d6d9181b916a3d6327fad:20cd54a52f030a27ae1f3e1d730c281b13d89093c8959585b6ba78b19647b51b",
                12,
                1,
            ),
        );
        let config = AppConfig {
            profile: Some(state.clone()),
            drives: vec![drive],
            ..AppConfig::default()
        };

        let events = build_current_direct_root_events(dir.path(), &config, &state).unwrap();
        let drive_root = events
            .iter()
            .find(|event| event.key.starts_with("drive-root:"))
            .expect("drive root event");

        let mut expected = [
            other_app_key.to_string(),
            state.app_key_pubkey.clone(),
            third_app_key.to_string(),
        ];
        expected.sort();

        assert!(
            drive_root
                .key
                .ends_with(&format!(":{}", expected.join(",")))
        );
    }

    #[test]
    fn current_direct_root_events_prioritize_roots_before_profile_metadata() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Profile::create(dir.path(), Some("Windows".to_string())).unwrap();
        let state = owner.state.clone();
        let mut drive = Drive::primary(state.root_scope_id());
        drive.app_key_roots.insert(
            state.app_key_pubkey.clone(),
            AppKeyRootRef::legacy(
                "63c49f803c85645412e6fd431633b21f8ede15dd39150e709a087bc1b7cd960c:3e9a2656d97761cc705a80c405827bbd204e34fb83cb80ef59bcbad30bfec5e4",
                11,
                2,
            ),
        );
        let config = AppConfig {
            profile: Some(state.clone()),
            drives: vec![drive],
            ..AppConfig::default()
        };

        let events = build_current_direct_root_events(dir.path(), &config, &state).unwrap();
        let root_index = events
            .iter()
            .position(|event| event.key.starts_with("drive-root:"))
            .expect("drive root event");
        let profile_index = events
            .iter()
            .position(|event| event.key.starts_with("profile-op:"))
            .expect("profile metadata event");

        assert!(
            root_index < profile_index,
            "drive roots should be published before profile metadata"
        );
    }

    #[test]
    fn direct_root_republish_keeps_latest_sequence_per_root_family() {
        let mut exchange = DirectRootExchange::default();
        let older = DirectRootEvent {
            key: "drive-root:remote:main:7:old-hash:old-key:local,remote".to_string(),
            event_id: "old-event".to_string(),
            event_json: "{\"id\":\"old\"}".to_string(),
        };
        let newer = DirectRootEvent {
            key: "drive-root:remote:main:8:new-hash:new-key:local,remote".to_string(),
            event_id: "new-event".to_string(),
            event_json: "{\"id\":\"new\"}".to_string(),
        };

        exchange.cache_event(older.clone());
        exchange.cache_event(newer.clone());
        exchange.cache_event(older);
        let events = exchange.events_for_publish(Vec::new());

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.event_id, newer.event_id);
    }

    #[test]
    fn direct_root_republish_filters_cached_roots_superseded_by_local_root() {
        let mut exchange = DirectRootExchange::default();
        let older = DirectRootEvent {
            key: "drive-root:local:main:7:old-hash:old-key:local,remote".to_string(),
            event_id: "old-event".to_string(),
            event_json: "{\"id\":\"old\"}".to_string(),
        };
        let newer = DirectRootEvent {
            key: "drive-root:local:main:8:new-hash:new-key:local,remote".to_string(),
            event_id: "new-event".to_string(),
            event_json: "{\"id\":\"new\"}".to_string(),
        };

        exchange.cache_event(older);
        let events = exchange.events_for_publish(vec![newer.clone()]);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.event_id, newer.event_id);
    }

    #[test]
    fn direct_root_app_message_coalescing_keeps_latest_root_per_family() {
        let older_drive =
            direct_root_app_message("drive-root:remote:main:7:old-hash:old-key:local,remote");
        let newer_drive =
            direct_root_app_message("drive-root:remote:main:8:new-hash:new-key:local,remote");
        let duplicate_newer =
            direct_root_app_message("drive-root:remote:main:8:new-hash:new-key:local,remote");
        let share_root =
            direct_root_app_message("share-root:share:remote:2:share-hash:share-key:local,remote");

        let (messages, skipped) = coalesce_direct_root_app_messages(vec![
            older_drive,
            share_root,
            newer_drive,
            duplicate_newer,
        ]);

        assert_eq!(skipped, 2);
        assert_eq!(
            direct_root_app_message_keys(&messages),
            vec![
                "drive-root:remote:main:8:new-hash:new-key:local,remote".to_string(),
                "share-root:share:remote:2:share-hash:share-key:local,remote".to_string(),
            ]
        );
    }

    #[test]
    fn direct_root_app_message_coalescing_preserves_unsequenced_messages() {
        let profile = direct_root_app_message("profile-op:profile:event");
        let broken = crate::FipsAppMessage {
            peer_id: "peer".to_string(),
            topic: DIRECT_ROOT_APP_TOPIC.to_string(),
            data: b"not json".to_vec(),
        };
        let other_topic = crate::FipsAppMessage {
            peer_id: "peer".to_string(),
            topic: "iris-drive/device-link/v1/request".to_string(),
            data: b"link".to_vec(),
        };
        let older_drive =
            direct_root_app_message("drive-root:remote:main:7:old-hash:old-key:local,remote");
        let newer_drive =
            direct_root_app_message("drive-root:remote:main:8:new-hash:new-key:local,remote");

        let (messages, skipped) = coalesce_direct_root_app_messages(vec![
            profile.clone(),
            broken.clone(),
            older_drive,
            other_topic.clone(),
            newer_drive,
        ]);

        assert_eq!(skipped, 1);
        assert_eq!(
            direct_root_app_message_keys(&messages[..1]),
            vec!["drive-root:remote:main:8:new-hash:new-key:local,remote".to_string()]
        );
        assert_eq!(messages[1], profile);
        assert_eq!(messages[2], broken);
        assert_eq!(messages[3], other_topic);
    }

    #[test]
    fn direct_root_app_message_coalescing_dedupes_unsequenced_direct_frames() {
        let first_profile =
            direct_root_app_message_with_event_json("profile-op:profile:event", "old");
        let latest_profile =
            direct_root_app_message_with_event_json("profile-op:profile:event", "latest");
        let other_profile =
            direct_root_app_message_with_event_json("profile-op:profile:other-event", "other");

        let (messages, skipped) = coalesce_direct_root_app_messages(vec![
            first_profile,
            other_profile.clone(),
            latest_profile,
        ]);

        assert_eq!(skipped, 1);
        assert_eq!(messages.len(), 2);
        assert_eq!(direct_root_app_message_event_json(&messages[0]), "latest");
        assert_eq!(messages[1], other_profile);
    }

    #[test]
    fn direct_root_app_message_coalescing_dedupes_files_root_frames() {
        let first_files_root = direct_root_app_message_with_event_json(
            "files-root:remote:main:root-hash:root-key",
            "old",
        );
        let latest_files_root = direct_root_app_message_with_event_json(
            "files-root:remote:main:root-hash:root-key",
            "latest",
        );
        let other_files_root = direct_root_app_message_with_event_json(
            "files-root:remote:main:other-root:other-key",
            "other",
        );

        let (messages, skipped) = coalesce_direct_root_app_messages(vec![
            first_files_root,
            other_files_root.clone(),
            latest_files_root,
        ]);

        assert_eq!(skipped, 1);
        assert_eq!(messages.len(), 2);
        assert_eq!(direct_root_app_message_event_json(&messages[0]), "latest");
        assert_eq!(messages[1], other_files_root);
    }

    #[test]
    fn direct_root_hint_frame_fits_single_fips_app_packet() {
        let local = "11".repeat(32);
        let remote = "22".repeat(32);
        let third = "33".repeat(32);
        let root_cid = sample_root_cid('a', 'b');
        let key = format!("drive-root:{remote}:main:7:{root_cid}:{local},{remote},{third}");
        let hint_bytes = encode_direct_root_hint_frame(&key, &"44".repeat(32)).unwrap();
        let full_bytes = serde_json::to_vec(&DirectRootFrame {
            key,
            event_id: "44".repeat(32),
            event_json: "x".repeat(3000),
        })
        .unwrap();

        assert!(hint_bytes.len() <= hashtree_fips_transport::FIPS_APP_FRAGMENT_SIZE);
        assert!(full_bytes.len() > hashtree_fips_transport::FIPS_APP_FRAGMENT_SIZE);
    }

    #[test]
    fn direct_root_hint_applies_authorized_drive_root_from_source_app_key() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Profile::create(dir.path(), Some("Mac".to_string())).unwrap();
        let mut state = owner.state.clone();
        state.profile_roster_ops.clear();
        state.profile_roster_projection = None;
        state.app_keys = None;
        let remote = AppKey::generate(dir.path().join("remote-key"));
        let remote_pubkey = remote.pubkey_hex();
        let old_root = sample_root_cid('1', '2');
        let new_root = sample_root_cid('3', '4');
        let mut drive = Drive::primary(state.root_scope_id());
        drive.app_key_roots.insert(
            remote_pubkey.clone(),
            AppKeyRootRef::legacy(old_root, 10, 1),
        );
        let mut config = AppConfig {
            profile: Some(state),
            drives: vec![drive],
            ..AppConfig::default()
        };
        let key = format!("drive-root:{remote_pubkey}:main:2:{new_root}:{remote_pubkey}");

        let report =
            apply_direct_root_key_hint_to_config(&mut config, &key, &remote.pubkey_bech32(), 20)
                .unwrap();
        let stored = config.drives[0].app_key_roots.get(&remote_pubkey).unwrap();

        assert_eq!(report.outcome, DirectRootHintApply::Applied);
        assert_eq!(stored.root_cid, new_root);
        assert_eq!(stored.app_key_seq, 2);
        assert_eq!(stored.published_at, 20);
    }

    #[test]
    fn direct_root_hint_rejects_sender_mismatch() {
        let dir = tempfile::tempdir().unwrap();
        let owner = Profile::create(dir.path(), Some("Mac".to_string())).unwrap();
        let mut state = owner.state.clone();
        state.profile_roster_ops.clear();
        state.profile_roster_projection = None;
        state.app_keys = None;
        let remote = AppKey::generate(dir.path().join("remote-key"));
        let impostor = AppKey::generate(dir.path().join("impostor-key"));
        let remote_pubkey = remote.pubkey_hex();
        let old_root = sample_root_cid('1', '2');
        let new_root = sample_root_cid('3', '4');
        let mut drive = Drive::primary(state.root_scope_id());
        drive.app_key_roots.insert(
            remote_pubkey.clone(),
            AppKeyRootRef::legacy(old_root.clone(), 10, 1),
        );
        let mut config = AppConfig {
            profile: Some(state),
            drives: vec![drive],
            ..AppConfig::default()
        };
        let key = format!("drive-root:{remote_pubkey}:main:2:{new_root}:{remote_pubkey}");

        let report =
            apply_direct_root_key_hint_to_config(&mut config, &key, &impostor.pubkey_bech32(), 20)
                .unwrap();
        let stored = config.drives[0].app_key_roots.get(&remote_pubkey).unwrap();

        assert_eq!(report.outcome, DirectRootHintApply::SenderMismatch);
        assert_eq!(stored.root_cid, old_root);
    }

    #[test]
    fn direct_root_app_message_coalescing_prefers_full_frame_over_hint_for_same_root() {
        let key = "drive-root:remote:main:8:new-hash:new-key:local,remote";
        let hint = direct_root_app_hint_message(key);
        let full = direct_root_app_message_with_event_json(key, "full");

        let (messages, skipped) = coalesce_direct_root_app_messages(vec![hint, full]);

        assert_eq!(skipped, 1);
        assert_eq!(messages.len(), 1);
        assert_eq!(direct_root_app_message_event_json(&messages[0]), "full");
    }

    #[test]
    fn direct_root_state_request_frame_decodes() {
        let bytes = encode_direct_root_state_request_frame("profile-scope").unwrap();
        let frame = decode_direct_root_wire_frame(&bytes).unwrap();

        assert_eq!(
            frame,
            DirectRootWireFrame::Request(DirectRootStateRequestFrame {
                root_scope_id: "profile-scope".to_string(),
                request: true,
            })
        );
    }

    #[test]
    fn direct_root_state_request_app_messages_are_not_coalesced() {
        let request = crate::FipsAppMessage {
            peer_id: "peer".to_string(),
            topic: DIRECT_ROOT_APP_TOPIC.to_string(),
            data: encode_direct_root_state_request_frame("profile-scope").unwrap(),
        };
        let root =
            direct_root_app_message("drive-root:remote:main:7:old-hash:old-key:local,remote");

        let (messages, skipped) = coalesce_direct_root_app_messages(vec![request.clone(), root]);

        assert_eq!(skipped, 0);
        assert_eq!(messages.len(), 2);
        assert!(messages.contains(&request));
    }

    #[test]
    fn direct_root_mesh_event_coalescing_keeps_latest_root_per_family() {
        let older_drive =
            direct_root_mesh_event("drive-root:remote:main:7:old-hash:old-key:local,remote");
        let newer_drive =
            direct_root_mesh_event("drive-root:remote:main:8:new-hash:new-key:local,remote");
        let duplicate_newer =
            direct_root_mesh_event("drive-root:remote:main:8:new-hash:new-key:local,remote");
        let share_root =
            direct_root_mesh_event("share-root:share:remote:2:share-hash:share-key:local,remote");

        let (messages, skipped) = coalesce_direct_root_mesh_events(vec![
            older_drive,
            share_root,
            newer_drive,
            duplicate_newer,
        ]);

        assert_eq!(skipped, 2);
        assert_eq!(
            direct_root_mesh_event_keys(&messages),
            vec![
                "drive-root:remote:main:8:new-hash:new-key:local,remote".to_string(),
                "share-root:share:remote:2:share-hash:share-key:local,remote".to_string(),
            ]
        );
    }

    #[test]
    fn direct_root_mesh_event_coalescing_preserves_unsequenced_messages() {
        let profile = direct_root_mesh_event("profile-op:profile:event");
        let broken = crate::FipsMeshPubsubEvent {
            stream_id: direct_root_mesh_stream("scope"),
            seq: 1,
            origin_peer_id: "origin".to_string(),
            from_peer_id: "peer".to_string(),
            payload: b"not json".to_vec(),
        };
        let other_stream = crate::FipsMeshPubsubEvent {
            stream_id: "other-stream".to_string(),
            seq: 1,
            origin_peer_id: "origin".to_string(),
            from_peer_id: "peer".to_string(),
            payload: b"not direct roots".to_vec(),
        };
        let older_drive =
            direct_root_mesh_event("drive-root:remote:main:7:old-hash:old-key:local,remote");
        let newer_drive =
            direct_root_mesh_event("drive-root:remote:main:8:new-hash:new-key:local,remote");

        let (messages, skipped) = coalesce_direct_root_mesh_events(vec![
            profile.clone(),
            broken.clone(),
            older_drive,
            other_stream.clone(),
            newer_drive,
        ]);

        assert_eq!(skipped, 1);
        assert_eq!(
            direct_root_mesh_event_keys(&messages[..1]),
            vec!["drive-root:remote:main:8:new-hash:new-key:local,remote".to_string()]
        );
        assert_eq!(messages[1], profile);
        assert_eq!(messages[2], broken);
        assert_eq!(messages[3], other_stream);
    }

    #[test]
    fn direct_root_mesh_event_coalescing_dedupes_unsequenced_direct_frames() {
        let first_profile =
            direct_root_mesh_event_with_event_json("profile-op:profile:event", "old");
        let latest_profile =
            direct_root_mesh_event_with_event_json("profile-op:profile:event", "latest");
        let other_profile =
            direct_root_mesh_event_with_event_json("profile-op:profile:other-event", "other");

        let (messages, skipped) = coalesce_direct_root_mesh_events(vec![
            first_profile,
            other_profile.clone(),
            latest_profile,
        ]);

        assert_eq!(skipped, 1);
        assert_eq!(messages.len(), 2);
        assert_eq!(direct_root_mesh_event_event_json(&messages[0]), "latest");
        assert_eq!(messages[1], other_profile);
    }

    #[test]
    fn direct_root_mesh_event_coalescing_dedupes_files_root_frames() {
        let first_files_root = direct_root_mesh_event_with_event_json(
            "files-root:remote:main:root-hash:root-key",
            "old",
        );
        let latest_files_root = direct_root_mesh_event_with_event_json(
            "files-root:remote:main:root-hash:root-key",
            "latest",
        );
        let other_files_root = direct_root_mesh_event_with_event_json(
            "files-root:remote:main:other-root:other-key",
            "other",
        );

        let (messages, skipped) = coalesce_direct_root_mesh_events(vec![
            first_files_root,
            other_files_root.clone(),
            latest_files_root,
        ]);

        assert_eq!(skipped, 1);
        assert_eq!(messages.len(), 2);
        assert_eq!(direct_root_mesh_event_event_json(&messages[0]), "latest");
        assert_eq!(messages[1], other_files_root);
    }

    #[test]
    fn direct_root_cache_slot_parses_real_cid_shape() {
        assert_eq!(
            direct_root_cache_slot("drive-root:device:main:8:root-hash:root-key:device,remote"),
            Some(DirectRootCacheSlot {
                family: "drive-root:device:main".to_string(),
                seq: 8,
                recipient_count: 2,
            })
        );
        assert_eq!(
            direct_root_cache_slot("share-root:share:device:9:root-hash:root-key:device,remote"),
            Some(DirectRootCacheSlot {
                family: "share-root:share:device".to_string(),
                seq: 9,
                recipient_count: 2,
            })
        );
    }

    #[test]
    fn direct_root_republish_collapses_recipient_list_variants() {
        let mut exchange = DirectRootExchange::default();
        let narrow = DirectRootEvent {
            key: "drive-root:remote:main:8:same-hash:same-key:local".to_string(),
            event_id: "narrow-event".to_string(),
            event_json: "{\"id\":\"narrow\"}".to_string(),
        };
        let wide = DirectRootEvent {
            key: "drive-root:remote:main:8:same-hash:same-key:local,remote".to_string(),
            event_id: "wide-event".to_string(),
            event_json: "{\"id\":\"wide\"}".to_string(),
        };

        exchange.cache_event(narrow);
        exchange.cache_event(wide.clone());
        let events = exchange.events_for_publish(Vec::new());

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event.event_id, wide.event_id);
    }

    #[test]
    fn direct_root_republish_skips_cached_files_root_events() {
        let mut exchange = DirectRootExchange::default();
        let remote = DirectRootEvent {
            key: "files-root:remote:main:root-hash:root-key".to_string(),
            event_id: "remote-files-root".to_string(),
            event_json: "{\"id\":\"remote\"}".to_string(),
        };

        exchange.cache_event(remote.clone());
        let events = exchange.events_for_publish(Vec::new());

        assert!(events.is_empty());
        assert!(exchange.seen_keys.contains(&remote.key));
    }

    #[test]
    fn direct_root_heartbeat_publishes_local_root_events_only() {
        let mut exchange = DirectRootExchange::default();
        exchange.cache_event(DirectRootEvent {
            key: "drive-root:remote:main:7:remote-hash:remote-key:local,remote".to_string(),
            event_id: "remote-event".to_string(),
            event_json: "{\"id\":\"remote\"}".to_string(),
        });
        let drive = DirectRootEvent {
            key: "drive-root:local:main:8:drive-hash:drive-key:local,remote".to_string(),
            event_id: "drive-event".to_string(),
            event_json: "{\"id\":\"drive\"}".to_string(),
        };
        let share = DirectRootEvent {
            key: "share-root:share:local:4:share-hash:share-key:local,remote".to_string(),
            event_id: "share-event".to_string(),
            event_json: "{\"id\":\"share\"}".to_string(),
        };
        let files = DirectRootEvent {
            key: "files-root:local:main:drive-hash:drive-key".to_string(),
            event_id: "files-event".to_string(),
            event_json: "{\"id\":\"files\"}".to_string(),
        };
        let profile = DirectRootEvent {
            key: "profile-op:profile:op".to_string(),
            event_id: "profile-event".to_string(),
            event_json: "{\"id\":\"profile\"}".to_string(),
        };

        let events = exchange.local_root_events_for_publish(
            vec![drive.clone(), share.clone(), files, profile],
            DirectRootPublishSource::LocalHeartbeat,
        );

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].event.event_id, drive.event_id);
        assert_eq!(events[0].source, DirectRootPublishSource::LocalHeartbeat);
        assert_eq!(events[1].event.event_id, share.event_id);
        assert_eq!(events[1].source, DirectRootPublishSource::LocalHeartbeat);
        assert!(should_publish_direct_root_hint(
            &drive.key,
            DirectRootPublishSource::LocalHeartbeat
        ));
        assert!(!should_publish_direct_root_full_frame(
            &drive.key,
            DirectRootPublishSource::LocalHeartbeat,
            0
        ));
        assert!(!should_publish_direct_root_full_frame(
            &share.key,
            DirectRootPublishSource::LocalHeartbeat,
            3
        ));
    }

    #[test]
    fn direct_root_state_request_reply_includes_cached_remote_roots() {
        let mut exchange = DirectRootExchange::default();
        let local = DirectRootEvent {
            key: "drive-root:local:main:8:local-hash:local-key:local,remote".to_string(),
            event_id: "local-event".to_string(),
            event_json: "{\"id\":\"local\"}".to_string(),
        };
        let remote = DirectRootEvent {
            key: "drive-root:remote:main:9:remote-hash:remote-key:local,remote".to_string(),
            event_id: "remote-event".to_string(),
            event_json: "{\"id\":\"remote\"}".to_string(),
        };

        exchange.cache_event(remote.clone());
        let events = exchange.state_request_events_for_publish(vec![local.clone()]);

        assert_eq!(events.len(), 2);
        let local_reply = events
            .iter()
            .find(|event| event.event.event_id == local.event_id)
            .unwrap();
        assert_eq!(
            local_reply.source,
            DirectRootPublishSource::StateRequestReply
        );
        assert_eq!(
            direct_root_publish_attempts_for_source(
                &local.key,
                DirectRootPublishSource::StateRequestReply,
            ),
            4
        );
        assert!(should_publish_direct_root_full_frame(
            &local.key,
            DirectRootPublishSource::StateRequestReply,
            0
        ));
        assert!(!should_publish_direct_root_full_frame(
            &local.key,
            DirectRootPublishSource::StateRequestReply,
            1
        ));
        let cached_reply = events
            .iter()
            .find(|event| event.event.event_id == remote.event_id)
            .unwrap();
        assert_eq!(
            cached_reply.source,
            DirectRootPublishSource::CachedStateRequestReply
        );
        assert_eq!(
            direct_root_publish_attempts_for_source(
                &remote.key,
                DirectRootPublishSource::CachedStateRequestReply,
            ),
            4
        );
        assert!(!should_publish_direct_root_hint(
            &remote.key,
            DirectRootPublishSource::CachedStateRequestReply,
        ));
        assert!(should_publish_direct_root_full_frame(
            &remote.key,
            DirectRootPublishSource::CachedStateRequestReply,
            3
        ));
    }

    #[test]
    fn direct_root_cached_relay_roots_publish_newer_sequence_immediately() {
        let mut exchange = DirectRootExchange::default();
        let key = "drive-root:device:main:8:root-hash:root-key:device,remote";
        let newer_key = "drive-root:device:main:9:new-root-hash:new-root-key:device,remote";
        let now = Instant::now();

        assert!(exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::CachedRelay,
            now
        ));
        assert!(!exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::CachedRelay,
            now + Duration::from_secs(DIRECT_ROOT_REPUBLISH_INTERVAL_SECS - 1)
        ));
        assert!(exchange.should_publish_candidate_key(
            newer_key,
            DirectRootPublishSource::CachedRelay,
            now + Duration::from_millis(1)
        ));
    }

    #[test]
    fn direct_root_state_request_replies_use_separate_throttle() {
        let mut exchange = DirectRootExchange::default();
        let key = "drive-root:device:main:8:root-hash:root-key:device,remote";
        let now = Instant::now();

        assert!(exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::LocalCurrent,
            now
        ));
        assert!(!exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::LocalCurrent,
            now + Duration::from_millis(500)
        ));
        assert!(exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::StateRequestReply,
            now + Duration::from_millis(500)
        ));
        assert!(!exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::StateRequestReply,
            now + Duration::from_secs(1)
        ));
        assert!(exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::StateRequestReply,
            now + Duration::from_secs(3)
        ));
        assert!(exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::CachedStateRequestReply,
            now + Duration::from_millis(500)
        ));
        assert!(!exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::CachedStateRequestReply,
            now + Duration::from_secs(1)
        ));
        assert!(exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::CachedStateRequestReply,
            now + Duration::from_secs(3)
        ));
    }

    #[test]
    fn direct_root_state_request_reply_throttle_is_per_target_peer() {
        let mut exchange = DirectRootExchange::default();
        let key = "drive-root:device:main:8:root-hash:root-key:device,remote";
        let now = Instant::now();

        assert!(exchange.should_publish_candidate_key_for_target(
            key,
            DirectRootPublishSource::CachedStateRequestReply,
            Some("peer-a"),
            now
        ));
        assert!(!exchange.should_publish_candidate_key_for_target(
            key,
            DirectRootPublishSource::CachedStateRequestReply,
            Some("peer-a"),
            now + Duration::from_millis(500)
        ));
        assert!(exchange.should_publish_candidate_key_for_target(
            key,
            DirectRootPublishSource::CachedStateRequestReply,
            Some("peer-b"),
            now + Duration::from_millis(500)
        ));
        assert!(exchange.should_publish_candidate_key_for_target(
            key,
            DirectRootPublishSource::StateRequestReply,
            Some("peer-a"),
            now + Duration::from_millis(500)
        ));
    }

    #[test]
    fn direct_root_state_request_replies_also_use_mesh_for_targeted_recovery() {
        assert!(should_publish_targeted_direct_root_reply_over_mesh(
            DirectRootPublishSource::StateRequestReply
        ));
        assert!(should_publish_targeted_direct_root_reply_over_mesh(
            DirectRootPublishSource::CachedStateRequestReply
        ));
        assert!(!should_publish_targeted_direct_root_reply_over_mesh(
            DirectRootPublishSource::LocalCurrent
        ));
        assert!(!should_publish_targeted_direct_root_reply_over_mesh(
            DirectRootPublishSource::LocalHeartbeat
        ));
        assert!(!should_publish_targeted_direct_root_reply_over_mesh(
            DirectRootPublishSource::CachedRelay
        ));
    }

    #[test]
    fn direct_root_periodic_state_requests_are_throttled() {
        let mut exchange = DirectRootExchange::default();
        let now = Instant::now();

        assert!(exchange.should_publish_state_request("scope", now));
        assert!(!exchange.should_publish_state_request(
            "scope",
            now + Duration::from_secs(DIRECT_ROOT_STATE_REQUEST_INTERVAL_SECS - 1),
        ));
        assert!(exchange.should_publish_state_request(
            "scope",
            now + Duration::from_secs(DIRECT_ROOT_STATE_REQUEST_INTERVAL_SECS),
        ));
    }

    #[test]
    fn direct_root_heartbeat_shares_local_root_throttle() {
        let mut exchange = DirectRootExchange::default();
        let key = "drive-root:device:main:8:root-hash:root-key:device,remote";
        let now = Instant::now();

        assert_eq!(
            direct_root_publish_attempts_for_source(key, DirectRootPublishSource::LocalHeartbeat),
            4
        );
        assert!(should_publish_direct_root_hint(
            key,
            DirectRootPublishSource::LocalHeartbeat
        ));
        assert!(!should_publish_direct_root_full_frame(
            key,
            DirectRootPublishSource::LocalHeartbeat,
            0
        ));
        assert!(!should_publish_direct_root_full_frame(
            key,
            DirectRootPublishSource::LocalHeartbeat,
            3
        ));
        assert!(exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::LocalCurrent,
            now
        ));
        assert!(!exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::LocalHeartbeat,
            now + Duration::from_millis(500)
        ));
        assert!(exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::LocalHeartbeat,
            now + Duration::from_secs(DIRECT_ROOT_REPUBLISH_INTERVAL_SECS)
        ));
    }

    #[test]
    fn direct_root_cache_event_preserves_same_key_republish_throttle() {
        let mut exchange = DirectRootExchange::default();
        let key = "drive-root:device:main:8:root-hash:root-key:device,remote";
        let now = Instant::now();
        let event = DirectRootEvent {
            key: key.to_string(),
            event_id: "event".to_string(),
            event_json: "{\"id\":\"event\"}".to_string(),
        };

        assert!(exchange.should_publish_key(key, now));
        exchange.cache_event(event.clone());

        assert!(!exchange.should_publish_key(key, now + Duration::from_millis(500)));
        exchange.cache_event(event);
        assert!(!exchange.should_publish_key(
            key,
            now + Duration::from_secs(DIRECT_ROOT_REPUBLISH_INTERVAL_SECS - 1)
        ));
    }

    #[test]
    fn direct_root_mesh_route_churn_does_not_clear_cached_relay_throttle() {
        let mut exchange = DirectRootExchange::default();
        let key = "drive-root:device:main:7:root-hash:root-key:device,remote";
        let now = Instant::now();

        assert!(exchange.refresh_known_root_peer_sets(
            ["authorized-a".to_string(), "authorized-b".to_string()],
            ["mesh-a".to_string()],
        ));
        assert!(exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::CachedRelay,
            now
        ));

        assert!(exchange.refresh_known_root_peer_sets(
            ["authorized-a".to_string(), "authorized-b".to_string()],
            ["mesh-b".to_string()],
        ));
        assert!(!exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::CachedRelay,
            now + Duration::from_millis(500)
        ));
    }

    #[test]
    fn direct_root_authorized_peer_loss_does_not_clear_republish_throttle() {
        let mut exchange = DirectRootExchange::default();
        let key = "drive-root:device:main:7:root-hash:root-key:device,remote";
        let now = Instant::now();

        assert!(exchange.refresh_known_root_peer_sets(
            ["authorized-a".to_string(), "authorized-b".to_string()],
            ["mesh-a".to_string()],
        ));
        assert!(exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::CachedRelay,
            now
        ));

        assert!(exchange.refresh_known_root_peer_sets(
            ["authorized-a".to_string()],
            ["mesh-a".to_string()],
        ));
        assert!(!exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::CachedRelay,
            now + Duration::from_millis(500)
        ));
    }

    #[test]
    fn direct_root_new_authorized_peer_clears_republish_throttle() {
        let mut exchange = DirectRootExchange::default();
        let key = "drive-root:device:main:7:root-hash:root-key:device,remote";
        let now = Instant::now();

        assert!(exchange.refresh_known_root_peer_sets(
            ["authorized-a".to_string(), "authorized-b".to_string()],
            ["mesh-a".to_string()],
        ));
        assert!(exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::CachedRelay,
            now
        ));

        assert!(exchange.refresh_known_root_peer_sets(
            [
                "authorized-a".to_string(),
                "authorized-b".to_string(),
                "authorized-c".to_string(),
            ],
            ["mesh-a".to_string()],
        ));
        assert!(exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::CachedRelay,
            now + Duration::from_secs(1)
        ));
    }

    #[test]
    fn direct_root_returning_authorized_peer_clears_republish_throttle() {
        let mut exchange = DirectRootExchange::default();
        let key = "drive-root:device:main:7:root-hash:root-key:device,remote";
        let now = Instant::now();

        assert!(exchange.refresh_known_root_peer_sets(
            ["authorized-a".to_string(), "authorized-b".to_string()],
            ["mesh-a".to_string()],
        ));
        assert!(exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::CachedRelay,
            now
        ));

        assert!(exchange.refresh_known_root_peer_sets(
            ["authorized-a".to_string()],
            ["mesh-a".to_string()],
        ));
        assert!(!exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::CachedRelay,
            now + Duration::from_millis(500)
        ));

        assert!(exchange.refresh_known_root_peer_sets(
            ["authorized-a".to_string(), "authorized-b".to_string()],
            ["mesh-a".to_string()],
        ));
        assert!(exchange.should_publish_candidate_key(
            key,
            DirectRootPublishSource::CachedRelay,
            now + Duration::from_secs(1)
        ));
    }

    #[test]
    fn direct_root_metadata_republishes_on_longer_cadence() {
        let mut exchange = DirectRootExchange::default();
        let key = "profile-op:profile:op";
        let now = Instant::now();

        assert!(exchange.should_publish_key(key, now));
        assert!(!exchange.should_publish_key(
            key,
            now + Duration::from_secs(DIRECT_ROOT_REPUBLISH_INTERVAL_SECS)
        ));
        assert!(exchange.should_publish_key(
            key,
            now + Duration::from_secs(DIRECT_ROOT_METADATA_REPUBLISH_INTERVAL_SECS)
        ));
    }

    fn direct_root_app_message(key: &str) -> crate::FipsAppMessage {
        direct_root_app_message_with_event_json(key, "{}")
    }

    fn direct_root_app_hint_message(key: &str) -> crate::FipsAppMessage {
        crate::FipsAppMessage {
            peer_id: "peer".to_string(),
            topic: DIRECT_ROOT_APP_TOPIC.to_string(),
            data: encode_direct_root_hint_frame(key, &format!("{key}:event")).unwrap(),
        }
    }

    fn direct_root_app_message_with_event_json(
        key: &str,
        event_json: &str,
    ) -> crate::FipsAppMessage {
        let frame = DirectRootFrame {
            key: key.to_string(),
            event_id: format!("{key}:event"),
            event_json: event_json.to_string(),
        };
        crate::FipsAppMessage {
            peer_id: "peer".to_string(),
            topic: DIRECT_ROOT_APP_TOPIC.to_string(),
            data: serde_json::to_vec(&frame).unwrap(),
        }
    }

    fn direct_root_app_message_keys(messages: &[crate::FipsAppMessage]) -> Vec<String> {
        messages
            .iter()
            .filter_map(|message| serde_json::from_slice::<DirectRootFrame>(&message.data).ok())
            .map(|frame| frame.key)
            .collect()
    }

    fn direct_root_app_message_event_json(message: &crate::FipsAppMessage) -> String {
        serde_json::from_slice::<DirectRootFrame>(&message.data)
            .unwrap()
            .event_json
    }

    fn sample_root_cid(hash_char: char, key_char: char) -> String {
        format!(
            "{}:{}",
            hash_char.to_string().repeat(64),
            key_char.to_string().repeat(64)
        )
    }

    fn direct_root_mesh_event(key: &str) -> crate::FipsMeshPubsubEvent {
        direct_root_mesh_event_with_event_json(key, "{}")
    }

    fn direct_root_mesh_event_with_event_json(
        key: &str,
        event_json: &str,
    ) -> crate::FipsMeshPubsubEvent {
        let frame = DirectRootFrame {
            key: key.to_string(),
            event_id: format!("{key}:event"),
            event_json: event_json.to_string(),
        };
        crate::FipsMeshPubsubEvent {
            stream_id: direct_root_mesh_stream("scope"),
            seq: 1,
            origin_peer_id: "origin".to_string(),
            from_peer_id: "peer".to_string(),
            payload: serde_json::to_vec(&frame).unwrap(),
        }
    }

    fn direct_root_mesh_event_keys(messages: &[crate::FipsMeshPubsubEvent]) -> Vec<String> {
        messages
            .iter()
            .filter_map(|message| serde_json::from_slice::<DirectRootFrame>(&message.payload).ok())
            .map(|frame| frame.key)
            .collect()
    }

    fn direct_root_mesh_event_event_json(message: &crate::FipsMeshPubsubEvent) -> String {
        serde_json::from_slice::<DirectRootFrame>(&message.payload)
            .unwrap()
            .event_json
    }
}
