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
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

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
    #[error("invalid device pubkey: {0}")]
    InvalidDevicePubkey(String),
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

pub const MAX_INBOUND_DEVICE_LINK_REQUESTS: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PendingDeviceLinkRequest {
    pub admin_device_pubkey: String,
    pub requested_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InboundDeviceLinkRequest {
    pub device_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub requested_at: u64,
}

/// Persisted account state. Lives inside `AppConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AccountState {
    pub owner_pubkey: String,
    pub device_pubkey: String,
    pub has_owner_signing_authority: bool,
    pub authorization_state: DeviceAuthorizationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_keys: Option<AppKeysSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outbound_device_link_request: Option<PendingDeviceLinkRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inbound_device_link_requests: Vec<InboundDeviceLinkRequest>,
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
        if self.authorization_state == DeviceAuthorizationState::Authorized {
            self.outbound_device_link_request = None;
        }
    }

    /// Adopt an incoming `AppKeys` snapshot. Returns the apply decision
    /// so callers can decide whether to log a change. Side-effect:
    /// `authorization_state` is recomputed.
    pub fn apply_app_keys(&mut self, incoming: AppKeysSnapshot) -> ApplyDecision {
        let decision = apply_snapshot(&mut self.app_keys, incoming);
        self.recompute_authorization();
        decision
    }

    pub fn queue_outbound_device_link_request(
        &mut self,
        admin_device_pubkey: String,
        requested_at: u64,
    ) -> Result<bool, AccountError> {
        if !is_pubkey_hex(&admin_device_pubkey) {
            return Err(AccountError::InvalidDevicePubkey(admin_device_pubkey));
        }
        if self.device_pubkey == admin_device_pubkey {
            return Ok(false);
        }
        let next = PendingDeviceLinkRequest {
            admin_device_pubkey,
            requested_at,
        };
        let changed = self.outbound_device_link_request.as_ref() != Some(&next);
        self.outbound_device_link_request = Some(next);
        Ok(changed)
    }

    pub fn record_inbound_device_link_request(
        &mut self,
        owner_pubkey: &str,
        device_pubkey: &str,
        label: Option<String>,
        requested_at: u64,
    ) -> Result<bool, AccountError> {
        if owner_pubkey != self.owner_pubkey || !self.has_owner_signing_authority {
            return Ok(false);
        }
        if !is_pubkey_hex(device_pubkey) {
            return Err(AccountError::InvalidDevicePubkey(device_pubkey.to_string()));
        }
        if device_pubkey == self.device_pubkey
            || self
                .app_keys
                .as_ref()
                .is_some_and(|snap| snap.contains(device_pubkey))
        {
            self.inbound_device_link_requests
                .retain(|request| request.device_pubkey != device_pubkey);
            return Ok(false);
        }

        let label = label.and_then(|label| {
            let trimmed = label.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });
        let mut changed = false;
        if let Some(existing) = self
            .inbound_device_link_requests
            .iter_mut()
            .find(|request| request.device_pubkey == device_pubkey)
        {
            let next_requested_at = existing.requested_at.max(requested_at);
            if existing.requested_at != next_requested_at || existing.label != label {
                existing.requested_at = next_requested_at;
                existing.label = label;
                changed = true;
            }
        } else {
            self.inbound_device_link_requests
                .push(InboundDeviceLinkRequest {
                    device_pubkey: device_pubkey.to_string(),
                    label,
                    requested_at,
                });
            changed = true;
        }

        if self.inbound_device_link_requests.len() > MAX_INBOUND_DEVICE_LINK_REQUESTS {
            self.inbound_device_link_requests
                .sort_by_key(|request| request.requested_at);
            while self.inbound_device_link_requests.len() > MAX_INBOUND_DEVICE_LINK_REQUESTS {
                self.inbound_device_link_requests.remove(0);
            }
            changed = true;
        }
        self.inbound_device_link_requests
            .sort_by(|left, right| left.device_pubkey.cmp(&right.device_pubkey));
        Ok(changed)
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
        let device_label = resolve_device_label(device_label, &device.pubkey_hex());

        let mut state = AccountState {
            owner_pubkey: owner.pubkey_hex(),
            device_pubkey: device.pubkey_hex(),
            has_owner_signing_authority: true,
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label: device_label.clone(),
            app_keys: None,
            outbound_device_link_request: None,
            inbound_device_link_requests: Vec::new(),
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
        let device_label = resolve_device_label(device_label, &device.pubkey_hex());

        let state = AccountState {
            owner_pubkey: owner.pubkey_hex(),
            device_pubkey: device.pubkey_hex(),
            has_owner_signing_authority: true,
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label,
            app_keys: None,
            outbound_device_link_request: None,
            inbound_device_link_requests: Vec::new(),
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
        let device_label = resolve_device_label(device_label, &device.pubkey_hex());

        let state = AccountState {
            owner_pubkey: owner_pubkey_hex,
            device_pubkey: device.pubkey_hex(),
            has_owner_signing_authority: false,
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label,
            app_keys: None,
            outbound_device_link_request: None,
            inbound_device_link_requests: Vec::new(),
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
        device_pubkey_hex: &str,
        label: Option<String>,
    ) -> Result<&AppKeysSnapshot, AccountError> {
        if !self.state.can_manage_devices() {
            return Err(AccountError::NoOwnerAuthority);
        }
        if let Some(snap) = &self.state.app_keys
            && snap.contains(device_pubkey_hex)
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
            pubkey: device_pubkey_hex.to_string(),
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
        self.state
            .inbound_device_link_requests
            .retain(|request| request.device_pubkey != device_pubkey_hex);
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

fn resolve_device_label(label: Option<String>, pubkey_hex: &str) -> Option<String> {
    let hostname = detected_hostname();
    resolve_device_label_with_hostname(label, hostname.as_deref(), pubkey_hex)
}

fn resolve_device_label_with_hostname(
    label: Option<String>,
    hostname: Option<&str>,
    pubkey_hex: &str,
) -> Option<String> {
    label
        .and_then(|value| normalize_device_label(&value))
        .or_else(|| hostname.and_then(normalize_hostname_label))
        .or_else(|| Some(default_device_label_for_pubkey(pubkey_hex)))
}

fn detected_hostname() -> Option<String> {
    ["IRIS_DRIVE_DEVICE_NAME", "COMPUTERNAME", "HOSTNAME"]
        .iter()
        .find_map(|key| {
            env::var(key)
                .ok()
                .and_then(|value| normalize_hostname_label(&value))
        })
        .or_else(|| {
            Command::new("hostname")
                .output()
                .ok()
                .filter(|output| output.status.success())
                .and_then(|output| String::from_utf8(output.stdout).ok())
                .and_then(|value| normalize_hostname_label(&value))
        })
}

fn normalize_hostname_label(hostname: &str) -> Option<String> {
    let first_label = hostname
        .trim()
        .trim_matches('.')
        .split('.')
        .find_map(normalize_device_label)?;
    let lower = first_label.to_ascii_lowercase();
    if lower == "localhost" || looks_like_generated_hex_label(&lower) {
        return None;
    }
    Some(first_label)
}

fn normalize_device_label(value: &str) -> Option<String> {
    let normalized = value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(['.', '-'])
        .trim()
        .to_string();
    if normalized.is_empty() {
        return None;
    }
    Some(normalized.chars().take(64).collect())
}

fn default_device_label_for_pubkey(pubkey_hex: &str) -> String {
    let suffix = pubkey_hex.chars().take(8).collect::<String>();
    if suffix.is_empty() {
        "device".to_string()
    } else {
        format!("device {suffix}")
    }
}

fn looks_like_generated_hex_label(value: &str) -> bool {
    (12..=64).contains(&value.len()) && value.chars().all(|ch| ch.is_ascii_hexdigit())
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
mod tests;
