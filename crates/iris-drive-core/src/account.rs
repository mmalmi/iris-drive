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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{Keys, PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::app_keys::{AppKeysSnapshot, ApplyDecision, DeviceEntry, apply_snapshot};
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
    #[error("no AppKeys snapshot yet")]
    NoCurrentSnapshot,
    #[error("no DCK wrap for this device (revoked or never authorized)")]
    NoWrapForThisDevice,
    #[error("failed to wrap DCK: {0}")]
    Wrap(String),
    #[error("failed to unwrap DCK: {0}")]
    Unwrap(String),
    #[error("decrypted DCK has wrong length: expected 32 bytes, got {0}")]
    InvalidDckLength(usize),
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
        matches!(
            self.authorization_state,
            DeviceAuthorizationState::Authorized
        )
    }

    /// Can this device add/remove other devices in the roster?
    #[must_use]
    pub fn can_manage_devices(&self) -> bool {
        self.has_owner_signing_authority
    }

    /// Recompute `authorization_state` from the current `AppKeys` snapshot.
    pub fn recompute_authorization(&mut self) {
        self.authorization_state = match &self.app_keys {
            Some(snap) if snap.contains(&self.device_pubkey) => {
                DeviceAuthorizationState::Authorized
            }
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
        let devices = vec![DeviceEntry {
            pubkey: state.device_pubkey.clone(),
            added_at: now,
            label: device_label,
        }];
        let dck = generate_dck();
        let wraps = wrap_dck_for_devices(owner.keys().secret_key(), &devices, &dck)?;
        let snap = AppKeysSnapshot {
            owner_pubkey: state.owner_pubkey.clone(),
            created_at: now,
            devices,
            dck_generation: 1,
            wrapped_dck: wraps,
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

    /// Approve a new device by appending it to the `AppKeys` snapshot
    /// and rotating the DCK so the new device gets a fresh wrap.
    /// Bumps `created_at` and `dck_generation`. Callers should fan the
    /// new snapshot out over Nostr.
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
        let owner = self
            .owner_key
            .as_ref()
            .ok_or(AccountError::NoOwnerAuthority)?;
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
        let dck = generate_dck();
        let wraps = wrap_dck_for_devices(owner.keys().secret_key(), &devices, &dck)?;
        let next_gen = self
            .state
            .app_keys
            .as_ref()
            .map_or(1, |s| s.dck_generation + 1);
        let new_snap = AppKeysSnapshot {
            owner_pubkey: self.state.owner_pubkey.clone(),
            created_at: now,
            devices,
            dck_generation: next_gen,
            wrapped_dck: wraps,
        };
        self.state.apply_app_keys(new_snap);
        Ok(self.state.app_keys.as_ref().expect("just applied"))
    }

    /// Revoke a device from the roster and rotate the DCK so the
    /// revoked device cannot decrypt any subsequent content. Bumps
    /// `created_at` and `dck_generation`.
    pub fn revoke_device(
        &mut self,
        device_pubkey_hex: &str,
    ) -> Result<&AppKeysSnapshot, AccountError> {
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
        let owner = self
            .owner_key
            .as_ref()
            .ok_or(AccountError::NoOwnerAuthority)?;
        let now = next_local_timestamp(self.state.app_keys.as_ref());
        let devices: Vec<DeviceEntry> = snap
            .devices
            .iter()
            .filter(|d| d.pubkey != device_pubkey_hex)
            .cloned()
            .collect();
        let dck = generate_dck();
        let wraps = wrap_dck_for_devices(owner.keys().secret_key(), &devices, &dck)?;
        let next_gen = snap.dck_generation + 1;
        let new_snap = AppKeysSnapshot {
            owner_pubkey: self.state.owner_pubkey.clone(),
            created_at: now,
            devices,
            dck_generation: next_gen,
            wrapped_dck: wraps,
        };
        self.state.apply_app_keys(new_snap);
        Ok(self.state.app_keys.as_ref().expect("just applied"))
    }

    /// Rotate the DCK without changing the device roster. Useful for
    /// periodic key freshness ("rotate weekly even with no membership
    /// churn"). Owner-only.
    pub fn rotate_dck(&mut self) -> Result<&AppKeysSnapshot, AccountError> {
        if !self.state.can_manage_devices() {
            return Err(AccountError::NoOwnerAuthority);
        }
        let owner = self
            .owner_key
            .as_ref()
            .ok_or(AccountError::NoOwnerAuthority)?;
        let snap = self
            .state
            .app_keys
            .as_ref()
            .ok_or(AccountError::NoCurrentSnapshot)?;
        let devices = snap.devices.clone();
        let next_gen = snap.dck_generation + 1;
        let dck = generate_dck();
        let wraps = wrap_dck_for_devices(owner.keys().secret_key(), &devices, &dck)?;
        let now = next_local_timestamp(Some(snap));
        let new_snap = AppKeysSnapshot {
            owner_pubkey: self.state.owner_pubkey.clone(),
            created_at: now,
            devices,
            dck_generation: next_gen,
            wrapped_dck: wraps,
        };
        self.state.apply_app_keys(new_snap);
        Ok(self.state.app_keys.as_ref().expect("just applied"))
    }

    /// Decrypt this device's DCK wrap from the current snapshot. Errors
    /// with `NoWrapForThisDevice` if the device has been revoked or
    /// never authorized.
    pub fn current_dck(&self) -> Result<[u8; 32], AccountError> {
        let snap = self
            .state
            .app_keys
            .as_ref()
            .ok_or(AccountError::NoCurrentSnapshot)?;
        let wrap = snap
            .wrapped_dck
            .get(&self.state.device_pubkey)
            .ok_or(AccountError::NoWrapForThisDevice)?;
        let owner_pk = PublicKey::from_hex(&self.state.owner_pubkey)
            .map_err(|e| AccountError::InvalidOwnerPubkey(e.to_string()))?;
        let bytes = nip44::decrypt_to_bytes(self.device.keys().secret_key(), &owner_pk, wrap)
            .map_err(|e| AccountError::Unwrap(e.to_string()))?;
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| AccountError::InvalidDckLength(bytes.len()))?;
        Ok(arr)
    }
}

fn generate_dck() -> [u8; 32] {
    // Fresh 32 random bytes via nostr-sdk's RNG (no extra deps).
    let keys = Keys::generate();
    let mut out = [0u8; 32];
    out.copy_from_slice(keys.secret_key().as_secret_bytes());
    out
}

fn wrap_dck_for_devices(
    owner_secret: &SecretKey,
    devices: &[DeviceEntry],
    dck: &[u8; 32],
) -> Result<BTreeMap<String, String>, AccountError> {
    let mut wraps = BTreeMap::new();
    for d in devices {
        let pk = PublicKey::from_hex(&d.pubkey)
            .map_err(|e| AccountError::InvalidOwnerPubkey(e.to_string()))?;
        let ct = nip44::encrypt(owner_secret, &pk, dck.as_slice(), Nip44Version::V2)
            .map_err(|e| AccountError::Wrap(e.to_string()))?;
        wraps.insert(d.pubkey.clone(), ct);
    }
    Ok(wraps)
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

    /// Helper: produce a valid secp256k1 x-only pubkey hex for tests.
    /// Random fake hex strings often fail NIP-44 because only ~half of
    /// 32-byte values lie on the curve.
    fn fresh_device_pubkey() -> String {
        Keys::generate().public_key().to_hex()
    }

    #[test]
    fn approve_adds_device_to_roster() {
        let dir = tempdir().unwrap();
        let mut acct = Account::create(dir.path(), None).unwrap();
        let new_device = fresh_device_pubkey();
        let snap = acct
            .approve_device(new_device.clone(), Some("phone".into()))
            .unwrap();
        assert_eq!(snap.devices.len(), 2);
        assert!(snap.contains(&new_device));
    }

    #[test]
    fn approve_without_owner_authority_errors() {
        let dir = tempdir().unwrap();
        // Use a real x-only pubkey hex; the test only ever fails on the authority
        // check before reaching crypto, so this is fine.
        let owner = fresh_device_pubkey();
        let mut acct = Account::link(dir.path(), owner, None).unwrap();
        match acct.approve_device(fresh_device_pubkey(), None) {
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
        let target = fresh_device_pubkey();
        acct.approve_device(target.clone(), None).unwrap();
        let snap = acct.revoke_device(&target).unwrap();
        assert!(!snap.contains(&target));
    }

    #[test]
    fn revoke_missing_device_errors() {
        let dir = tempdir().unwrap();
        let mut acct = Account::create(dir.path(), None).unwrap();
        // Pubkey is well-formed but not in the roster.
        let stranger = fresh_device_pubkey();
        match acct.revoke_device(&stranger) {
            Err(AccountError::DeviceNotInRoster) => {}
            _ => panic!("expected DeviceNotInRoster"),
        }
    }

    // ------------ DCK rotation / forward secrecy tests ------------

    #[test]
    fn create_seeds_dck_generation_one_with_self_wrap() {
        let dir = tempdir().unwrap();
        let acct = Account::create(dir.path(), None).unwrap();
        let snap = acct.state.app_keys.as_ref().unwrap();
        assert_eq!(snap.dck_generation, 1);
        // One wrap, for the current device.
        assert_eq!(snap.wrapped_dck.len(), 1);
        assert!(snap.wrapped_dck.contains_key(&acct.state.device_pubkey));
    }

    #[test]
    fn current_dck_is_decryptable_by_owner_device() {
        let dir = tempdir().unwrap();
        let acct = Account::create(dir.path(), None).unwrap();
        let dck = acct.current_dck().unwrap();
        assert_eq!(dck.len(), 32);
        // Two reads return same key (state is deterministic).
        let dck2 = acct.current_dck().unwrap();
        assert_eq!(dck, dck2);
    }

    #[test]
    fn approve_rotates_dck_generation_and_wraps_to_all_devices() {
        let dir = tempdir().unwrap();
        let mut acct = Account::create(dir.path(), None).unwrap();
        let gen_before = acct.state.app_keys.as_ref().unwrap().dck_generation;
        let new_device = fresh_device_pubkey();
        let snap = acct
            .approve_device(new_device.clone(), Some("phone".into()))
            .unwrap();
        assert!(snap.dck_generation > gen_before);
        // Every authorized device has a wrap.
        assert_eq!(snap.wrapped_dck.len(), snap.devices.len());
        for d in &snap.devices {
            assert!(snap.wrapped_dck.contains_key(&d.pubkey));
        }
    }

    #[test]
    fn revoke_rotates_dck_and_drops_revoked_device_wrap() {
        let dir = tempdir().unwrap();
        let mut acct = Account::create(dir.path(), None).unwrap();
        let target = fresh_device_pubkey();
        let own_device_pubkey = acct.state.device_pubkey.clone();
        acct.approve_device(target.clone(), None).unwrap();
        let gen_before = acct.state.app_keys.as_ref().unwrap().dck_generation;
        let snap = acct.revoke_device(&target).unwrap();
        assert!(snap.dck_generation > gen_before);
        // Revoked device no longer has a wrap.
        assert!(!snap.wrapped_dck.contains_key(&target));
        // Remaining device(s) still have wraps.
        assert!(snap.wrapped_dck.contains_key(&own_device_pubkey));
    }

    #[test]
    fn dck_changes_after_rotation() {
        let dir = tempdir().unwrap();
        let mut acct = Account::create(dir.path(), None).unwrap();
        let dck_before = acct.current_dck().unwrap();
        acct.rotate_dck().unwrap();
        let dck_after = acct.current_dck().unwrap();
        assert_ne!(dck_before, dck_after);
    }

    #[test]
    fn rotate_dck_preserves_roster() {
        let dir = tempdir().unwrap();
        let mut acct = Account::create(dir.path(), None).unwrap();
        let new_device = fresh_device_pubkey();
        acct.approve_device(new_device.clone(), None).unwrap();
        let devices_before: Vec<_> = acct
            .state
            .app_keys
            .as_ref()
            .unwrap()
            .devices
            .iter()
            .map(|d| d.pubkey.clone())
            .collect();
        acct.rotate_dck().unwrap();
        let devices_after: Vec<_> = acct
            .state
            .app_keys
            .as_ref()
            .unwrap()
            .devices
            .iter()
            .map(|d| d.pubkey.clone())
            .collect();
        assert_eq!(devices_before, devices_after);
        // Both devices still have a wrap for the new DCK.
        for d in &devices_after {
            assert!(
                acct.state
                    .app_keys
                    .as_ref()
                    .unwrap()
                    .wrapped_dck
                    .contains_key(d)
            );
        }
    }

    #[test]
    fn rotate_dck_without_owner_authority_errors() {
        let dir = tempdir().unwrap();
        let owner = fresh_device_pubkey();
        let mut acct = Account::link(dir.path(), owner, None).unwrap();
        match acct.rotate_dck() {
            Err(AccountError::NoOwnerAuthority) => {}
            other => panic!("expected NoOwnerAuthority, got {:?}", other.is_ok()),
        }
    }

    #[test]
    fn current_dck_without_snapshot_errors() {
        let dir = tempdir().unwrap();
        let owner = fresh_device_pubkey();
        let acct = Account::link(dir.path(), owner, None).unwrap();
        match acct.current_dck() {
            Err(AccountError::NoCurrentSnapshot) => {}
            other => panic!("expected NoCurrentSnapshot, got {:?}", other.is_ok()),
        }
    }

    #[test]
    fn linked_device_with_approved_wrap_decrypts_same_dck_as_owner() {
        // This is the end-to-end crypto test: owner creates,
        // owner approves a *real* device keypair, the device then
        // independently decrypts its wrap and recovers the same DCK
        // the owner has.
        let owner_dir = tempdir().unwrap();
        let mut owner_acct = Account::create(owner_dir.path(), None).unwrap();

        // Manually create a "linked device" keypair we control end-to-end.
        let linked_dir = tempdir().unwrap();
        let linked_device = DeviceIdentity::generate(linked_dir.path().join("key"));
        linked_device.save().unwrap();
        let linked_pubkey = linked_device.pubkey_hex();

        // Owner approves the device's pubkey.
        owner_acct
            .approve_device(linked_pubkey.clone(), Some("phone".into()))
            .unwrap();
        let owner_dck = owner_acct.current_dck().unwrap();

        // Reconstruct an Account from the linked device's perspective:
        // device key is the one we generated; AccountState mirrors what
        // the device would see after pulling the latest snapshot.
        let snapshot_for_linked = owner_acct.state.app_keys.clone();
        let linked_state = AccountState {
            owner_pubkey: owner_acct.state.owner_pubkey.clone(),
            device_pubkey: linked_pubkey.clone(),
            has_owner_signing_authority: false,
            authorization_state: DeviceAuthorizationState::Authorized,
            device_label: Some("phone".into()),
            app_keys: snapshot_for_linked,
        };
        let linked_acct = Account {
            state: linked_state,
            device: linked_device,
            owner_key: None,
        };

        let linked_dck = linked_acct.current_dck().unwrap();
        assert_eq!(
            owner_dck, linked_dck,
            "linked device must derive the same DCK the owner does"
        );
    }

    #[test]
    fn revoked_device_cannot_decrypt_new_dck() {
        // Owner approves linked device, sees a DCK, then revokes it.
        // After revoke, the linked device should fail current_dck()
        // because its wrap is no longer present.
        let owner_dir = tempdir().unwrap();
        let mut owner_acct = Account::create(owner_dir.path(), None).unwrap();
        let linked_dir = tempdir().unwrap();
        let linked_device = DeviceIdentity::generate(linked_dir.path().join("key"));
        linked_device.save().unwrap();
        let linked_pubkey = linked_device.pubkey_hex();

        owner_acct
            .approve_device(linked_pubkey.clone(), None)
            .unwrap();
        owner_acct.revoke_device(&linked_pubkey).unwrap();

        let linked_state = AccountState {
            owner_pubkey: owner_acct.state.owner_pubkey.clone(),
            device_pubkey: linked_pubkey,
            has_owner_signing_authority: false,
            authorization_state: DeviceAuthorizationState::Revoked,
            device_label: None,
            app_keys: owner_acct.state.app_keys.clone(),
        };
        let linked_acct = Account {
            state: linked_state,
            device: linked_device,
            owner_key: None,
        };
        match linked_acct.current_dck() {
            Err(AccountError::NoWrapForThisDevice) => {}
            other => panic!("expected NoWrapForThisDevice, got {:?}", other.is_ok()),
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
            dck_generation: acct.state.app_keys.as_ref().unwrap().dck_generation + 1,
            wrapped_dck: BTreeMap::new(),
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
