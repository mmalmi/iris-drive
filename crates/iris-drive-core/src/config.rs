//! Persistent app config: drive list, schema version, identity reference.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use std::collections::BTreeMap;

use crate::account::AccountState;
use crate::CONFIG_SCHEMA_VERSION;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse: {0}")]
    Parse(String),
    #[error("serialize: {0}")]
    Serialize(String),
    #[error("schema version {found} not supported (expected {expected})")]
    SchemaMismatch { found: u32, expected: u32 },
}

/// Default Nostr relays for new installs. Users override via config or
/// `--relay` flags. Public, write-friendly, no auth — fine for v1.
pub const DEFAULT_RELAYS: &[&str] = &["wss://relay.damus.io", "wss://nos.lol"];

/// Default Blossom servers for new installs — HTTP blob hosts used to
/// transfer the actual htree blocks between devices. Empty by default
/// (block replication is opt-in). Configure via `blossom_servers` or
/// `--blossom-server` flags.
pub const DEFAULT_BLOSSOM_SERVERS: &[&str] = &[];

/// Top-level app state stored at `<config_dir>/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub schema_version: u32,
    /// Account state. `None` until the user has run a create / restore
    /// / link flow.
    #[serde(default)]
    pub account: Option<AccountState>,
    #[serde(default)]
    pub drives: Vec<Drive>,
    /// Relays to publish to and subscribe from. Defaults to
    /// [`DEFAULT_RELAYS`] on a fresh install.
    #[serde(default = "default_relays")]
    pub relays: Vec<String>,
    /// Blossom HTTP blob servers used for block replication between
    /// devices. Empty by default; configure to enable file-content
    /// sync (not just metadata).
    #[serde(default)]
    pub blossom_servers: Vec<String>,
}

fn default_relays() -> Vec<String> {
    DEFAULT_RELAYS.iter().map(|s| (*s).to_string()).collect()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            account: None,
            drives: Vec::new(),
            relays: default_relays(),
            blossom_servers: DEFAULT_BLOSSOM_SERVERS
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
        }
    }
}

impl AppConfig {
    /// Insert or update a drive by `drive_id`. Returns `true` if the drive
    /// was new.
    pub fn upsert_drive(&mut self, drive: Drive) -> bool {
        if let Some(existing) = self.drives.iter_mut().find(|d| d.drive_id == drive.drive_id) {
            *existing = drive;
            false
        } else {
            self.drives.push(drive);
            true
        }
    }

    #[must_use] 
    pub fn drive(&self, drive_id: &str) -> Option<&Drive> {
        self.drives.iter().find(|d| d.drive_id == drive_id)
    }

    pub fn remove_drive(&mut self, drive_id: &str) -> Option<Drive> {
        let pos = self.drives.iter().position(|d| d.drive_id == drive_id)?;
        Some(self.drives.remove(pos))
    }

    /// Load from path. Missing file → `Default::default()`.
    pub fn load_or_default(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)?;
        let parsed: Self = toml::from_str(&raw).map_err(|e| ConfigError::Parse(e.to_string()))?;
        if parsed.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::SchemaMismatch {
                found: parsed.schema_version,
                expected: CONFIG_SCHEMA_VERSION,
            });
        }
        Ok(parsed)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = toml::to_string_pretty(self).map_err(|e| ConfigError::Serialize(e.to_string()))?;
        fs::write(path, raw)?;
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriveRole {
    /// The user owns this drive. They can edit and reshare.
    Owner,
    /// Shared with this user; they can edit.
    Editor,
    /// Shared with this user read-only.
    Reader,
}

impl DriveRole {
    #[must_use] 
    pub fn can_write(self) -> bool {
        matches!(self, DriveRole::Owner | DriveRole::Editor)
    }
}

/// A drive is one logical mount-point. The user's primary "My Drive" is
/// stored as `drive_id = "main", role = Owner, owner_pubkey = self`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Drive {
    pub owner_pubkey: String,
    pub drive_id: String,
    pub display_name: String,
    pub role: DriveRole,
    /// Per-device drive roots, keyed by `device_pubkey` (hex). Every
    /// authorized device publishes its own root tree; the merged view
    /// is computed by LWW across all entries (see
    /// [`crate::merge::merge_drives`]). Single-device installs carry
    /// exactly one entry here.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub device_roots: BTreeMap<String, DeviceRootRef>,
    /// Deprecated: this device's most-recent root CID. Retained as a
    /// flat scalar for compatibility with existing tooling that hasn't
    /// learned `device_roots` yet. New code should read
    /// `device_roots[my_device_pubkey].root_cid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_root_cid: Option<String>,
    /// Local filesystem path this device backs the drive with. Set on
    /// the first `idrive import`; the daemon watches this dir for
    /// changes and auto-republishes. `None` until first import.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_dir: Option<std::path::PathBuf>,
    /// Symmetric key for encrypted drives, hex-encoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_hex: Option<String>,
}

/// One device's contribution to a drive. Each authorized device
/// publishes its own root tree; the merged drive view aggregates
/// across all entries via LWW per path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceRootRef {
    /// htree root CID the device most recently published.
    pub root_cid: String,
    /// Unix-seconds publication time. Drives LWW ordering at merge
    /// time: among writers for the same path, the device with the
    /// most recent `published_at` wins.
    pub published_at: i64,
    /// DCK generation this root was sealed with. Lets readers detect
    /// stale device roots that pre-date a rotation.
    pub dck_generation: u64,
}

impl Drive {
    pub fn primary(owner_pubkey: impl Into<String>) -> Self {
        Self {
            owner_pubkey: owner_pubkey.into(),
            drive_id: "main".into(),
            display_name: "My Drive".into(),
            role: DriveRole::Owner,
            device_roots: BTreeMap::new(),
            last_root_cid: None,
            working_dir: None,
            key_hex: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn default_has_no_drives_and_current_schema() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.schema_version, CONFIG_SCHEMA_VERSION);
        assert!(cfg.drives.is_empty());
    }

    #[test]
    fn round_trip_through_toml() {
        let mut cfg = AppConfig::default();
        cfg.upsert_drive(Drive::primary("abc123"));
        cfg.upsert_drive(Drive {
            owner_pubkey: "def456".into(),
            drive_id: "shared-photos".into(),
            display_name: "Photos from Alice".into(),
            role: DriveRole::Reader,
            device_roots: BTreeMap::new(),
            last_root_cid: Some("Q123abc".into()),
            working_dir: None,
            key_hex: Some("deadbeef".into()),
        });

        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        cfg.save(&path).unwrap();
        let loaded = AppConfig::load_or_default(&path).unwrap();
        assert_eq!(loaded.drives.len(), 2);
        assert_eq!(loaded.drives, cfg.drives);
    }

    #[test]
    fn load_missing_file_returns_default() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("does-not-exist.toml");
        let cfg = AppConfig::load_or_default(&path).unwrap();
        assert!(cfg.drives.is_empty());
    }

    #[test]
    fn upsert_replaces_by_drive_id() {
        let mut cfg = AppConfig::default();
        assert!(cfg.upsert_drive(Drive::primary("abc")));
        let mut updated = Drive::primary("abc");
        updated.display_name = "Renamed".into();
        assert!(!cfg.upsert_drive(updated)); // not new
        assert_eq!(cfg.drives.len(), 1);
        assert_eq!(cfg.drives[0].display_name, "Renamed");
    }

    #[test]
    fn remove_drive_returns_removed() {
        let mut cfg = AppConfig::default();
        cfg.upsert_drive(Drive::primary("abc"));
        let removed = cfg.remove_drive("main").unwrap();
        assert_eq!(removed.owner_pubkey, "abc");
        assert!(cfg.drives.is_empty());
        assert!(cfg.remove_drive("main").is_none());
    }

    #[test]
    fn schema_mismatch_errors() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let bogus = format!(
            "schema_version = {}\ndrives = []\n",
            CONFIG_SCHEMA_VERSION + 1
        );
        std::fs::write(&path, bogus).unwrap();
        match AppConfig::load_or_default(&path) {
            Err(ConfigError::SchemaMismatch { .. }) => {}
            other => panic!("expected SchemaMismatch, got {other:?}"),
        }
    }

    #[test]
    fn drive_role_can_write() {
        assert!(DriveRole::Owner.can_write());
        assert!(DriveRole::Editor.can_write());
        assert!(!DriveRole::Reader.can_write());
    }

    #[test]
    fn primary_drive_is_owner_named_main() {
        let d = Drive::primary("abc");
        assert_eq!(d.drive_id, "main");
        assert_eq!(d.role, DriveRole::Owner);
        assert_eq!(d.display_name, "My Drive");
    }
}
