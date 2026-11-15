//! What this device knows of the profile's device reports, kept in the
//! config directory so every process (the daemon, the app, the CLI) shows the
//! same at once, without asking relays: its own last measurement and what it
//! last sent, and the newest report heard from each other device.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use nostr_sdk::Event;
use serde::{Deserialize, Serialize};

use super::event::{is_device_storage_event_for, open_device_storage_event};
use super::report::DeviceStorageReport;
use crate::config::AppConfig;
use crate::paths::{config_path_in, key_path_in};
use crate::sealed_content::SealedContentError;
use crate::{AppKey, PRIMARY_DRIVE_ID, ProfileState};

pub const DEVICE_STORAGE_FILE: &str = "device-storage.json";
const LOCK_FILE: &str = "device-storage.lock";

/// What this device last sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SentReport {
    /// Unix seconds.
    pub at: u64,
    /// The `created_at` it was sent with.
    pub created_at: u64,
    pub recipients: Vec<String>,
    pub report: DeviceStorageReport,
}

/// A report another device sent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeardReport {
    /// When the device sent it: the event's `created_at`.
    pub created_at: u64,
    pub report: DeviceStorageReport,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStorageState {
    /// The profile these are for; another profile starts afresh.
    #[serde(default)]
    pub profile_id: String,
    /// This device's newest measurement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub own: Option<DeviceStorageReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent: Option<SentReport>,
    /// The newest report of each other device, by its key.
    #[serde(default)]
    pub heard: BTreeMap<String, HeardReport>,
    /// The newest `created_at` relays had of this device's own report, so a
    /// new one is always later, also after reinstalling.
    #[serde(default)]
    pub own_heard_at: u64,
}

impl DeviceStorageState {
    #[must_use]
    pub fn load(config_dir: &Path, profile_id: &str) -> Self {
        std::fs::read(config_dir.join(DEVICE_STORAGE_FILE))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Self>(&bytes).ok())
            .filter(|state| state.profile_id == profile_id)
            .unwrap_or_else(|| Self {
                profile_id: profile_id.to_owned(),
                ..Self::default()
            })
    }

    fn save(&self, config_dir: &Path) -> Result<()> {
        let path = config_dir.join(DEVICE_STORAGE_FILE);
        crate::atomic_file::atomic_write_private(&path, &serde_json::to_vec(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// The `created_at` for the next report: never at or before the last one.
    #[must_use]
    pub fn next_created_at(&self, now: u64) -> u64 {
        now.max(self.sent.as_ref().map_or(0, |sent| sent.created_at + 1))
            .max(self.own_heard_at.saturating_add(1))
    }
}

/// Read, change and save the state under a file lock, so the daemon and the
/// app never write over each other.
pub fn update_device_storage_state<T>(
    config_dir: &Path,
    profile_id: &str,
    change: impl FnOnce(&mut DeviceStorageState) -> T,
) -> Result<T> {
    std::fs::create_dir_all(config_dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(config_dir.join(LOCK_FILE))?;
    // The OS releases the lock when `lock` drops or the process exits.
    lock.lock()?;
    let mut state = DeviceStorageState::load(config_dir, profile_id);
    let before = state.clone();
    let result = change(&mut state);
    if state != before {
        state.save(config_dir)?;
    }
    Ok(result)
}

/// The profile's devices: the ones a report is sent to and heard from, the
/// same ones a Drive root is wrapped for.
#[must_use]
pub fn profile_devices(config: &AppConfig, state: &ProfileState) -> Vec<String> {
    let mut devices = config.drive(PRIMARY_DRIVE_ID).map_or_else(
        || state.active_root_writer_app_key_pubkeys(),
        |drive| crate::drive_root_recipient_app_key_pubkeys(state, drive),
    );
    devices.push(state.app_key_pubkey.clone());
    devices.sort();
    devices.dedup();
    devices
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceStorageApply {
    /// Not a report of this profile.
    NotOurs,
    /// Signed by a key that is not one of the profile's devices.
    Unauthorized,
    /// Not wrapped for this device: sent before it was linked.
    KeyUnavailable,
    /// Nothing newer than what this device has.
    Current,
    Applied,
}

/// What taking in a report did, and whether its sender has not been told
/// this device's own report yet, as a device linked since it was last sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceStorageApplied {
    pub outcome: DeviceStorageApply,
    pub sender_not_told: bool,
}

/// Take in another device's report, or note this device's own as relays
/// have it.
pub fn apply_device_storage_event(
    config_dir: &Path,
    config: &AppConfig,
    event: &Event,
) -> Result<DeviceStorageApplied> {
    let not = |outcome| DeviceStorageApplied {
        outcome,
        sender_not_told: false,
    };
    let Some(state) = config.profile.as_ref() else {
        return Ok(not(DeviceStorageApply::NotOurs));
    };
    let profile_id = state.root_scope_id();
    if !is_device_storage_event_for(event, &profile_id) {
        return Ok(not(DeviceStorageApply::NotOurs));
    }
    let author = event.pubkey.to_hex();
    let created_at = event.created_at.as_secs();
    if author == state.app_key_pubkey {
        if event.verify().is_err() {
            return Ok(not(DeviceStorageApply::Unauthorized));
        }
        update_device_storage_state(config_dir, &profile_id, |stored| {
            stored.own_heard_at = stored.own_heard_at.max(created_at);
        })?;
        return Ok(not(DeviceStorageApply::Current));
    }
    let devices = profile_devices(config, state);
    if !devices.contains(&author) {
        return Ok(not(DeviceStorageApply::Unauthorized));
    }
    let known = DeviceStorageState::load(config_dir, &profile_id);
    if known
        .heard
        .get(&author)
        .is_some_and(|heard| heard.created_at >= created_at)
    {
        return Ok(not(DeviceStorageApply::Current));
    }
    let app_key = AppKey::load(key_path_in(config_dir)).context("loading app key")?;
    let report = match open_device_storage_event(event, &profile_id, app_key.keys()) {
        Ok(report) => report,
        Err(SealedContentError::KeyUnavailable) => {
            return Ok(DeviceStorageApplied {
                outcome: DeviceStorageApply::KeyUnavailable,
                sender_not_told: not_told(&known, &author),
            });
        }
        Err(SealedContentError::Invalid(_)) => return Ok(not(DeviceStorageApply::Unauthorized)),
    };
    update_device_storage_state(config_dir, &profile_id, |stored| {
        let newer = stored
            .heard
            .get(&author)
            .is_none_or(|heard| heard.created_at < created_at);
        if newer {
            stored
                .heard
                .insert(author.clone(), HeardReport { created_at, report });
        }
        DeviceStorageApplied {
            outcome: if newer {
                DeviceStorageApply::Applied
            } else {
                DeviceStorageApply::Current
            },
            sender_not_told: not_told(stored, &author),
        }
    })
}

fn not_told(state: &DeviceStorageState, author: &str) -> bool {
    state
        .sent
        .as_ref()
        .is_some_and(|sent| !sent.recipients.iter().any(|device| device == author))
}

/// Take in a report that came from relays, and have this process's reporter
/// tell its sender when it has not been told yet.
pub fn take_in_relay_event(config_dir: &Path, event: &Event) -> Result<DeviceStorageApplied> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let applied = apply_device_storage_event(config_dir, &config, event)?;
    if applied.sender_not_told {
        super::reporter::nudge_running_reporter();
    }
    Ok(applied)
}
