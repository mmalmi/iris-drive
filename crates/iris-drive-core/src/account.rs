//! Account state machine.
//!
//! Wraps the user's identity model — owner pubkey, this device's
//! identity, whether this device can sign the `AppKeys` roster, and the
//! current authorization state.
//!
//! Three creation paths mirror the iris-chat-rs onboarding flows:
//!
//! 1. **Create** — fresh owner key + fresh device key. Single-device
//!    default; the install has owner signing authority.
//! 2. **Restore** — import an existing owner `nsec` onto this device.
//!    Generate a fresh device key. Device has owner authority.
//! 3. **Link** — paste an owner npub. Generate a fresh device key.
//!    Device does **not** have owner authority and starts in
//!    `AwaitingApproval`. It must be approved by an owner-capable
//!    device before its drive root is honoured.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::app_keys::{apply_snapshot, ApplyDecision, AppKeysSnapshot, DeviceEntry};
use crate::identity::{DeviceIdentity, IdentityError, OwnerKey};
use crate::paths::{key_path_in, owner_key_path_in};

#[derive(Debug, Error)]
pub enum AccountError {
    #[error("identity: {0}")]
    Identity(#[from] IdentityError),
    #[error("invalid owner pubkey: {0}")]
    InvalidOwnerPubkey(String),
    #[error("this device does not have owner signing authority")]
    NoOwnerAuthority,
    #[error("device already authorized")]
    AlreadyAuthorized,
    #[error("device not in roster")]
    DeviceNotInRoster,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Per-device authorization status relative to the owner's `AppKeys` roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceAuthorizationState {
    /// This device is in the latest `AppKeys` snapshot.
    Authorized,
    /// This device is not yet in the latest `AppKeys` snapshot; the user
    /// must approve it from an owner-capable device.
    AwaitingApproval,
    /// This device was previously authorized and has since been removed.
    Revoked,
}

/// Persisted account state. Lives inside `AppConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccountState {
    pub owner_pubkey: String,
    pub device_pubkey: String,
    pub has_owner_signing_authority: bool,
    pub authorization_state: DeviceAuthorizationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_keys: Option<AppKeysSnapshot>,
}

impl AccountState {
    /// Has the latest `AppKeys` snapshot included this device?
    #[must_use]
    pub fn is_authorized(&self) -> bool {
        matches!(self.authorization_state, DeviceAuthorizationState::Authorized)
    }

    /// Can this device add/remove other devices in the roster?
    #[must_use]
    pub fn can_manage_devices(&self) -> bool {
        self.has_owner_signing_authority
    }

    /// Recompute `authorization_state` from the current `AppKeys` snapshot.
    pub fn recompute_authorization(&mut self) {
        self.authorization_state = match &self.app_keys {
            Some(snap) if snap.contains(&self.device_pubkey) => DeviceAuthorizationState::Authorized,
            Some(_) => {
                // Previously authorized → Revoked; never authorized → AwaitingApproval.
                match self.authorization_state {
                    DeviceAuthorizationState::Authorized => DeviceAuthorizationState::Revoked,
                    other => other,
                }
            }
            None => self.authorization_state,
        };
    }

    /// Adopt an incoming `AppKeys` snapshot. Returns the apply decision
    /// so callers can decide whether to log a change. Side-effect:
    /// `authorization_state` is recomputed.
    pub fn apply_app_keys(&mut self, incoming: AppKeysSnapshot) -> ApplyDecision {
        let decision = apply_snapshot(&mut self.app_keys, incoming);
        self.recompute_authorization();
        decision
    }
}

/// In-memory account: persisted state + the keypairs it references.
pub struct Account {
    pub state: AccountState,
    pub device: DeviceIdentity,
    pub owner_key: Option<OwnerKey>,
}

impl Account {
    /// **Create** flow — fresh owner + fresh device, both saved to the
    /// config dir. The device is auto-authorized via a self-signed
    /// single-entry `AppKeys` snapshot.
    pub fn create(config_dir: &Path, device_label: Option<String>) -> Result<Self, AccountError> {
        let device = DeviceIdentity::generate(key_path_in(config_dir));
        device.save()?;
        let owner = OwnerKey::generate(owner_key_path_in(config_dir));
        owner.save()?;

        let mut state = AccountState {
            owner_pubkey: owner.pubkey_hex(),
            device_pubkey: device.pubkey_hex(),
            has_owner_signing_authority: true,
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label: device_label.clone(),
            app_keys: None,
        };

        let now = current_unix_seconds();
        let snap = AppKeysSnapshot {
            owner_pubkey: state.owner_pubkey.clone(),
            created_at: now,
            devices: vec![DeviceEntry {
                pubkey: state.device_pubkey.clone(),
                added_at: now,
                label: device_label,
            }],
        };
        state.apply_app_keys(snap);

        Ok(Self {
            state,
            device,
            owner_key: Some(owner),
        })
    }

    /// **Restore** flow — import existing owner nsec, generate a fresh
    /// device. The device must still be added to the existing `AppKeys`
    /// (caller is responsible for issuing the approval if one is needed;
    /// `apply_app_keys` will compute authorization state once a snapshot
    /// arrives).
    pub fn restore(
        config_dir: &Path,
        owner_nsec: &str,
        device_label: Option<String>,
    ) -> Result<Self, AccountError> {
        let device = DeviceIdentity::generate(key_path_in(config_dir));
        device.save()?;
        let owner = OwnerKey::from_secret(owner_nsec, owner_key_path_in(config_dir))?;
        owner.save()?;

        let state = AccountState {
            owner_pubkey: owner.pubkey_hex(),
            device_pubkey: device.pubkey_hex(),
            has_owner_signing_authority: true,
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label,
            app_keys: None,
        };

        Ok(Self {
            state,
            device,
            owner_key: Some(owner),
        })
    }

    /// **Link** flow — generate a fresh device key, accept the user's
    /// owner-npub claim, start in `AwaitingApproval`. The owner must
    /// approve this device from another install before any drive
    /// publishes will be honoured.
    pub fn link(
        config_dir: &Path,
        owner_pubkey_hex: String,
        device_label: Option<String>,
    ) -> Result<Self, AccountError> {
        if !is_pubkey_hex(&owner_pubkey_hex) {
            return Err(AccountError::InvalidOwnerPubkey(owner_pubkey_hex));
        }
        let device = DeviceIdentity::generate(key_path_in(config_dir));
        device.save()?;

        let state = AccountState {
            owner_pubkey: owner_pubkey_hex,
            device_pubkey: device.pubkey_hex(),
            has_owner_signing_authority: false,
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label,
            app_keys: None,
        };

        Ok(Self {
            state,
            device,
            owner_key: None,
        })
    }

    /// Load an account from its config dir. Reconstructs the in-memory
    /// view from the persisted `AccountState` plus the on-disk key
    /// files. Errors if the device key is missing — caller should run a
    /// create/restore/link flow first.
    pub fn load(state: AccountState, config_dir: &Path) -> Result<Self, AccountError> {
        let device = DeviceIdentity::load(key_path_in(config_dir))?;
        let owner_key = if state.has_owner_signing_authority {
            Some(OwnerKey::load(owner_key_path_in(config_dir))?)
        } else {
            None
        };
        Ok(Self {
            state,
            device,
            owner_key,
        })
    }

    /// Approve a new device by appending it to the `AppKeys` snapshot and
    /// re-publishing under the owner key. Bumps `created_at` to now;
    /// callers should fan the new snapshot out over Nostr.
    pub fn approve_device(
        &mut self,
        device_pubkey_hex: String,
        label: Option<String>,
    ) -> Result<&AppKeysSnapshot, AccountError> {
        if !self.state.can_manage_devices() {
            return Err(AccountError::NoOwnerAuthority);
        }
        if let Some(snap) = &self.state.app_keys
            && snap.contains(&device_pubkey_hex)
        {
            return Err(AccountError::AlreadyAuthorized);
        }
        let now = next_local_timestamp(self.state.app_keys.as_ref());
        let mut devices = self
            .state
            .app_keys
            .as_ref()
            .map(|s| s.devices.clone())
            .unwrap_or_default();
        devices.push(DeviceEntry {
            pubkey: device_pubkey_hex,
            added_at: now,
            label,
        });
        let new_snap = AppKeysSnapshot {
            owner_pubkey: self.state.owner_pubkey.clone(),
            created_at: now,
            devices,
        };
        self.state.apply_app_keys(new_snap);
        Ok(self.state.app_keys.as_ref().expect("just applied"))
    }

    /// Revoke a device from the roster. Bumps `created_at` to now.
    pub fn revoke_device(&mut self, device_pubkey_hex: &str) -> Result<&AppKeysSnapshot, AccountError> {
        if !self.state.can_manage_devices() {
            return Err(AccountError::NoOwnerAuthority);
        }
        let snap = self
            .state
            .app_keys
            .as_ref()
            .ok_or(AccountError::DeviceNotInRoster)?;
        if !snap.contains(device_pubkey_hex) {
            return Err(AccountError::DeviceNotInRoster);
        }
        let now = next_local_timestamp(self.state.app_keys.as_ref());
        let devices: Vec<DeviceEntry> = snap
            .devices
            .iter()
            .filter(|d| d.pubkey != device_pubkey_hex)
            .cloned()
            .collect();
        let new_snap = AppKeysSnapshot {
            owner_pubkey: self.state.owner_pubkey.clone(),
            created_at: now,
            devices,
        };
        self.state.apply_app_keys(new_snap);
        Ok(self.state.app_keys.as_ref().expect("just applied"))
    }
}

fn current_unix_seconds() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Pick a `created_at` for a local mutation that is strictly greater
/// than the current snapshot's. Same-second merges are additive
/// (designed for remote-vs-remote races), so locally we need to bypass
/// them — otherwise rapid approve+revoke cycles would be no-ops.
fn next_local_timestamp(current: Option<&AppKeysSnapshot>) -> i64 {
    let now = current_unix_seconds();
    match current {
        Some(snap) if snap.created_at >= now => snap.created_at + 1,
        _ => now,
    }
}

fn is_pubkey_hex(s: &str) -> bool {
    s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// Convenience: load just the keypair paths under a given config dir.
#[must_use]
pub fn account_paths(config_dir: &Path) -> AccountPaths {
    AccountPaths {
        device_key: key_path_in(config_dir),
        owner_key: owner_key_path_in(config_dir),
    }
}

pub struct AccountPaths {
    pub device_key: PathBuf,
    pub owner_key: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_yields_owner_capable_authorized_account() {
        let dir = tempdir().unwrap();
        let acct = Account::create(dir.path(), Some("my-laptop".into())).unwrap();
        assert!(acct.state.has_owner_signing_authority);
        assert!(acct.state.is_authorized());
        assert!(acct.owner_key.is_some());
        // Both key files exist.
        assert!(dir.path().join("key").exists());
        assert!(dir.path().join("owner_key").exists());
        // AppKeys lists one device — this one.
        let snap = acct.state.app_keys.as_ref().unwrap();
        assert_eq!(snap.devices.len(), 1);
        assert_eq!(snap.devices[0].pubkey, acct.state.device_pubkey);
    }

    #[test]
    fn restore_uses_provided_owner_nsec() {
        let dir_a = tempdir().unwrap();
        let original = Account::create(dir_a.path(), None).unwrap();
        let original_owner_pubkey = original.state.owner_pubkey.clone();
        let nsec = original
            .owner_key
            .as_ref()
            .unwrap()
            .keys()
            .secret_key()
            .to_secret_hex();

        let dir_b = tempdir().unwrap();
        let restored = Account::restore(dir_b.path(), &nsec, None).unwrap();
        assert_eq!(restored.state.owner_pubkey, original_owner_pubkey);
        // Device key should differ from the original.
        assert_ne!(restored.state.device_pubkey, original.state.device_pubkey);
        assert!(restored.state.has_owner_signing_authority);
        // No AppKeys yet on this device (network would seed it).
        assert!(restored.state.app_keys.is_none());
    }

    #[test]
    fn link_starts_awaiting_approval_no_owner_key() {
        let dir = tempdir().unwrap();
        // Fake owner npub (64 hex chars).
        let owner = "ab".repeat(32);
        let acct = Account::link(dir.path(), owner.clone(), Some("phone".into())).unwrap();
        assert_eq!(acct.state.owner_pubkey, owner);
        assert!(!acct.state.has_owner_signing_authority);
        assert_eq!(
            acct.state.authorization_state,
            DeviceAuthorizationState::AwaitingApproval
        );
        assert!(acct.owner_key.is_none());
        // owner_key file does NOT exist on a linked install.
        assert!(!dir.path().join("owner_key").exists());
    }

    #[test]
    fn link_with_invalid_pubkey_errors() {
        let dir = tempdir().unwrap();
        let result = Account::link(dir.path(), "not-a-real-pubkey".into(), None);
        match result {
            Err(AccountError::InvalidOwnerPubkey(_)) => {}
            other => panic!("expected InvalidOwnerPubkey, got {:?}", other.is_ok()),
        }
    }

    #[test]
    fn approve_adds_device_to_roster() {
        let dir = tempdir().unwrap();
        let mut acct = Account::create(dir.path(), None).unwrap();
        let new_device = "ff".repeat(32);
        let snap = acct.approve_device(new_device.clone(), Some("phone".into())).unwrap();
        assert_eq!(snap.devices.len(), 2);
        assert!(snap.contains(&new_device));
    }

    #[test]
    fn approve_without_owner_authority_errors() {
        let dir = tempdir().unwrap();
        let owner = "ab".repeat(32);
        let mut acct = Account::link(dir.path(), owner, None).unwrap();
        match acct.approve_device("ff".repeat(32), None) {
            Err(AccountError::NoOwnerAuthority) => {}
            _ => panic!("expected NoOwnerAuthority"),
        }
    }

    #[test]
    fn approving_already_authorized_device_errors() {
        let dir = tempdir().unwrap();
        let mut acct = Account::create(dir.path(), None).unwrap();
        let current = acct.state.device_pubkey.clone();
        match acct.approve_device(current, None) {
            Err(AccountError::AlreadyAuthorized) => {}
            _ => panic!("expected AlreadyAuthorized"),
        }
    }

    #[test]
    fn revoke_removes_device_from_roster() {
        let dir = tempdir().unwrap();
        let mut acct = Account::create(dir.path(), None).unwrap();
        let target = "ff".repeat(32);
        acct.approve_device(target.clone(), None).unwrap();
        let snap = acct.revoke_device(&target).unwrap();
        assert!(!snap.contains(&target));
    }

    #[test]
    fn revoke_missing_device_errors() {
        let dir = tempdir().unwrap();
        let mut acct = Account::create(dir.path(), None).unwrap();
        match acct.revoke_device(&"00".repeat(32)) {
            Err(AccountError::DeviceNotInRoster) => {}
            _ => panic!("expected DeviceNotInRoster"),
        }
    }

    #[test]
    fn external_revocation_marks_state_revoked() {
        let dir = tempdir().unwrap();
        let mut acct = Account::create(dir.path(), None).unwrap();
        assert!(acct.state.is_authorized());
        // Pretend a new snapshot from owner removes this device.
        let new_snap = AppKeysSnapshot {
            owner_pubkey: acct.state.owner_pubkey.clone(),
            created_at: acct.state.app_keys.as_ref().unwrap().created_at + 1,
            devices: vec![DeviceEntry {
                pubkey: "ff".repeat(32),
                added_at: 0,
                label: None,
            }],
        };
        acct.state.apply_app_keys(new_snap);
        assert_eq!(
            acct.state.authorization_state,
            DeviceAuthorizationState::Revoked
        );
    }

    #[test]
    fn load_round_trips_account_state() {
        let dir = tempdir().unwrap();
        let created = Account::create(dir.path(), Some("desktop".into())).unwrap();
        let state = created.state.clone();
        let loaded = Account::load(state.clone(), dir.path()).unwrap();
        assert_eq!(loaded.state, state);
        assert_eq!(loaded.device.pubkey_hex(), created.device.pubkey_hex());
        assert!(loaded.owner_key.is_some());
    }

    #[test]
    fn load_for_linked_device_skips_owner_key() {
        let dir = tempdir().unwrap();
        let owner = "ab".repeat(32);
        let linked = Account::link(dir.path(), owner, None).unwrap();
        let state = linked.state.clone();
        let loaded = Account::load(state, dir.path()).unwrap();
        assert!(loaded.owner_key.is_none());
    }
}
