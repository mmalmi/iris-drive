//! Private, local-only status shared by all friend-backup interfaces.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use super::storage::FriendBackupStore;
use super::{FriendBackupConfig, backup_npub, encode_backup_invite};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(crate) struct PeerRuntimeStatus {
    pub root_hash: Option<String>,
    pub last_synced_at: Option<i64>,
    pub last_checked_at: Option<i64>,
    pub last_audit_attempt_at: Option<i64>,
    pub hosted_at: Option<i64>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FriendBackupStatus {
    pub backup_npub: String,
    pub invite: String,
    pub capacity_bytes: u64,
    pub used_bytes: u64,
    pub friends: Vec<FriendBackupPeerStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FriendBackupPeerStatus {
    pub npub: String,
    pub label: Option<String>,
    pub quota_bytes: u64,
    pub used_bytes: u64,
    pub state: String,
    pub state_label: String,
    pub detail: String,
    pub last_synced_at: Option<i64>,
    pub last_checked_at: Option<i64>,
}

pub fn friend_backup_status(config_dir: &Path) -> Result<FriendBackupStatus> {
    let config = FriendBackupConfig::load(config_dir)?;
    let usage = FriendBackupStore::open(config_dir.join("friend-backups"))?.all_usage_bytes()?;
    let runtime = read_runtime_status(config_dir)?;
    let npub = if crate::paths::key_path_in(config_dir).exists() {
        backup_npub(config_dir)?
    } else {
        String::new()
    };
    let invite = if npub.is_empty() {
        String::new()
    } else {
        encode_backup_invite(&npub)?
    };
    let friends = config
        .friends
        .iter()
        .map(|peer| {
            let saved = runtime.get(&peer.npub).cloned().unwrap_or_default();
            let (state, label, detail) = if !peer.enabled {
                ("paused", "Paused", "Friend backups are paused".to_string())
            } else if let Some(error) = saved.error.as_ref() {
                ("unverified", "Needs attention", error.clone())
            } else if saved.last_checked_at.is_some_and(|checked| {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                i64::try_from(now)
                    .unwrap_or(i64::MAX)
                    .saturating_sub(checked)
                    > 6 * 60 * 60
            }) {
                (
                    "unverified",
                    "Check due",
                    "The last backup check is more than six hours old".to_string(),
                )
            } else if saved.last_checked_at.is_some() {
                (
                    "verified",
                    "Backup checked",
                    "Random encrypted blocks retrieved successfully".to_string(),
                )
            } else if saved.last_synced_at.is_some() {
                (
                    "backed_up",
                    "Backed up",
                    "Your friend has received your encrypted backup".to_string(),
                )
            } else if saved.hosted_at.is_some() {
                (
                    "hosting",
                    "Helping a friend",
                    "Their encrypted backup is stored here".to_string(),
                )
            } else {
                (
                    "waiting",
                    "Waiting for friend",
                    "Both people add each other's backup link or npub".to_string(),
                )
            };
            FriendBackupPeerStatus {
                npub: peer.npub.clone(),
                label: peer.label.clone(),
                quota_bytes: peer.quota_bytes,
                used_bytes: usage.get(&peer.npub).copied().unwrap_or(0),
                state: state.into(),
                state_label: label.into(),
                detail,
                last_synced_at: saved.last_synced_at,
                last_checked_at: saved.last_checked_at,
            }
        })
        .collect();
    Ok(FriendBackupStatus {
        backup_npub: npub,
        invite,
        capacity_bytes: config.capacity_bytes,
        used_bytes: usage.values().copied().sum(),
        friends,
    })
}

pub(crate) fn read_runtime_status(
    config_dir: &Path,
) -> Result<BTreeMap<String, PeerRuntimeStatus>> {
    match std::fs::read(config_dir.join("friend-backup-status.json")) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn write_runtime_status(
    config_dir: &Path,
    status: &BTreeMap<String, PeerRuntimeStatus>,
) -> Result<()> {
    std::fs::create_dir_all(config_dir)?;
    let mut file = tempfile::NamedTempFile::new_in(config_dir)?;
    file.write_all(&serde_json::to_vec(status)?)?;
    file.as_file().sync_all()?;
    file.persist(config_dir.join("friend-backup-status.json"))?;
    Ok(())
}
