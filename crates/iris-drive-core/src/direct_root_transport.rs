use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use hashtree_core::Cid;
use nostr_sdk::{Event, JsonUtil};
use serde::{Deserialize, Serialize};

use crate::paths::{config_path_in, key_path_in};
use crate::{
    AccountState, AppConfig, DeviceIdentity, FsFipsBlockSync, PRIMARY_DRIVE_ID,
    authorized_device_pubkeys,
};

pub const DIRECT_ROOT_APP_TOPIC: &str = "iris-drive/root-events/v1/direct";
pub const DIRECT_ROOT_MESH_STREAM_PREFIX: &str = "iris-drive/root-events/v1";

const DIRECT_ROOT_EVENT_CACHE_CAP: usize = 128;
const DIRECT_ROOT_REPUBLISH_INTERVAL_SECS: u64 = 5;
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

#[derive(Default)]
pub struct DirectRootExchange {
    cached_events: BTreeMap<String, DirectRootEvent>,
    published_keys: BTreeMap<String, Instant>,
    seen_keys: BTreeSet<String>,
    subscribed_streams: BTreeSet<String>,
    known_mesh_peers: BTreeSet<String>,
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
        let Some(state) = config.account.as_ref() else {
            return Ok(());
        };
        let root_scope_id = state.root_scope_id();
        self.subscribe_owner_stream(&root_scope_id, sync).await;
        let stream = direct_root_mesh_stream(&root_scope_id);
        let events = self.events_for_publish(
            build_current_direct_root_events(config_dir, &config, state)
                .map_err(|error| format!("{error:#}"))?,
        );
        let now = Instant::now();
        for event in events {
            if !self.should_publish_key(&event.key, now) {
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
        for message in sync.drain_mesh_pubsub_events().await {
            if !message
                .stream_id
                .starts_with(DIRECT_ROOT_MESH_STREAM_PREFIX)
            {
                continue;
            }
            let frame: DirectRootFrame = serde_json::from_slice(&message.payload)
                .map_err(|error| format!("parsing direct-root mesh frame: {error}"))?;
            self.apply_frame(config_dir, sync, frame).await?;
        }
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
                self.cache_event(direct_event);
                if changed {
                    self.announce_current_state(config_dir, sync).await?;
                }
                Ok(true)
            }
            Err(error) => {
                self.seen_keys.remove(&frame.key);
                Err(format!("{error:#}"))
            }
        }
    }

    async fn subscribe_owner_stream(&mut self, owner_pubkey: &str, sync: &FsFipsBlockSync) {
        let stream = direct_root_mesh_stream(owner_pubkey);
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
        let mut root_peers = sync
            .authorized_peer_ids()
            .await
            .into_iter()
            .collect::<BTreeSet<_>>();
        root_peers.extend(sync.mesh_peer_ids().await);
        if root_peers != self.known_mesh_peers {
            self.known_mesh_peers = root_peers;
            self.published_keys.clear();
            return true;
        }
        false
    }

    fn next_mesh_publish_seq(&mut self) -> u64 {
        self.next_mesh_publish_seq = self.next_mesh_publish_seq.saturating_add(1).max(1);
        self.next_mesh_publish_seq
    }

    fn should_publish_key(&mut self, key: &str, now: Instant) -> bool {
        if self.published_keys.get(key).is_some_and(|last| {
            now.duration_since(*last) < Duration::from_secs(DIRECT_ROOT_REPUBLISH_INTERVAL_SECS)
        }) {
            return false;
        }
        self.published_keys.insert(key.to_owned(), now);
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

    fn events_for_publish(&self, local_events: Vec<DirectRootEvent>) -> Vec<DirectRootEvent> {
        let mut events = Vec::with_capacity(local_events.len() + self.cached_events.len());
        let mut keys = BTreeSet::new();
        for event in local_events {
            let event = self.event_for_publish(event);
            keys.insert(event.key.clone());
            events.push(event);
        }
        events.extend(
            self.cached_events
                .values()
                .filter(|event| !keys.contains(&event.key))
                .cloned(),
        );
        events
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

pub fn build_current_direct_root_events(
    config_dir: &Path,
    config: &AppConfig,
    state: &AccountState,
) -> Result<Vec<DirectRootEvent>> {
    let mut events = Vec::new();
    for op in &state.profile_roster_ops {
        let event =
            Event::from_json(&op.event_json).context("parsing IrisProfile roster op event")?;
        events.push(direct_root_event(
            format!("profile-op:{}:{}", state.profile_id, op.op_id),
            &event,
        ));
    }
    if let Some(drive) = config.drive(PRIMARY_DRIVE_ID)
        && let Some(root) = drive.device_roots.get(&state.device_pubkey)
    {
        let device = DeviceIdentity::load(key_path_in(config_dir)).context("loading device key")?;
        let authorized_devices = authorized_device_pubkeys(state);
        let event = crate::nostr_events::build_drive_root_event(
            device.keys(),
            &state.root_scope_id(),
            &drive.drive_id,
            root,
            &authorized_devices,
        )
        .context("building drive-root event")?;
        events.push(direct_root_event(
            format!(
                "drive-root:{}:{}:{}:{}:{}",
                state.device_pubkey,
                drive.drive_id,
                root.device_seq,
                root.root_cid,
                authorized_devices.join(",")
            ),
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
pub fn direct_root_mesh_stream(owner_pubkey: &str) -> String {
    format!("{DIRECT_ROOT_MESH_STREAM_PREFIX}/{owner_pubkey}")
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
    if crate::nostr_events::is_drive_root_event_coordinate(event) {
        let device = DeviceIdentity::load(key_path_in(config_dir)).context("loading device key")?;
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
    let Some(account) = config.account.as_ref() else {
        return false;
    };
    config.drive(PRIMARY_DRIVE_ID).is_some_and(|drive| {
        drive
            .device_roots
            .iter()
            .any(|(device, root)| device != &account.device_pubkey && root.root_cid == root_cid)
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
