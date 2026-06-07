//! Persistent app config: drive list, schema version, identity reference.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use std::collections::BTreeMap;

use crate::CONFIG_SCHEMA_VERSION;
use crate::iris_profile::IrisProfileId;
use crate::profile::ProfileState;
use crate::root_meta::{DriveRootMeta, RootObservation, RootParent};
use crate::sharing::{ShareShortcut, SharedFolder};

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
/// `--relay` flags. Kept in sync with hashtree's shared defaults.
pub const DEFAULT_RELAYS: &[&str] = hashtree_config::DEFAULT_RELAYS;

/// Default Blossom servers for new installs — HTTP blob hosts used as a
/// remote cache for htree block replication. Direct FIPS transfer between
/// authorized Iris Drive instances is preferred when peers are online.
/// `upload.iris.to` rejects unencrypted uploads, matching Iris Drive's
/// private-by-default model.
pub const DEFAULT_BLOSSOM_SERVERS: &[&str] = &["https://upload.iris.to"];

/// Top-level app state stored at `<config_dir>/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u32,
    /// Local `IrisProfile` state. `None` until the user has run a create /
    /// restore / link flow.
    #[serde(default)]
    pub profile: Option<ProfileState>,
    /// Optional local profile metadata collected during account creation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_profile: Option<UserProfile>,
    #[serde(default)]
    pub drives: Vec<Drive>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shared_folders: Vec<SharedFolder>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub share_shortcuts: Vec<ShareShortcut>,
    /// Relays to publish to and subscribe from. Defaults to
    /// [`DEFAULT_RELAYS`] on a fresh install.
    #[serde(default = "default_relays")]
    pub relays: Vec<String>,
    /// Blossom HTTP blob servers used for block replication between
    /// devices. Defaults to [`DEFAULT_BLOSSOM_SERVERS`] on fresh
    /// installs and when loading older configs that lack this field.
    #[serde(default = "default_blossom_servers")]
    pub blossom_servers: Vec<String>,
    /// Local loopback gateway/resolver for browser access through
    /// `nhash.iris.localhost`. Defaults on so native shells are
    /// interoperable with web Drive links without extra setup.
    #[serde(default = "default_true")]
    pub local_nhash_resolver_enabled: bool,
    /// Encrypted offsite backup endpoints. Blossom targets are usable
    /// today; FIPS targets are stored for the future direct-device
    /// backup transport.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backup_targets: Vec<BackupTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UserProfile {
    pub username: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo_path: Option<String>,
}

impl UserProfile {
    #[must_use]
    pub fn from_optional(username: Option<&str>, photo_path: Option<&str>) -> Option<Self> {
        let username = username
            .map(str::trim)
            .filter(|value| !value.is_empty())?
            .to_string();
        let photo_path = photo_path.and_then(|path| {
            let trimmed = path.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });
        Some(Self {
            username,
            photo_path,
        })
    }
}

fn default_relays() -> Vec<String> {
    DEFAULT_RELAYS.iter().map(|s| (*s).to_string()).collect()
}

fn default_blossom_servers() -> Vec<String> {
    DEFAULT_BLOSSOM_SERVERS
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            schema_version: CONFIG_SCHEMA_VERSION,
            profile: None,
            user_profile: None,
            drives: Vec::new(),
            shared_folders: Vec::new(),
            share_shortcuts: Vec::new(),
            relays: default_relays(),
            blossom_servers: default_blossom_servers(),
            local_nhash_resolver_enabled: true,
            backup_targets: Vec::new(),
        }
    }
}

impl AppConfig {
    /// Insert or update a drive by `drive_id`. Returns `true` if the drive
    /// was new.
    pub fn upsert_drive(&mut self, drive: Drive) -> bool {
        if let Some(existing) = self
            .drives
            .iter_mut()
            .find(|d| d.drive_id == drive.drive_id)
        {
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

    #[must_use]
    pub fn shared_folder(&self, share_id: IrisProfileId) -> Option<&SharedFolder> {
        self.shared_folders
            .iter()
            .find(|share| share.share_id == share_id)
    }

    pub fn upsert_shared_folder(&mut self, shared_folder: SharedFolder) -> bool {
        if let Some(existing) = self
            .shared_folders
            .iter_mut()
            .find(|existing| existing.share_id == shared_folder.share_id)
        {
            *existing = shared_folder;
            false
        } else {
            self.shared_folders.push(shared_folder);
            true
        }
    }

    pub fn remove_shared_folder(&mut self, share_id: IrisProfileId) -> Option<SharedFolder> {
        let pos = self
            .shared_folders
            .iter()
            .position(|share| share.share_id == share_id)?;
        Some(self.shared_folders.remove(pos))
    }

    pub fn upsert_share_shortcut(&mut self, shortcut: ShareShortcut) -> bool {
        if let Some(existing) = self
            .share_shortcuts
            .iter_mut()
            .find(|existing| existing.path == shortcut.path)
        {
            *existing = shortcut;
            false
        } else {
            self.share_shortcuts.push(shortcut);
            true
        }
    }

    pub fn remove_share_shortcuts_for_share(
        &mut self,
        share_id: IrisProfileId,
    ) -> Vec<ShareShortcut> {
        let mut removed = Vec::new();
        self.share_shortcuts.retain(|shortcut| {
            if shortcut.share_id == share_id {
                removed.push(shortcut.clone());
                false
            } else {
                true
            }
        });
        removed
    }

    pub fn upsert_backup_target(&mut self, target: BackupTarget) -> bool {
        if let Some(existing) = self
            .backup_targets
            .iter_mut()
            .find(|existing| existing.id == target.id)
        {
            let last_sync = existing.last_sync.clone();
            let last_check = existing.last_check.clone();
            *existing = target;
            if existing.last_sync.is_none() {
                existing.last_sync = last_sync;
            }
            if existing.last_check.is_none() {
                existing.last_check = last_check;
            }
            false
        } else {
            self.backup_targets.push(target);
            true
        }
    }

    pub fn remove_backup_target(&mut self, target_id: &str) -> Option<BackupTarget> {
        let pos = self
            .backup_targets
            .iter()
            .position(|target| target.id == target_id)?;
        Some(self.backup_targets.remove(pos))
    }

    /// Load from path. Missing file → `Default::default()`.
    pub fn load_or_default(path: impl AsRef<Path>) -> Result<Self, ConfigError> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(path)?;
        let mut parsed: Self =
            toml::from_str(&raw).map_err(|e| ConfigError::Parse(e.to_string()))?;
        if parsed.schema_version != CONFIG_SCHEMA_VERSION {
            return Err(ConfigError::SchemaMismatch {
                found: parsed.schema_version,
                expected: CONFIG_SCHEMA_VERSION,
            });
        }
        let adopted_root_scope_id = if let Some(account) = parsed.profile.as_mut() {
            let adopted = account.adopt_single_roster_profile_id();
            if !adopted {
                account.sync_app_keys_from_profile();
            }
            adopted.then(|| account.root_scope_id())
        } else {
            None
        };
        if let Some(root_scope_id) = adopted_root_scope_id {
            parsed.sync_primary_drive_scope(root_scope_id);
        }
        Ok(parsed)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), ConfigError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut config = self.clone();
        if path.exists()
            && let Ok(existing) = Self::load_or_default(path)
        {
            config.preserve_newer_app_key_roots(&existing);
        }
        let raw =
            toml::to_string_pretty(&config).map_err(|e| ConfigError::Serialize(e.to_string()))?;
        fs::write(path, raw)?;
        Ok(())
    }

    fn preserve_newer_app_key_roots(&mut self, existing: &Self) {
        for drive in &mut self.drives {
            let Some(existing_drive) = existing.drive(&drive.drive_id) else {
                continue;
            };
            for (app_key_pubkey, existing_root) in &existing_drive.app_key_roots {
                let keep_existing = drive
                    .app_key_roots
                    .get(app_key_pubkey)
                    .is_none_or(|incoming| existing_root.is_newer_than(incoming));
                if keep_existing {
                    drive
                        .app_key_roots
                        .insert(app_key_pubkey.clone(), existing_root.clone());
                }
            }
            if let Some(account) = self.profile.as_ref()
                && let Some(root) = drive.app_key_roots.get(&account.app_key_pubkey)
            {
                drive.last_root_cid = Some(root.root_cid.clone());
            }
        }
    }

    pub fn sync_primary_drive_scope(&mut self, root_scope_id: String) {
        if let Some(drive) = self
            .drives
            .iter_mut()
            .find(|drive| drive.drive_id == crate::daemon::PRIMARY_DRIVE_ID)
        {
            drive.root_scope_id = root_scope_id;
        } else {
            self.upsert_drive(Drive::primary(root_scope_id));
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupTargetKind {
    Blossom,
    Fips,
    Filesystem,
    Lmdb,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupTarget {
    pub id: String,
    pub kind: BackupTargetKind,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<BackupTargetSync>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_check: Option<BackupTargetCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupTargetSync {
    pub state: String,
    pub root_cid: String,
    pub synced_at: i64,
    pub total_hashes: usize,
    pub uploaded: usize,
    pub already_present: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupTargetCheck {
    pub state: String,
    pub root_cid: String,
    pub checked_at: i64,
    pub total_hashes: usize,
    pub sample_size: usize,
    pub sampled_hashes: usize,
    pub present: usize,
    pub missing: usize,
    pub unknown: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_bytes_per_second: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn default_true() -> bool {
    true
}

impl DriveRole {
    #[must_use]
    pub fn can_write(self) -> bool {
        matches!(self, DriveRole::Owner | DriveRole::Editor)
    }
}

/// A drive is one logical mount-point. The user's primary "My Drive" is
/// stored as `drive_id = "main", role = Owner, root_scope_id = IrisProfileId`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Drive {
    pub root_scope_id: String,
    pub drive_id: String,
    pub display_name: String,
    pub role: DriveRole,
    /// Per-AppKey drive roots, keyed by `app_key_pubkey` (hex). Every
    /// authorized app install publishes its own root tree; the merged view
    /// is computed causally across all entries, with timestamp ordering
    /// for legacy roots (see [`crate::merge::merge_drives`]).
    /// Single-AppKey installs carry exactly one entry here.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub app_key_roots: BTreeMap<String, AppKeyRootRef>,
    /// This `AppKey`'s most-recent root CID as a flat scalar for status surfaces
    /// that do not need the full per-AppKey root map. New code should read
    /// `app_key_roots[my_app_key_pubkey].root_cid`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_root_cid: Option<String>,
    /// Symmetric key for encrypted drives, hex-encoded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_hex: Option<String>,
}

/// One `AppKey`'s contribution to a drive. Each authorized app install
/// publishes its own complete root tree. Causal fields are optional
/// for legacy roots; new roots fill them from `.hashtree/root.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppKeyRootRef {
    /// htree root CID the `AppKey` most recently published.
    pub root_cid: String,
    /// Unix-seconds publication time. Used for display and as the
    /// deterministic ordering when a legacy root has no causal fields.
    pub published_at: i64,
    /// DCK generation this root was sealed with. Lets readers detect
    /// stale `AppKey` roots that pre-date a rotation.
    pub dck_generation: u64,
    /// Monotonic per-AppKey sequence for this drive. `0` means legacy
    /// root with unknown causality.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub app_key_seq: u64,
    /// Direct parent roots this snapshot replaces or incorporates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<RootParent>,
    /// Latest roots this `AppKey` had observed when publishing this
    /// snapshot, keyed by `AppKey` pubkey.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub observed: BTreeMap<String, RootObservation>,
    /// Local bookkeeping root that can be used as base state but should
    /// not be announced as this `AppKey`'s own edit.
    #[serde(default, skip_serializing_if = "is_false")]
    pub local_only: bool,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(value: &u64) -> bool {
    *value == 0
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(value: &bool) -> bool {
    !*value
}

impl AppKeyRootRef {
    #[must_use]
    pub fn legacy(root_cid: impl Into<String>, published_at: i64, dck_generation: u64) -> Self {
        Self {
            root_cid: root_cid.into(),
            published_at,
            dck_generation,
            app_key_seq: 0,
            parents: Vec::new(),
            observed: BTreeMap::new(),
            local_only: false,
        }
    }

    #[must_use]
    pub fn from_meta(root_cid: impl Into<String>, published_at: i64, meta: &DriveRootMeta) -> Self {
        Self {
            root_cid: root_cid.into(),
            published_at,
            dck_generation: meta.dck_generation,
            app_key_seq: meta.app_key_seq,
            parents: meta.parents.clone(),
            observed: meta.observed.clone(),
            local_only: meta.local_only,
        }
    }

    #[must_use]
    pub fn is_newer_than(&self, other: &Self) -> bool {
        if self.app_key_seq > 0 || other.app_key_seq > 0 {
            return self.app_key_seq > other.app_key_seq;
        }
        self.published_at > other.published_at
    }
}

impl Drive {
    pub fn primary(root_scope_id: impl Into<String>) -> Self {
        Self {
            root_scope_id: root_scope_id.into(),
            drive_id: "main".into(),
            display_name: "My Drive".into(),
            role: DriveRole::Owner,
            app_key_roots: BTreeMap::new(),
            last_root_cid: None,
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
    fn default_blossom_server_is_upload_iris_to() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.blossom_servers, vec!["https://upload.iris.to"]);
    }

    #[test]
    fn default_relays_match_hashtree_defaults() {
        let cfg = AppConfig::default();
        assert_eq!(cfg.relays, hashtree_config::DEFAULT_RELAYS);
    }

    #[test]
    fn default_enables_local_nhash_resolver() {
        let cfg = AppConfig::default();
        assert!(cfg.local_nhash_resolver_enabled);
    }

    #[test]
    fn user_profile_requires_username_before_photo() {
        assert_eq!(
            UserProfile::from_optional(None, Some("/tmp/avatar.png")),
            None
        );
        assert_eq!(
            UserProfile::from_optional(Some("  ada  "), Some("  /tmp/avatar.png  ")),
            Some(UserProfile {
                username: "ada".into(),
                photo_path: Some("/tmp/avatar.png".into()),
            })
        );
    }

    #[test]
    fn stale_drive_fields_are_rejected() {
        let raw = format!(
            r#"
schema_version = {CONFIG_SCHEMA_VERSION}
relays = ["wss://relay.example"]
blossom_servers = ["https://upload.example"]

[[drives]]
root_scope_id = "018fd1a3-8e37-7dc4-bd29-0cf06bdbe3f0"
drive_id = "main"
display_name = "My Drive"
role = "owner"
working_dir = "/tmp/Iris Drive"
"#
        );
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, raw).unwrap();
        let error = AppConfig::load_or_default(&path).unwrap_err();
        assert!(error.to_string().contains("unknown field `working_dir`"));
    }

    #[test]
    fn stale_account_config_field_is_rejected() {
        let raw = format!(
            r#"
schema_version = {CONFIG_SCHEMA_VERSION}

[account]
profile_id = "018fd1a3-8e37-7dc4-bd29-0cf06bdbe3f0"
app_key_pubkey = "device-a"
app_key_link_secret = "link-secret"
authorization_state = "authorized"
"#
        );
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, raw).unwrap();
        let error = AppConfig::load_or_default(&path).unwrap_err();
        assert!(error.to_string().contains("unknown field `account`"));
    }

    #[test]
    fn missing_blossom_servers_field_loads_default_server() {
        let raw = format!("schema_version = {CONFIG_SCHEMA_VERSION}\n");
        let cfg: AppConfig = toml::from_str(&raw).unwrap();
        assert_eq!(cfg.blossom_servers, vec!["https://upload.iris.to"]);
    }

    #[test]
    fn missing_local_nhash_resolver_field_loads_enabled() {
        let raw = format!("schema_version = {CONFIG_SCHEMA_VERSION}\n");
        let cfg: AppConfig = toml::from_str(&raw).unwrap();
        assert!(cfg.local_nhash_resolver_enabled);
    }

    #[test]
    fn legacy_app_key_root_ref_defaults_causal_fields() {
        let raw = r#"
root_cid = "cid-legacy"
published_at = 1234
dck_generation = 1
"#;
        let root: AppKeyRootRef = toml::from_str(raw).unwrap();
        assert_eq!(root.root_cid, "cid-legacy");
        assert_eq!(root.published_at, 1234);
        assert_eq!(root.dck_generation, 1);
        assert_eq!(root.app_key_seq, 0);
        assert!(root.parents.is_empty());
        assert!(root.observed.is_empty());
        assert!(!root.local_only);
    }

    #[test]
    fn round_trip_through_toml() {
        let mut cfg = AppConfig::default();
        cfg.upsert_drive(Drive::primary(crate::IrisProfileId::new_v4().to_string()));
        cfg.upsert_drive(Drive {
            root_scope_id: crate::IrisProfileId::new_v4().to_string(),
            drive_id: "shared-photos".into(),
            display_name: "Photos from Alice".into(),
            role: DriveRole::Reader,
            app_key_roots: BTreeMap::new(),
            last_root_cid: Some("Q123abc".into()),
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
    fn app_keys_projection_is_hydrated_from_profile_roster_not_persisted() {
        let dir = tempdir().unwrap();
        let account = crate::profile::Profile::create(dir.path(), Some("Mac".into())).unwrap();
        let expected = account
            .state
            .app_keys
            .clone()
            .expect("created account has derived app keys");
        let mut cfg = AppConfig {
            profile: Some(account.state.clone()),
            ..AppConfig::default()
        };
        cfg.upsert_drive(Drive::primary(account.state.root_scope_id()));

        let path = dir.path().join("config.toml");
        cfg.save(&path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("[profile]"));
        assert!(!raw.contains("[account"));
        assert!(raw.contains("profile_roster_ops"));
        assert!(!raw.contains("[profile.app_keys]"));

        let loaded = AppConfig::load_or_default(&path).unwrap();
        let loaded_account = loaded.profile.expect("account persisted");
        assert_eq!(loaded_account.app_keys, Some(expected));
    }

    #[test]
    fn load_adopts_single_roster_profile_id_from_recovery_evidence() {
        let owner_dir = tempdir().unwrap();
        let owner =
            crate::profile::Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
        let phrase = crate::recovery_phrase::load_recovery_phrase(
            crate::paths::recovery_phrase_path_in(owner_dir.path()),
        )
        .unwrap();

        let restored_dir = tempdir().unwrap();
        let restored =
            crate::profile::Profile::restore(restored_dir.path(), &phrase, Some("Web".into()))
                .unwrap();
        let temporary_profile_id = restored.state.profile_id;
        let mut restored_state = restored.state.clone();
        restored_state.profile_roster_ops = owner.state.profile_roster_ops.clone();
        restored_state.app_keys = None;

        let mut cfg = AppConfig {
            profile: Some(restored_state),
            ..AppConfig::default()
        };
        cfg.upsert_drive(Drive::primary(temporary_profile_id.to_string()));
        let path = restored_dir.path().join("config.toml");
        cfg.save(&path).unwrap();

        let loaded = AppConfig::load_or_default(&path).unwrap();
        let loaded_state = loaded.profile.as_ref().expect("profile persisted");
        assert_eq!(loaded_state.profile_id, owner.state.profile_id);
        assert_ne!(loaded_state.profile_id, temporary_profile_id);
        assert!(loaded_state.app_keys.is_some());
        assert_eq!(
            loaded.drive("main").unwrap().root_scope_id,
            owner.state.root_scope_id()
        );
    }

    #[test]
    fn save_preserves_newer_app_key_root_already_on_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut newer = AppConfig {
            profile: Some(ProfileState {
                profile_id: crate::IrisProfileId::new_v4(),
                app_key_pubkey: "device-a".into(),
                profile_roster_ops: Vec::new(),
                app_key_link_secret: "link-secret".into(),
                authorization_state: crate::profile::AppKeyAuthorizationState::Authorized,
                app_key_label: None,
                app_keys: None,
                outbound_app_key_link_request: None,
                inbound_app_key_link_requests: Vec::new(),
            }),
            ..AppConfig::default()
        };
        let mut newer_drive = Drive::primary(crate::IrisProfileId::new_v4().to_string());
        newer_drive.app_key_roots.insert(
            "device-a".into(),
            AppKeyRootRef {
                root_cid: "newer".into(),
                published_at: 20,
                dck_generation: 1,
                app_key_seq: 7,
                parents: Vec::new(),
                observed: BTreeMap::new(),
                local_only: false,
            },
        );
        newer_drive.last_root_cid = Some("newer".into());
        newer.upsert_drive(newer_drive);
        newer.save(&path).unwrap();

        let mut stale = newer.clone();
        let drive = stale
            .drives
            .iter_mut()
            .find(|d| d.drive_id == "main")
            .unwrap();
        drive.app_key_roots.insert(
            "device-a".into(),
            AppKeyRootRef {
                root_cid: "stale".into(),
                published_at: 30,
                dck_generation: 1,
                app_key_seq: 6,
                parents: Vec::new(),
                observed: BTreeMap::new(),
                local_only: false,
            },
        );
        drive.last_root_cid = Some("stale".into());
        stale.save(&path).unwrap();

        let loaded = AppConfig::load_or_default(&path).unwrap();
        let drive = loaded.drive("main").unwrap();
        let root = drive.app_key_roots.get("device-a").unwrap();
        assert_eq!(root.root_cid, "newer");
        assert_eq!(root.app_key_seq, 7);
        assert_eq!(drive.last_root_cid.as_deref(), Some("newer"));
    }

    #[test]
    fn save_accepts_newer_app_key_root_over_disk() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        let mut older = AppConfig::default();
        let mut older_drive = Drive::primary(crate::IrisProfileId::new_v4().to_string());
        older_drive.app_key_roots.insert(
            "device-a".into(),
            AppKeyRootRef {
                root_cid: "older".into(),
                published_at: 10,
                dck_generation: 1,
                app_key_seq: 1,
                parents: Vec::new(),
                observed: BTreeMap::new(),
                local_only: false,
            },
        );
        older.upsert_drive(older_drive);
        older.save(&path).unwrap();

        let mut newer = older.clone();
        let drive = newer
            .drives
            .iter_mut()
            .find(|d| d.drive_id == "main")
            .unwrap();
        drive.app_key_roots.insert(
            "device-a".into(),
            AppKeyRootRef {
                root_cid: "newer".into(),
                published_at: 11,
                dck_generation: 1,
                app_key_seq: 2,
                parents: Vec::new(),
                observed: BTreeMap::new(),
                local_only: false,
            },
        );
        newer.save(&path).unwrap();

        let loaded = AppConfig::load_or_default(&path).unwrap();
        let root = loaded
            .drive("main")
            .unwrap()
            .app_key_roots
            .get("device-a")
            .unwrap();
        assert_eq!(root.root_cid, "newer");
        assert_eq!(root.app_key_seq, 2);
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
        let root_scope_id = crate::IrisProfileId::new_v4().to_string();
        assert!(cfg.upsert_drive(Drive::primary(root_scope_id.clone())));
        let mut updated = Drive::primary(root_scope_id);
        updated.display_name = "Renamed".into();
        assert!(!cfg.upsert_drive(updated)); // not new
        assert_eq!(cfg.drives.len(), 1);
        assert_eq!(cfg.drives[0].display_name, "Renamed");
    }

    #[test]
    fn remove_drive_returns_removed() {
        let mut cfg = AppConfig::default();
        let root_scope_id = crate::IrisProfileId::new_v4().to_string();
        cfg.upsert_drive(Drive::primary(root_scope_id.clone()));
        let removed = cfg.remove_drive("main").unwrap();
        assert_eq!(removed.root_scope_id, root_scope_id);
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
    fn primary_drive_uses_root_scope_named_main() {
        let root_scope_id = crate::IrisProfileId::new_v4().to_string();
        let d = Drive::primary(root_scope_id.clone());
        assert_eq!(d.drive_id, "main");
        assert_eq!(d.root_scope_id, root_scope_id);
        assert_eq!(d.role, DriveRole::Owner);
        assert_eq!(d.display_name, "My Drive");
    }
}
