//! Devices this device let into a folder and has yet to tell. They are kept
//! in a private file beside the config, so a note that could not be sent
//! goes out the next time Drive reaches its relays.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context, Result};
use nostr_sdk::Client;
use serde::{Deserialize, Serialize};

use super::{ShareAddedNotice, build_share_added_notices};
use crate::config::AppConfig;
use crate::config_lock::ConfigMutationLock;
use crate::paths::{config_path_in, key_path_in};
use crate::{AppKey, NostrIdentityId, shared_folder_app_key_can_admin};

pub const SHARE_ADDED_FILE: &str = "share-added.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareAddedOutbox {
    /// The devices to tell, by folder.
    #[serde(default)]
    pub untold: BTreeMap<String, BTreeSet<String>>,
}

impl ShareAddedOutbox {
    pub fn load(config_dir: &Path) -> Result<Self> {
        let path = config_dir.join(SHARE_ADDED_FILE);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("reading {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, config_dir: &Path) -> Result<()> {
        let path = config_dir.join(SHARE_ADDED_FILE);
        let bytes = serde_json::to_vec_pretty(self)?;
        crate::atomic_file::atomic_write_private(&path, &bytes)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Drop what `told` lists: its notes were sent, or are no longer due.
    fn settle(&mut self, told: &Self) {
        for (share_id, devices) in &told.untold {
            if let Some(untold) = self.untold.get_mut(share_id) {
                untold.retain(|device| !devices.contains(device));
            }
        }
        self.untold.retain(|_, devices| !devices.is_empty());
    }
}

/// Remember the devices just let into folders. The caller holds the config
/// lock and saves the folders afterwards: if Drive stops in between, the
/// devices are let in again and owed again.
pub(crate) fn owe_share_added_notices(
    config_dir: &Path,
    added: &[(NostrIdentityId, Vec<String>)],
) -> Result<()> {
    if added.iter().all(|(_, devices)| devices.is_empty()) {
        return Ok(());
    }
    let mut outbox = ShareAddedOutbox::load(config_dir)?;
    for (share_id, devices) in added {
        outbox
            .untold
            .entry(share_id.to_string())
            .or_default()
            .extend(devices.iter().cloned());
    }
    outbox.save(config_dir)
}

/// Send the notes still owed through a relay client the caller keeps open,
/// and return how many went out. A device that has left its folder since,
/// or a folder this device no longer manages, is owed nothing.
pub async fn publish_share_added_notices(client: &Client, config_dir: &Path) -> Result<usize> {
    if ShareAddedOutbox::load(config_dir)?.untold.is_empty() || client.relays().await.is_empty() {
        return Ok(0);
    }
    // Who is owed and who is in each folder, as of the same moment.
    let lock = ConfigMutationLock::acquire(config_dir).await?;
    let owed = ShareAddedOutbox::load(config_dir)?;
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    drop(lock);
    let app_key = AppKey::load(key_path_in(config_dir)).context("loading app key")?;
    let admin = app_key.keys().public_key().to_hex();
    let now = i64::try_from(nostr_sdk::Timestamp::now().as_secs()).unwrap_or(i64::MAX);
    let mut events = Vec::new();
    for (share_id, devices) in &owed.untold {
        let Some(folder) = config
            .shared_folders
            .iter()
            .find(|folder| folder.share_id.to_string() == *share_id)
            .filter(|folder| shared_folder_app_key_can_admin(folder, &admin))
        else {
            continue;
        };
        for device in devices {
            let Some(member) = folder.active_member_for_app_key(device) else {
                continue;
            };
            events.extend(build_share_added_notices(
                app_key.keys(),
                &ShareAddedNotice::new(folder, member.role),
                std::slice::from_ref(device),
                now,
            )?);
        }
    }
    for event in &events {
        client
            .send_event(event)
            .await
            .map_err(|error| anyhow::anyhow!("telling a device of a shared folder: {error}"))?;
    }
    let lock = ConfigMutationLock::acquire(config_dir).await?;
    let mut outbox = ShareAddedOutbox::load(config_dir)?;
    outbox.settle(&owed);
    outbox.save(config_dir)?;
    drop(lock);
    Ok(events.len())
}
