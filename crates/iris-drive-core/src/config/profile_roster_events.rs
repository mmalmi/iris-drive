use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use nostr_sdk::{Event, JsonUtil};
use serde::{Deserialize, Serialize};

use crate::config::ConfigError;
use crate::nostr_identity::NostrIdentityId;
use crate::{SignedNostrIdentityRosterOp, parse_nostr_identity_roster_op_event};

const PROFILE_ROSTER_EVENTS_FILE: &str = "profile-roster-events.json";
pub(super) const PROFILE_ROSTER_EVENTS_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub(super) struct ProfileRosterEventStore {
    pub schema_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<NostrIdentityId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
}

pub(super) fn profile_roster_events_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(PROFILE_ROSTER_EVENTS_FILE)
}

pub(super) fn load_profile_roster_events(
    config_path: &Path,
    expected_profile_id: Option<NostrIdentityId>,
) -> Result<Vec<SignedNostrIdentityRosterOp>, ConfigError> {
    let path = profile_roster_events_path(config_path);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(&path)?;
    let store: ProfileRosterEventStore =
        serde_json::from_str(&raw).map_err(|error| ConfigError::Parse(error.to_string()))?;
    if store.schema_version > PROFILE_ROSTER_EVENTS_SCHEMA_VERSION {
        return Ok(Vec::new());
    }
    Ok(known_profile_roster_ops_from_events(
        &store.events,
        expected_profile_id.or(store.profile_id),
    ))
}

pub(super) fn save_profile_roster_events(
    config_path: &Path,
    profile_id: NostrIdentityId,
    ops: &[SignedNostrIdentityRosterOp],
) -> Result<(), ConfigError> {
    let path = profile_roster_events_path(config_path);
    let mut events = if path.exists() {
        let raw = fs::read_to_string(&path)?;
        serde_json::from_str::<ProfileRosterEventStore>(&raw)
            .map(|store| store.events)
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    events.extend(ops.iter().map(|op| op.event_json.clone()));
    events = dedupe_event_json(events);
    let store = ProfileRosterEventStore {
        schema_version: PROFILE_ROSTER_EVENTS_SCHEMA_VERSION,
        profile_id: Some(profile_id),
        events,
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let raw = serde_json::to_string_pretty(&store)
        .map_err(|error| ConfigError::Serialize(error.to_string()))?;
    crate::atomic_file::atomic_write(&path, raw.as_bytes())?;
    Ok(())
}

pub(super) fn merge_profile_roster_ops(
    current: &[SignedNostrIdentityRosterOp],
    incoming: &[SignedNostrIdentityRosterOp],
) -> Vec<SignedNostrIdentityRosterOp> {
    let mut by_id = BTreeMap::new();
    for op in current.iter().chain(incoming.iter()) {
        by_id.insert(op.op_id.clone(), op.clone());
    }
    by_id.into_values().collect()
}

fn known_profile_roster_ops_from_events(
    events: &[String],
    expected_profile_id: Option<NostrIdentityId>,
) -> Vec<SignedNostrIdentityRosterOp> {
    let mut by_id = BTreeMap::new();
    for event_json in events {
        let Ok(event) = Event::from_json(event_json) else {
            continue;
        };
        let Ok(op) = parse_nostr_identity_roster_op_event(&event) else {
            continue;
        };
        if expected_profile_id.is_some_and(|profile_id| op.content.profile_id != profile_id) {
            continue;
        }
        by_id.insert(op.op_id.clone(), op);
    }
    by_id.into_values().collect()
}

fn dedupe_event_json(events: Vec<String>) -> Vec<String> {
    let mut by_key = BTreeMap::new();
    for event_json in events {
        let key = Event::from_json(&event_json)
            .map(|event| event.id.to_string())
            .unwrap_or_else(|_| format!("raw:{event_json}"));
        by_key.insert(key, event_json);
    }
    by_key.into_values().collect()
}
