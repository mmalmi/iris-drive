use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use hashtree_core::Cid;
use nostr_sdk::{Event, JsonUtil};
use serde::{Deserialize, Serialize};

use crate::paths::{config_path_in, key_path_in};
use crate::{
    AppConfig, AppKey, FsFipsBlockSync, PRIMARY_DRIVE_ID, ProfileState,
    drive_root_recipient_app_key_pubkeys,
};

pub const DIRECT_ROOT_APP_TOPIC: &str = "iris-drive/root-events/v1/direct";
pub const DIRECT_ROOT_MESH_STREAM_PREFIX: &str = "iris-drive/root-events/v1";

const DIRECT_ROOT_EVENT_CACHE_CAP: usize = 128;
const DIRECT_ROOT_REPUBLISH_INTERVAL_SECS: u64 = 5;
const DIRECT_ROOT_METADATA_REPUBLISH_INTERVAL_SECS: u64 = 300;
const DIRECT_ROOT_DOWNLOAD_TIMEOUT_SECS: u64 = 8;

#[derive(Debug, Serialize, Deserialize)]
pub struct DirectRootFrame {
    pub key: String,
    pub event_id: String,
    pub event_json: String,
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
    CachedRelay,
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
            let DirectRootPublishEvent { event, source } = publish_event;
            if !self.should_publish_candidate_key(&event.key, source, now) {
                continue;
            }
            let frame = DirectRootFrame {
                key: event.key.clone(),
                event_id: event.event_id.clone(),
                event_json: event.event_json.clone(),
            };
            let bytes = serde_json::to_vec(&frame)
                .map_err(|error| format!("encoding direct root: {error}"))?;
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
            let seq = self.next_mesh_publish_seq();
            let publish = sync.publish_mesh_pubsub(stream.clone(), seq, bytes).await;
            tracing::debug!(
                root_key = event.key.as_str(),
                sent_peers = publish.sent_peers,
                seq,
                "published direct-root mesh event over FIPS"
            );
        }
        self.prune_published_keys();
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
        let frame: DirectRootFrame = serde_json::from_slice(&message.data)
            .map_err(|error| format!("parsing direct-root frame: {error}"))?;
        self.apply_frame(config_dir, sync, frame).await
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
        let frame: DirectRootFrame = serde_json::from_slice(&message.payload)
            .map_err(|error| format!("parsing direct-root mesh frame: {error}"))?;
        self.apply_frame(config_dir, sync, frame).await?;
        Ok(())
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
            self.known_publish_peers.extend(publish_peers);
            self.published_keys.clear();
        }
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

    fn should_publish_candidate_key(
        &mut self,
        key: &str,
        source: DirectRootPublishSource,
        now: Instant,
    ) -> bool {
        let throttle_key = direct_root_publish_throttle_key(key, source);
        if self.published_keys.get(&throttle_key).is_some_and(|last| {
            now.duration_since(*last)
                < Duration::from_secs(direct_root_republish_interval_secs_for_source(key, source))
        }) {
            return false;
        }
        self.published_keys.insert(throttle_key, now);
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
                direct_root_cache_slot(key).is_some_and(|cached| {
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
    let mut latest_roots = BTreeMap::<String, (DirectRootCacheSlot, crate::FipsAppMessage)>::new();
    let mut unsequenced_indices = BTreeMap::<String, usize>::new();
    let mut skipped = 0usize;

    for message in messages {
        let Some(frame_key) = direct_root_message_frame_key(&message) else {
            passthrough.push(message);
            continue;
        };
        let Some(slot) = direct_root_cache_slot(&frame_key) else {
            if let Some(cache_key) = direct_root_unsequenced_batch_key(&frame_key) {
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
                entry.insert((slot, message));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if direct_root_slot_is_strictly_newer(&slot, &entry.get().0) {
                    skipped = skipped.saturating_add(1);
                    entry.insert((slot, message));
                } else {
                    skipped = skipped.saturating_add(1);
                }
            }
        }
    }

    let mut coalesced = latest_roots
        .into_values()
        .map(|(_, message)| message)
        .collect::<Vec<_>>();
    coalesced.extend(passthrough);
    (coalesced, skipped)
}

pub fn coalesce_direct_root_mesh_events(
    messages: Vec<crate::FipsMeshPubsubEvent>,
) -> (Vec<crate::FipsMeshPubsubEvent>, usize) {
    let mut passthrough = Vec::new();
    let mut latest_roots =
        BTreeMap::<String, (DirectRootCacheSlot, crate::FipsMeshPubsubEvent)>::new();
    let mut unsequenced_indices = BTreeMap::<String, usize>::new();
    let mut skipped = 0usize;

    for message in messages {
        let Some(frame_key) = direct_root_mesh_event_frame_key(&message) else {
            passthrough.push(message);
            continue;
        };
        let Some(slot) = direct_root_cache_slot(&frame_key) else {
            if let Some(cache_key) = direct_root_unsequenced_batch_key(&frame_key) {
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
                entry.insert((slot, message));
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                if direct_root_slot_is_strictly_newer(&slot, &entry.get().0) {
                    skipped = skipped.saturating_add(1);
                    entry.insert((slot, message));
                } else {
                    skipped = skipped.saturating_add(1);
                }
            }
        }
    }

    let mut coalesced = latest_roots
        .into_values()
        .map(|(_, message)| message)
        .collect::<Vec<_>>();
    coalesced.extend(passthrough);
    (coalesced, skipped)
}

fn direct_root_message_frame_key(message: &crate::FipsAppMessage) -> Option<String> {
    if message.topic != DIRECT_ROOT_APP_TOPIC {
        return None;
    }
    let frame: DirectRootFrame = serde_json::from_slice(&message.data).ok()?;
    Some(frame.key)
}

fn direct_root_mesh_event_frame_key(message: &crate::FipsMeshPubsubEvent) -> Option<String> {
    if !message
        .stream_id
        .starts_with(DIRECT_ROOT_MESH_STREAM_PREFIX)
    {
        return None;
    }
    let frame: DirectRootFrame = serde_json::from_slice(&message.payload).ok()?;
    Some(frame.key)
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
    if source == DirectRootPublishSource::CachedRelay && direct_root_cache_slot(key).is_some() {
        return DIRECT_ROOT_REPUBLISH_INTERVAL_SECS;
    }
    if direct_root_cache_slot(key).is_some() {
        DIRECT_ROOT_REPUBLISH_INTERVAL_SECS
    } else {
        DIRECT_ROOT_METADATA_REPUBLISH_INTERVAL_SECS
    }
}

fn direct_root_publish_throttle_key(key: &str, source: DirectRootPublishSource) -> String {
    if source == DirectRootPublishSource::CachedRelay && direct_root_cache_slot(key).is_some() {
        return format!("cached-relay:{key}");
    }
    key.to_string()
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
            Event::from_json(&op.event_json).context("parsing IrisProfile roster op event")?;
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
    if crate::is_iris_profile_roster_op_event_coordinate(event) {
        let outcome =
            crate::relay_sync::apply_remote_iris_profile_roster_op_event(&mut config, event)?;
        let changed = matches!(
            outcome,
            crate::relay_sync::IrisProfileRosterOpApply::Applied
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
