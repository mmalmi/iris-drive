//! Account state machine.
//!
//! Wraps the account roster model — stable account id, this device's
//! identity, whether this device is an admin in the current `AppKeys`
//! roster, and the current authorization state.
//!
//! Three creation paths mirror the iris-chat-rs onboarding flows:
//!
//! 1. **Create** — fresh device key. Single-device default; this device
//!    is the first admin and signs the first roster.
//! 2. **Restore** — import an existing admin-device `nsec`.
//! 3. **Link** — paste/scan an account/admin invite. Generate a fresh
//!    device key and wait in `AwaitingApproval` until an admin accepts it.

use std::collections::BTreeMap;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{JsonUtil, Keys, PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::app_keys::{
    AppKeysEventRecord, AppKeysSnapshot, ApplyDecision, DeviceEntry, DeviceRole, apply_snapshot,
};
use crate::config::{AppConfig, ConfigError};
use crate::identity::{DeviceIdentity, IdentityError, OwnerKey};
use crate::nostr_events::build_app_keys_event;
use crate::paths::{config_path_in, key_path_in, owner_key_path_in, sync_cache_path_in};

#[derive(Debug, Error)]
pub enum AccountError {
    #[error("identity: {0}")]
    Identity(#[from] IdentityError),
    #[error("invalid owner pubkey: {0}")]
    InvalidOwnerPubkey(String),
    #[error("invalid device pubkey: {0}")]
    InvalidDevicePubkey(String),
    #[error("this device is not an admin")]
    NoOwnerAuthority,
    #[error("device already authorized")]
    AlreadyAuthorized,
    #[error("device not in roster")]
    DeviceNotInRoster,
    #[error("cannot remove the last admin device")]
    CannotRemoveLastAdmin,
    #[error("no AppKeys snapshot yet")]
    NoCurrentSnapshot,
    #[error("no DCK wrap for this device (revoked or never authorized)")]
    NoWrapForThisDevice,
    #[error("failed to wrap DCK: {0}")]
    Wrap(String),
    #[error("failed to unwrap DCK: {0}")]
    Unwrap(String),
    #[error("failed to record signed roster event: {0}")]
    RosterEvent(String),
    #[error("decrypted DCK has wrong length: expected 32 bytes, got {0}")]
    InvalidDckLength(usize),
    #[error("config: {0}")]
    Config(#[from] ConfigError),
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
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub link_secret: String,
    pub requested_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InboundDeviceLinkRequest {
    pub device_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub link_secret: String,
    pub requested_at: u64,
}

/// Persisted account state. Lives inside `AppConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AccountState {
    /// Stable account id. New accounts use their first admin device pubkey.
    /// The name is kept for config/wire compatibility.
    pub owner_pubkey: String,
    pub device_pubkey: String,
    #[serde(default = "default_device_link_secret")]
    pub device_link_secret: String,
    /// Historical field name. In the current model this is true when the
    /// current device is an admin in the latest accepted roster.
    pub has_owner_signing_authority: bool,
    pub authorization_state: DeviceAuthorizationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_keys: Option<AppKeysSnapshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_keys_event: Option<AppKeysEventRecord>,
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
        self.app_keys
            .as_ref()
            .map_or(self.has_owner_signing_authority, |snap| {
                snap.is_admin(&self.device_pubkey)
            })
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
        self.has_owner_signing_authority = self
            .app_keys
            .as_ref()
            .map_or(self.has_owner_signing_authority, |snap| {
                snap.is_admin(&self.device_pubkey)
            });
        if self.authorization_state == DeviceAuthorizationState::Authorized {
            self.outbound_device_link_request = None;
        }
    }

    /// Adopt an incoming `AppKeys` snapshot. Returns the apply decision
    /// so callers can decide whether to log a change. Side-effect:
    /// `authorization_state` is recomputed.
    pub fn apply_app_keys(&mut self, incoming: AppKeysSnapshot) -> ApplyDecision {
        let decision = apply_snapshot(&mut self.app_keys, incoming);
        if decision == ApplyDecision::Merged {
            self.app_keys_event = None;
        }
        self.recompute_authorization();
        decision
    }

    pub fn apply_signed_app_keys(
        &mut self,
        incoming: AppKeysSnapshot,
        event: AppKeysEventRecord,
    ) -> ApplyDecision {
        let decision = self.apply_app_keys(incoming);
        match decision {
            ApplyDecision::Adopted | ApplyDecision::Replaced => {
                self.app_keys_event = Some(event);
            }
            ApplyDecision::Merged => {
                self.app_keys_event = None;
            }
            ApplyDecision::Rejected => {}
        }
        decision
    }

    pub fn queue_outbound_device_link_request(
        &mut self,
        admin_device_pubkey: String,
        link_secret: &str,
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
            link_secret: link_secret.trim().to_string(),
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
        link_secret: &str,
        requested_at: u64,
    ) -> Result<bool, AccountError> {
        if owner_pubkey != self.owner_pubkey || !self.can_manage_devices() {
            return Ok(false);
        }
        let link_secret = link_secret.trim();
        let expected_secret = self.device_link_secret.trim();
        if !expected_secret.is_empty() && link_secret != expected_secret {
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
            if existing.requested_at != next_requested_at
                || existing.label != label
                || existing.link_secret != link_secret
            {
                existing.requested_at = next_requested_at;
                existing.label = label;
                existing.link_secret = link_secret.to_string();
                changed = true;
            }
        } else {
            self.inbound_device_link_requests
                .push(InboundDeviceLinkRequest {
                    device_pubkey: device_pubkey.to_string(),
                    label,
                    link_secret: link_secret.to_string(),
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

    pub fn reject_inbound_device_link_request(
        &mut self,
        device_pubkey: &str,
    ) -> Result<bool, AccountError> {
        if !is_pubkey_hex(device_pubkey) {
            return Err(AccountError::InvalidDevicePubkey(device_pubkey.to_string()));
        }
        let before = self.inbound_device_link_requests.len();
        self.inbound_device_link_requests
            .retain(|request| request.device_pubkey != device_pubkey);
        Ok(before != self.inbound_device_link_requests.len())
    }

    pub fn reset_device_link_secret(&mut self) -> bool {
        let previous = self.device_link_secret.clone();
        self.device_link_secret = default_device_link_secret();
        let had_requests = !self.inbound_device_link_requests.is_empty();
        self.inbound_device_link_requests.clear();
        had_requests || self.device_link_secret != previous
    }
}

/// In-memory account: persisted state + the keypairs it references.
pub struct Account {
    pub state: AccountState,
    pub device: DeviceIdentity,
    /// Legacy compatibility slot. New accounts do not create or require a
    /// separate owner key; admin authority lives in the roster.
    pub owner_key: Option<OwnerKey>,
}

impl Account {
    /// **Create** flow — fresh device saved to the config dir. The device is
    /// auto-authorized as the first admin via a self-signed single-entry
    /// `AppKeys` snapshot.
    pub fn create(config_dir: &Path, device_label: Option<String>) -> Result<Self, AccountError> {
        let device = DeviceIdentity::generate(key_path_in(config_dir));
        device.save()?;
        let device_label = resolve_device_label(device_label, &device.pubkey_hex());

        let mut state = AccountState {
            owner_pubkey: device.pubkey_hex(),
            device_pubkey: device.pubkey_hex(),
            device_link_secret: default_device_link_secret(),
            has_owner_signing_authority: true,
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label: device_label.clone(),
            app_keys: None,
            app_keys_event: None,
            outbound_device_link_request: None,
            inbound_device_link_requests: Vec::new(),
        };

        let now = current_unix_seconds();
        let devices = vec![DeviceEntry::admin(
            state.device_pubkey.clone(),
            now,
            device_label,
        )];
        let dck = generate_dck();
        let wraps = wrap_dck_for_devices(device.keys().secret_key(), &devices, &dck)?;
        let snap = AppKeysSnapshot {
            owner_pubkey: state.owner_pubkey.clone(),
            signed_by_pubkey: Some(state.device_pubkey.clone()),
            created_at: now,
            devices,
            dck_generation: 1,
            wrapped_dck: wraps,
        };
        state.apply_app_keys(snap);

        let mut account = Self {
            state,
            device,
            owner_key: None,
        };
        account.record_current_app_keys_event()?;
        Ok(account)
    }

    /// **Restore** flow — import an existing admin-device nsec.
    pub fn restore(
        config_dir: &Path,
        device_nsec: &str,
        device_label: Option<String>,
    ) -> Result<Self, AccountError> {
        let device = DeviceIdentity::from_secret(device_nsec, key_path_in(config_dir))?;
        device.save()?;
        let device_label = resolve_device_label(device_label, &device.pubkey_hex());

        let mut state = AccountState {
            owner_pubkey: device.pubkey_hex(),
            device_pubkey: device.pubkey_hex(),
            device_link_secret: default_device_link_secret(),
            has_owner_signing_authority: true,
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label: device_label.clone(),
            app_keys: None,
            app_keys_event: None,
            outbound_device_link_request: None,
            inbound_device_link_requests: Vec::new(),
        };

        let now = current_unix_seconds();
        let devices = vec![DeviceEntry::admin(
            state.device_pubkey.clone(),
            now,
            device_label,
        )];
        let dck = generate_dck();
        let wraps = wrap_dck_for_devices(device.keys().secret_key(), &devices, &dck)?;
        let snap = AppKeysSnapshot {
            owner_pubkey: state.owner_pubkey.clone(),
            signed_by_pubkey: Some(state.device_pubkey.clone()),
            created_at: now,
            devices,
            dck_generation: 1,
            wrapped_dck: wraps,
        };
        state.apply_app_keys(snap);

        let mut account = Self {
            state,
            device,
            owner_key: None,
        };
        account.record_current_app_keys_event()?;
        Ok(account)
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
            device_link_secret: default_device_link_secret(),
            has_owner_signing_authority: false,
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label,
            app_keys: None,
            app_keys_event: None,
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
        Ok(Self {
            state,
            device,
            owner_key: None,
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
        let now = next_local_timestamp(self.state.app_keys.as_ref());
        let mut devices = self
            .state
            .app_keys
            .as_ref()
            .map(|s| s.devices.clone())
            .unwrap_or_default();
        devices.push(DeviceEntry::member(
            device_pubkey_hex.to_string(),
            now,
            label,
        ));
        let dck = generate_dck();
        let wraps = wrap_dck_for_devices(self.device.keys().secret_key(), &devices, &dck)?;
        let next_gen = self
            .state
            .app_keys
            .as_ref()
            .map_or(1, |s| s.dck_generation + 1);
        let new_snap = AppKeysSnapshot {
            owner_pubkey: self.state.owner_pubkey.clone(),
            signed_by_pubkey: Some(self.state.device_pubkey.clone()),
            created_at: now,
            devices,
            dck_generation: next_gen,
            wrapped_dck: wraps,
        };
        self.state.apply_app_keys(new_snap);
        self.record_current_app_keys_event()?;
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
        if snap.is_admin(device_pubkey_hex)
            && snap.devices.iter().filter(|d| d.is_admin()).count() <= 1
        {
            return Err(AccountError::CannotRemoveLastAdmin);
        }
        let now = next_local_timestamp(self.state.app_keys.as_ref());
        let devices: Vec<DeviceEntry> = snap
            .devices
            .iter()
            .filter(|d| d.pubkey != device_pubkey_hex)
            .cloned()
            .collect();
        let dck = generate_dck();
        let wraps = wrap_dck_for_devices(self.device.keys().secret_key(), &devices, &dck)?;
        let next_gen = snap.dck_generation + 1;
        let new_snap = AppKeysSnapshot {
            owner_pubkey: self.state.owner_pubkey.clone(),
            signed_by_pubkey: Some(self.state.device_pubkey.clone()),
            created_at: now,
            devices,
            dck_generation: next_gen,
            wrapped_dck: wraps,
        };
        self.state.apply_app_keys(new_snap);
        self.record_current_app_keys_event()?;
        Ok(self.state.app_keys.as_ref().expect("just applied"))
    }

    /// Rotate the DCK without changing the device roster. Useful for
    /// periodic key freshness ("rotate weekly even with no membership
    /// churn"). Owner-only.
    pub fn rotate_dck(&mut self) -> Result<&AppKeysSnapshot, AccountError> {
        if !self.state.can_manage_devices() {
            return Err(AccountError::NoOwnerAuthority);
        }
        let snap = self
            .state
            .app_keys
            .as_ref()
            .ok_or(AccountError::NoCurrentSnapshot)?;
        let devices = snap.devices.clone();
        let next_gen = snap.dck_generation + 1;
        let dck = generate_dck();
        let wraps = wrap_dck_for_devices(self.device.keys().secret_key(), &devices, &dck)?;
        let now = next_local_timestamp(Some(snap));
        let new_snap = AppKeysSnapshot {
            owner_pubkey: self.state.owner_pubkey.clone(),
            signed_by_pubkey: Some(self.state.device_pubkey.clone()),
            created_at: now,
            devices,
            dck_generation: next_gen,
            wrapped_dck: wraps,
        };
        self.state.apply_app_keys(new_snap);
        self.record_current_app_keys_event()?;
        Ok(self.state.app_keys.as_ref().expect("just applied"))
    }

    pub fn appoint_admin(
        &mut self,
        device_pubkey_hex: &str,
    ) -> Result<&AppKeysSnapshot, AccountError> {
        self.set_device_role(device_pubkey_hex, DeviceRole::Admin)
    }

    pub fn demote_admin(
        &mut self,
        device_pubkey_hex: &str,
    ) -> Result<&AppKeysSnapshot, AccountError> {
        self.set_device_role(device_pubkey_hex, DeviceRole::Member)
    }

    fn set_device_role(
        &mut self,
        device_pubkey_hex: &str,
        role: DeviceRole,
    ) -> Result<&AppKeysSnapshot, AccountError> {
        if !self.state.can_manage_devices() {
            return Err(AccountError::NoOwnerAuthority);
        }
        let snap = self
            .state
            .app_keys
            .as_ref()
            .ok_or(AccountError::NoCurrentSnapshot)?;
        let current = snap
            .device(device_pubkey_hex)
            .ok_or(AccountError::DeviceNotInRoster)?;
        if current.role == role {
            return Ok(self.state.app_keys.as_ref().expect("checked above"));
        }
        if current.is_admin()
            && role != DeviceRole::Admin
            && snap
                .devices
                .iter()
                .filter(|device| device.is_admin())
                .count()
                <= 1
        {
            return Err(AccountError::CannotRemoveLastAdmin);
        }
        let mut devices = snap.devices.clone();
        for device in &mut devices {
            if device.pubkey == device_pubkey_hex {
                device.role = role;
            }
        }
        let next_gen = snap.dck_generation + 1;
        let dck = generate_dck();
        let wraps = wrap_dck_for_devices(self.device.keys().secret_key(), &devices, &dck)?;
        let now = next_local_timestamp(Some(snap));
        let new_snap = AppKeysSnapshot {
            owner_pubkey: self.state.owner_pubkey.clone(),
            signed_by_pubkey: Some(self.state.device_pubkey.clone()),
            created_at: now,
            devices,
            dck_generation: next_gen,
            wrapped_dck: wraps,
        };
        self.state.apply_app_keys(new_snap);
        self.record_current_app_keys_event()?;
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
        let signer_pk = PublicKey::from_hex(snap.signer_pubkey())
            .map_err(|e| AccountError::InvalidOwnerPubkey(e.to_string()))?;
        let bytes = nip44::decrypt_to_bytes(self.device.keys().secret_key(), &signer_pk, wrap)
            .map_err(|e| AccountError::Unwrap(e.to_string()))?;
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| AccountError::InvalidDckLength(bytes.len()))?;
        Ok(arr)
    }

    fn record_current_app_keys_event(&mut self) -> Result<(), AccountError> {
        let Some(snapshot) = self.state.app_keys.as_ref() else {
            return Ok(());
        };
        if snapshot.signer_pubkey() != self.state.device_pubkey {
            return Ok(());
        }
        let event = build_app_keys_event(self.device.keys(), snapshot)
            .map_err(|e| AccountError::RosterEvent(e.to_string()))?;
        self.state.app_keys_event = Some(AppKeysEventRecord {
            event_id: event.id.to_hex(),
            signer_pubkey: event.pubkey.to_hex(),
            event_json: event.as_json(),
        });
        Ok(())
    }
}

fn generate_dck() -> [u8; 32] {
    // Fresh 32 random bytes via nostr-sdk's RNG (no extra deps).
    let keys = Keys::generate();
    let mut out = [0u8; 32];
    out.copy_from_slice(keys.secret_key().as_secret_bytes());
    out
}

fn default_device_link_secret() -> String {
    URL_SAFE_NO_PAD.encode(Uuid::new_v4().as_bytes())
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct LogoutReport {
    pub removed_key: bool,
    pub removed_owner_key: bool,
    pub removed_sync_cache: bool,
    pub cleared_account: bool,
    pub cleared_user_profile: bool,
    pub cleared_drives: bool,
    pub cleared_backup_targets: bool,
}

impl LogoutReport {
    #[must_use]
    pub fn changed(&self) -> bool {
        self.removed_key
            || self.removed_owner_key
            || self.removed_sync_cache
            || self.cleared_account
            || self.cleared_user_profile
            || self.cleared_drives
            || self.cleared_backup_targets
    }
}

pub fn logout_local_account(config_dir: &Path) -> Result<LogoutReport, AccountError> {
    let config_path = config_path_in(config_dir);
    let mut config = AppConfig::load_or_default(&config_path)?;

    let cleared_account = config.account.take().is_some();
    let cleared_user_profile = config.user_profile.take().is_some();
    let cleared_drives = !config.drives.is_empty();
    config.drives.clear();
    let cleared_backup_targets = !config.backup_targets.is_empty();
    config.backup_targets.clear();
    let mut report = LogoutReport {
        cleared_account,
        cleared_user_profile,
        cleared_drives,
        cleared_backup_targets,
        ..LogoutReport::default()
    };

    if config_path.exists()
        || report.cleared_account
        || report.cleared_user_profile
        || report.cleared_drives
        || report.cleared_backup_targets
    {
        config.save(&config_path)?;
    }

    report.removed_key = remove_file_if_present(&key_path_in(config_dir))?;
    report.removed_owner_key = remove_file_if_present(&owner_key_path_in(config_dir))?;
    report.removed_sync_cache = remove_file_if_present(&sync_cache_path_in(config_dir))?;

    Ok(report)
}

fn remove_file_if_present(path: &Path) -> Result<bool, std::io::Error> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests;
