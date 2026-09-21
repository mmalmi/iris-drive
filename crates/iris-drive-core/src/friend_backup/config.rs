use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail, ensure};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::FromBech32;
use serde::{Deserialize, Serialize};

use super::{parse_backup_contact, storage::FriendBackupStore};
use crate::config_lock::ConfigMutationLock;

pub const MAX_FRIENDS: usize = 64;
pub const MAX_FRIEND_LABEL_BYTES: usize = 128;
const MAX_CONFIG_BYTES: u64 = 64 * 1024;

/// Local-only storage commitments. No social follow event is involved.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FriendBackupConfig {
    pub capacity_bytes: u64,
    pub friends: Vec<FriendBackupPeer>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FriendBackupPeer {
    pub npub: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub quota_bytes: u64,
    pub enabled: bool,
}

impl FriendBackupPeer {
    pub fn pubkey_hex(&self) -> Result<String> {
        Ok(PublicKey::from_bech32(&parse_backup_contact(&self.npub)?)?.to_hex())
    }
}

impl FriendBackupConfig {
    pub fn load(config_dir: &Path) -> Result<Self> {
        let file = match std::fs::File::open(config_dir.join("friends.toml")) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error).context("opening private friend settings"),
        };
        let mut contents = String::new();
        file.take(MAX_CONFIG_BYTES + 1)
            .read_to_string(&mut contents)?;
        ensure!(
            contents.len() as u64 <= MAX_CONFIG_BYTES,
            "friend settings are too large"
        );
        let mut config: Self =
            toml::from_str(&contents).context("reading private friend settings")?;
        config.normalize()?;
        config.validate()?;
        Ok(config)
    }

    /// Save a complete snapshot. Prefer the mutation functions for updates to
    /// existing settings: they load the latest configuration inside the lock.
    pub fn save(&self, config_dir: &Path) -> Result<()> {
        let _lock = ConfigMutationLock::acquire_blocking(config_dir)?;
        let mut config = self.clone();
        config.normalize()?;
        config.validate()?;
        config.validate_retained(&retained_usage(config_dir)?)?;
        config.save_locked(config_dir)
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.friends.len() <= MAX_FRIENDS,
            "at most {MAX_FRIENDS} friends are supported"
        );
        let mut seen = BTreeSet::new();
        let mut allocated = 0_u64;
        for peer in &self.friends {
            let npub = parse_backup_contact(&peer.npub)?;
            ensure!(seen.insert(npub), "friend is listed more than once");
            normalize_label(peer.label.clone())?;
            if peer.enabled {
                allocated = allocated
                    .checked_add(peer.quota_bytes)
                    .context("friend storage allocation overflows")?;
            }
        }
        ensure!(
            allocated <= self.capacity_bytes,
            "friend allocations exceed the total space offered"
        );
        Ok(())
    }

    /// Retained bytes must remain explicitly covered, including storage for an
    /// address absent from the contact list. Never silently drop orphan usage.
    pub fn validate_retained(&self, retained: &BTreeMap<String, u64>) -> Result<()> {
        self.validate()?;
        let mut total = 0_u64;
        for (address, bytes) in retained {
            if *bytes == 0 {
                continue;
            }
            total = total
                .checked_add(*bytes)
                .context("retained backup bytes overflow")?;
            let npub = parse_backup_contact(address)?;
            let peer = self
                .friends
                .iter()
                .find(|peer| peer.npub == npub && peer.enabled)
                .context("cannot remove or disable a friend while their backup is retained")?;
            ensure!(
                *bytes <= peer.quota_bytes,
                "cannot reduce a friend's space below their retained backup size"
            );
        }
        ensure!(
            total <= self.capacity_bytes,
            "cannot reduce total space below retained backups"
        );
        Ok(())
    }

    fn normalize(&mut self) -> Result<()> {
        for peer in &mut self.friends {
            peer.npub = parse_backup_contact(&peer.npub)?;
            peer.label = normalize_label(peer.label.take())?;
        }
        self.friends.sort_by(|a, b| a.npub.cmp(&b.npub));
        Ok(())
    }

    fn save_locked(&self, config_dir: &Path) -> Result<()> {
        let bytes = toml::to_string_pretty(self).context("encoding private friend settings")?;
        ensure!(
            bytes.len() as u64 <= MAX_CONFIG_BYTES,
            "friend settings are too large"
        );
        // NamedTempFile creates owner-only files; permissions survive the rename.
        let mut temp = tempfile::NamedTempFile::new_in(config_dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temp.as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        temp.write_all(bytes.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(config_dir.join("friends.toml"))?;
        #[cfg(unix)]
        std::fs::File::open(config_dir)?.sync_all()?;
        Ok(())
    }
}

fn normalize_label(label: Option<String>) -> Result<Option<String>> {
    let Some(label) = label else { return Ok(None) };
    ensure!(
        label.len() <= MAX_FRIEND_LABEL_BYTES,
        "friend label is too long"
    );
    ensure!(
        !label.chars().any(char::is_control),
        "friend label contains control characters"
    );
    let label = label.trim();
    Ok((!label.is_empty()).then(|| label.to_owned()))
}

fn retained_usage(config_dir: &Path) -> Result<BTreeMap<String, u64>> {
    Ok(FriendBackupStore::open(config_dir.join("friend-backups"))?.all_usage_bytes()?)
}

fn mutate(
    config_dir: &Path,
    change: impl FnOnce(&mut FriendBackupConfig) -> Result<()>,
) -> Result<FriendBackupConfig> {
    let _lock = ConfigMutationLock::acquire_blocking(config_dir)?;
    let mut config = FriendBackupConfig::load(config_dir)?;
    change(&mut config)?;
    config.normalize()?;
    config.validate_retained(&retained_usage(config_dir)?)?;
    config.save_locked(config_dir)?;
    Ok(config)
}

pub fn set_capacity(config_dir: &Path, capacity_bytes: u64) -> Result<FriendBackupConfig> {
    mutate(config_dir, |config| {
        config.capacity_bytes = capacity_bytes;
        Ok(())
    })
}

pub fn upsert_friend(
    config_dir: &Path,
    input: &str,
    label: Option<String>,
    quota_bytes: u64,
    enabled: bool,
) -> Result<FriendBackupConfig> {
    let npub = parse_backup_contact(input)?;
    let label = normalize_label(label)?;
    mutate(config_dir, |config| {
        if crate::paths::key_path_in(config_dir).exists() {
            ensure!(
                npub != super::backup_npub(config_dir)?,
                "your own backup address cannot be a friend"
            );
        }
        let peer = FriendBackupPeer {
            npub: npub.clone(),
            label,
            quota_bytes,
            enabled,
        };
        if let Some(existing) = config.friends.iter_mut().find(|peer| peer.npub == npub) {
            *existing = peer;
        } else {
            config.friends.push(peer);
        }
        Ok(())
    })
}

pub fn remove_friend(config_dir: &Path, input: &str) -> Result<FriendBackupConfig> {
    let npub = parse_backup_contact(input)?;
    mutate(config_dir, |config| {
        let before = config.friends.len();
        config.friends.retain(|peer| peer.npub != npub);
        if config.friends.len() == before {
            bail!("friend is not configured");
        }
        Ok(())
    })
}
