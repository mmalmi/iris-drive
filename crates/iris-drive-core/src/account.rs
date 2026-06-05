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

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{Keys, PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::app_keys::{
    AppActorEntry, AppActorRole, AppKeysSnapshot, ApplyDecision, apply_snapshot,
};
use crate::config::{AppConfig, ConfigError};
use crate::identity::{DeviceIdentity, IdentityError, OwnerKey};
use crate::iris_profile::{
    IrisProfileCapabilities, IrisProfileError, IrisProfileFacet, IrisProfileId,
    IrisProfileKeyPurpose, IrisProfileRosterOp, IrisProfileRosterProjection,
    SignedIrisProfileRosterOp, build_iris_profile_roster_op_event,
    parse_iris_profile_roster_op_event, project_iris_profile_roster,
};
use crate::paths::{
    config_path_in, key_path_in, owner_key_path_in, recovery_phrase_path_in, sync_cache_path_in,
};
use crate::recovery_phrase::{
    RecoveryPhraseError, generate_recovery_phrase, recovery_phrase_to_profile_id,
    save_recovery_phrase, validate_recovery_phrase,
};

#[derive(Debug, Error)]
pub enum AccountError {
    #[error("identity: {0}")]
    Identity(#[from] IdentityError),
    #[error("iris profile: {0}")]
    IrisProfile(#[from] IrisProfileError),
    #[error("recovery phrase: {0}")]
    RecoveryPhrase(#[from] RecoveryPhraseError),
    #[error("recovery phrase belongs to profile {found}, expected {expected}")]
    RecoveryProfileMismatch {
        expected: IrisProfileId,
        found: IrisProfileId,
    },
    #[error("recovery authority is not active in this IrisProfile")]
    RecoveryAuthorityUnavailable,
    #[error("recovery authority cannot rotate key epochs")]
    RecoveryCannotRotateKeyEpochs,
    #[error("current app key was tombstoned and cannot be re-added")]
    CurrentAppKeyTombstoned,
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
    pub profile_id: IrisProfileId,
    /// Stable account id. New accounts use their first admin device pubkey.
    /// The name is kept for config/wire compatibility.
    pub owner_pubkey: String,
    pub device_pubkey: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profile_roster_ops: Vec<SignedIrisProfileRosterOp>,
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
    pub outbound_device_link_request: Option<PendingDeviceLinkRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inbound_device_link_requests: Vec<InboundDeviceLinkRequest>,
}

impl AccountState {
    #[must_use]
    pub fn profile_projection(&self) -> IrisProfileRosterProjection {
        project_iris_profile_roster(self.profile_id, self.profile_roster_ops.clone())
    }

    #[must_use]
    pub fn root_scope_id(&self) -> String {
        self.profile_id.to_string()
    }

    #[must_use]
    pub fn app_keys_from_profile(&self) -> Option<AppKeysSnapshot> {
        app_keys_from_profile_roster(
            &self.owner_pubkey,
            self.profile_id,
            &self.profile_roster_ops,
        )
    }

    pub fn sync_app_keys_from_profile(&mut self) -> bool {
        let Some(snapshot) = self.app_keys_from_profile() else {
            return false;
        };
        let changed = self.app_keys.as_ref() != Some(&snapshot);
        self.app_keys = Some(snapshot);
        self.recompute_authorization();
        changed
    }

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
        if !self.profile_roster_ops.is_empty() {
            return self
                .profile_projection()
                .can_admin_profile(&self.device_pubkey);
        }
        self.app_keys
            .as_ref()
            .map_or(self.has_owner_signing_authority, |snap| {
                snap.is_admin(&self.device_pubkey)
            })
    }

    /// Recompute `authorization_state` from the current `AppKeys` snapshot.
    pub fn recompute_authorization(&mut self) {
        if !self.profile_roster_ops.is_empty() {
            let projection = self.profile_projection();
            self.has_owner_signing_authority = projection.can_admin_profile(&self.device_pubkey);
            self.authorization_state = if projection.can_write_roots(&self.device_pubkey) {
                DeviceAuthorizationState::Authorized
            } else if projection.tombstones.contains_key(&self.device_pubkey)
                || self.authorization_state == DeviceAuthorizationState::Authorized
            {
                DeviceAuthorizationState::Revoked
            } else {
                self.authorization_state
            };
            if self.authorization_state == DeviceAuthorizationState::Authorized {
                self.outbound_device_link_request = None;
            }
            return;
        }
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
        self.recompute_authorization();
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

#[must_use]
pub fn app_keys_from_profile_roster(
    owner_pubkey: &str,
    profile_id: IrisProfileId,
    profile_roster_ops: &[SignedIrisProfileRosterOp],
) -> Option<AppKeysSnapshot> {
    let projection = project_iris_profile_roster(profile_id, profile_roster_ops.iter().cloned());
    app_keys_from_profile_projection(owner_pubkey, &projection)
}

#[must_use]
pub fn app_keys_from_profile_projection(
    owner_pubkey: &str,
    projection: &IrisProfileRosterProjection,
) -> Option<AppKeysSnapshot> {
    let key_epoch = projection.key_epochs.values().next_back()?;
    let app_key_pubkeys: BTreeSet<_> = projection.active_app_key_pubkeys().into_iter().collect();
    if app_key_pubkeys.is_empty() {
        return None;
    }
    let mut app_actors = projection
        .active_facets
        .values()
        .filter(|facet| facet.is_app_key())
        .map(|facet| {
            let role = if facet.capabilities.can_admin_profile {
                AppActorRole::Admin
            } else {
                AppActorRole::Member
            };
            AppActorEntry {
                pubkey: facet.pubkey.clone(),
                added_at: facet.added_at,
                label: facet.label.clone(),
                role,
            }
        })
        .collect::<Vec<_>>();
    app_actors.sort_by(|left, right| left.pubkey.cmp(&right.pubkey));
    let wrapped_dck = key_epoch
        .wrapped_dck
        .iter()
        .filter(|(pubkey, _)| app_key_pubkeys.contains(*pubkey))
        .map(|(pubkey, wrap)| (pubkey.clone(), wrap.clone()))
        .collect();
    Some(AppKeysSnapshot {
        owner_pubkey: owner_pubkey.to_string(),
        signed_by_pubkey: Some(key_epoch.signed_by_pubkey.clone()),
        created_at: key_epoch.created_at,
        app_actors,
        dck_generation: key_epoch.epoch,
        wrapped_dck,
    })
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
        let recovery_phrase = generate_recovery_phrase()?;
        let profile_id = recovery_phrase_to_profile_id(&recovery_phrase)?;
        let recovery_key =
            OwnerKey::from_recovery_phrase(&recovery_phrase, owner_key_path_in(config_dir))?;
        let device = DeviceIdentity::generate(key_path_in(config_dir));
        device.save()?;
        save_recovery_phrase(recovery_phrase_path_in(config_dir), &recovery_phrase)?;
        let device_label = resolve_device_label(device_label, &device.pubkey_hex());

        let mut state = AccountState {
            profile_id,
            owner_pubkey: device.pubkey_hex(),
            device_pubkey: device.pubkey_hex(),
            profile_roster_ops: Vec::new(),
            device_link_secret: default_device_link_secret(),
            has_owner_signing_authority: true,
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label: device_label.clone(),
            app_keys: None,
            outbound_device_link_request: None,
            inbound_device_link_requests: Vec::new(),
        };

        let now = current_unix_seconds();
        let app_actors = vec![AppActorEntry::admin(
            state.device_pubkey.clone(),
            now,
            device_label,
        )];
        let dck = generate_dck();
        let wraps = wrap_dck_for_app_actors(device.keys().secret_key(), &app_actors, &dck)?;
        let recovery_pubkey = recovery_key.pubkey_hex();
        state.profile_roster_ops = initial_profile_roster_ops(
            device.keys(),
            profile_id,
            &app_actors[0],
            Some(&recovery_pubkey),
            &dck,
            now,
        )?;
        let snap = AppKeysSnapshot {
            owner_pubkey: state.owner_pubkey.clone(),
            signed_by_pubkey: Some(state.device_pubkey.clone()),
            created_at: now,
            app_actors,
            dck_generation: 1,
            wrapped_dck: wraps,
        };
        state.apply_app_keys(snap);

        let account = Self {
            state,
            device,
            owner_key: None,
        };
        Ok(account)
    }

    /// **Restore** flow — import an existing admin-device nsec.
    pub fn restore(
        config_dir: &Path,
        device_nsec: &str,
        device_label: Option<String>,
    ) -> Result<Self, AccountError> {
        let recovery_phrase = validate_recovery_phrase(device_nsec).ok();
        let profile_id = recovery_phrase
            .as_deref()
            .map(recovery_phrase_to_profile_id)
            .transpose()?
            .unwrap_or_else(IrisProfileId::new_v4);
        let recovery_key = recovery_phrase
            .as_deref()
            .map(|phrase| OwnerKey::from_recovery_phrase(phrase, owner_key_path_in(config_dir)))
            .transpose()?;
        let device = if recovery_phrase.is_some() {
            DeviceIdentity::generate(key_path_in(config_dir))
        } else {
            DeviceIdentity::from_secret(device_nsec, key_path_in(config_dir))?
        };
        device.save()?;
        if let Some(phrase) = recovery_phrase {
            save_recovery_phrase(recovery_phrase_path_in(config_dir), &phrase)?;
        }
        let device_label = resolve_device_label(device_label, &device.pubkey_hex());

        let mut state = AccountState {
            profile_id,
            owner_pubkey: device.pubkey_hex(),
            device_pubkey: device.pubkey_hex(),
            profile_roster_ops: Vec::new(),
            device_link_secret: default_device_link_secret(),
            has_owner_signing_authority: true,
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label: device_label.clone(),
            app_keys: None,
            outbound_device_link_request: None,
            inbound_device_link_requests: Vec::new(),
        };

        let now = current_unix_seconds();
        let app_actors = vec![AppActorEntry::admin(
            state.device_pubkey.clone(),
            now,
            device_label,
        )];
        let dck = generate_dck();
        let wraps = wrap_dck_for_app_actors(device.keys().secret_key(), &app_actors, &dck)?;
        let recovery_pubkey = recovery_key.as_ref().map(OwnerKey::pubkey_hex);
        state.profile_roster_ops = initial_profile_roster_ops(
            device.keys(),
            profile_id,
            &app_actors[0],
            recovery_pubkey.as_deref(),
            &dck,
            now,
        )?;
        let snap = AppKeysSnapshot {
            owner_pubkey: state.owner_pubkey.clone(),
            signed_by_pubkey: Some(state.device_pubkey.clone()),
            created_at: now,
            app_actors,
            dck_generation: 1,
            wrapped_dck: wraps,
        };
        state.apply_app_keys(snap);

        let account = Self {
            state,
            device,
            owner_key: None,
        };
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
            profile_id: IrisProfileId::new_v4(),
            owner_pubkey: owner_pubkey_hex,
            device_pubkey: device.pubkey_hex(),
            profile_roster_ops: Vec::new(),
            device_link_secret: default_device_link_secret(),
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
        let now = next_profile_timestamp(&self.state);
        self.append_profile_roster_op(
            IrisProfileRosterOp::AddFacet {
                facet: IrisProfileFacet::app_key(
                    device_pubkey_hex.to_string(),
                    now,
                    label,
                    IrisProfileCapabilities::app_writer(),
                ),
            },
            now,
        )?;
        let dck = generate_dck();
        self.rotate_profile_dck_epoch(&dck, now + 1)?;
        self.state.sync_app_keys_from_profile();
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
            && snap.app_actors.iter().filter(|d| d.is_admin()).count() <= 1
        {
            return Err(AccountError::CannotRemoveLastAdmin);
        }
        let now = next_profile_timestamp(&self.state);
        self.append_profile_roster_op(
            IrisProfileRosterOp::TombstoneFacet {
                pubkey: device_pubkey_hex.to_string(),
                reason: None,
            },
            now,
        )?;
        let dck = generate_dck();
        self.rotate_profile_dck_epoch(&dck, now + 1)?;
        self.state.sync_app_keys_from_profile();
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
        let dck = generate_dck();
        let now = next_profile_timestamp(&self.state).max(next_local_timestamp(Some(snap)));
        self.rotate_profile_dck_epoch(&dck, now)?;
        self.state.sync_app_keys_from_profile();
        Ok(self.state.app_keys.as_ref().expect("just applied"))
    }

    /// Add a NIP-46 signer as an `IrisProfile` recovery authority. When
    /// `can_decrypt_key_epochs` is true, the current admin rotates the key
    /// epoch so the signer receives a DCK wrap immediately.
    pub fn add_nip46_recovery(
        &mut self,
        nip46_pubkey_hex: &str,
        label: Option<String>,
        can_decrypt_key_epochs: bool,
    ) -> Result<(), AccountError> {
        if !self.state.can_manage_devices() {
            return Err(AccountError::NoOwnerAuthority);
        }
        PublicKey::from_hex(nip46_pubkey_hex)
            .map_err(|e| AccountError::InvalidDevicePubkey(e.to_string()))?;
        let projection = self.state.profile_projection();
        if projection.active_facets.contains_key(nip46_pubkey_hex) {
            return Err(AccountError::AlreadyAuthorized);
        }
        if projection.tombstones.contains_key(nip46_pubkey_hex) {
            return Err(AccountError::CurrentAppKeyTombstoned);
        }

        let now = next_profile_timestamp(&self.state);
        self.append_profile_roster_op(
            IrisProfileRosterOp::AddFacet {
                facet: IrisProfileFacet::nip46(
                    nip46_pubkey_hex.to_string(),
                    now,
                    label,
                    can_decrypt_key_epochs,
                ),
            },
            now,
        )?;
        if can_decrypt_key_epochs {
            let dck = generate_dck();
            self.rotate_profile_dck_epoch(&dck, now + 1)?;
        }
        self.state.sync_app_keys_from_profile();
        Ok(())
    }

    /// Use the profile's recovery phrase authority to admit this install's
    /// fresh `AppKey` into an already-known `IrisProfile` roster.
    ///
    /// The recovery phrase stays a recovery/admin facet only: it proves it can
    /// decrypt the current epoch, signs the `AppKey` admission, then signs a
    /// coherent new key epoch wrapped to every active recipient.
    pub fn admit_current_app_key_with_recovery_phrase(
        &mut self,
        recovery_phrase: &str,
        label: Option<String>,
    ) -> Result<&AppKeysSnapshot, AccountError> {
        let recovery_phrase = validate_recovery_phrase(recovery_phrase)?;
        let found_profile_id = recovery_phrase_to_profile_id(&recovery_phrase)?;
        if found_profile_id != self.state.profile_id {
            return Err(AccountError::RecoveryProfileMismatch {
                expected: self.state.profile_id,
                found: found_profile_id,
            });
        }
        let recovery_key =
            OwnerKey::from_recovery_phrase(&recovery_phrase, owner_key_path_in(Path::new("")))?;
        self.admit_current_app_key_with_authority_keys(
            recovery_key.keys(),
            IrisProfileKeyPurpose::RecoveryPhrase,
            label,
        )
    }

    /// Use a configured NIP-46 signer authority to admit this install's fresh
    /// `AppKey`. If that signer is not configured to decrypt key epochs, the
    /// `AppKey` is authorized but its current DCK wrap remains repair-needed.
    pub fn admit_current_app_key_with_nip46_keys(
        &mut self,
        nip46_keys: &Keys,
        label: Option<String>,
    ) -> Result<&AppKeysSnapshot, AccountError> {
        self.admit_current_app_key_with_authority_keys(
            nip46_keys,
            IrisProfileKeyPurpose::Nip46Signer,
            label,
        )
    }

    fn admit_current_app_key_with_authority_keys(
        &mut self,
        authority_keys: &Keys,
        expected_purpose: IrisProfileKeyPurpose,
        label: Option<String>,
    ) -> Result<&AppKeysSnapshot, AccountError> {
        let authority_pubkey = authority_keys.public_key().to_hex();
        let projection = self.state.profile_projection();
        let Some(authority_facet) = projection.active_facets.get(&authority_pubkey) else {
            return Err(AccountError::RecoveryAuthorityUnavailable);
        };
        if !authority_facet.has_purpose(expected_purpose)
            || !authority_facet.capabilities.can_recover_app_keys
        {
            return Err(AccountError::RecoveryAuthorityUnavailable);
        }
        if projection
            .tombstones
            .contains_key(&self.state.device_pubkey)
        {
            return Err(AccountError::CurrentAppKeyTombstoned);
        }
        if projection.can_write_roots(&self.state.device_pubkey) {
            return Err(AccountError::AlreadyAuthorized);
        }
        let should_rotate_epoch = authority_facet.capabilities.can_decrypt_key_epochs;
        if should_rotate_epoch {
            self.current_dck_from_authority_keys(authority_keys, expected_purpose)?;
            if !authority_facet.capabilities.can_change_key_epochs() {
                return Err(AccountError::RecoveryCannotRotateKeyEpochs);
            }
        }

        let now = next_profile_timestamp(&self.state);
        let label = label.or_else(|| self.state.device_label.clone());
        let add_op = signed_profile_roster_op(
            authority_keys,
            self.state.profile_id,
            IrisProfileRosterOp::AddFacet {
                facet: IrisProfileFacet::app_key(
                    self.state.device_pubkey.clone(),
                    now,
                    label,
                    IrisProfileCapabilities::app_admin(),
                ),
            },
            now,
        )?;
        self.state.profile_roster_ops.push(add_op);

        if should_rotate_epoch {
            let dck = generate_dck();
            self.rotate_profile_dck_epoch_with_signer(authority_keys, &dck, now + 1)?;
        }
        self.state.sync_app_keys_from_profile();
        Ok(self.state.app_keys.as_ref().expect("just applied"))
    }

    pub fn appoint_admin(
        &mut self,
        device_pubkey_hex: &str,
    ) -> Result<&AppKeysSnapshot, AccountError> {
        self.set_device_role(device_pubkey_hex, AppActorRole::Admin)
    }

    pub fn demote_admin(
        &mut self,
        device_pubkey_hex: &str,
    ) -> Result<&AppKeysSnapshot, AccountError> {
        self.set_device_role(device_pubkey_hex, AppActorRole::Member)
    }

    fn set_device_role(
        &mut self,
        device_pubkey_hex: &str,
        role: AppActorRole,
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
            .app_actor(device_pubkey_hex)
            .ok_or(AccountError::DeviceNotInRoster)?;
        if current.role == role {
            return Ok(self.state.app_keys.as_ref().expect("checked above"));
        }
        if current.is_admin()
            && role != AppActorRole::Admin
            && snap
                .app_actors
                .iter()
                .filter(|device| device.is_admin())
                .count()
                <= 1
        {
            return Err(AccountError::CannotRemoveLastAdmin);
        }
        let capabilities = match role {
            AppActorRole::Admin => IrisProfileCapabilities::app_admin(),
            AppActorRole::Member => IrisProfileCapabilities::app_writer(),
        };
        let now = next_profile_timestamp(&self.state);
        self.append_profile_roster_op(
            IrisProfileRosterOp::SetCapabilities {
                pubkey: device_pubkey_hex.to_string(),
                capabilities,
            },
            now,
        )?;
        let dck = generate_dck();
        self.rotate_profile_dck_epoch(&dck, now + 1)?;
        self.state.sync_app_keys_from_profile();
        Ok(self.state.app_keys.as_ref().expect("just applied"))
    }

    fn append_profile_roster_op(
        &mut self,
        op: IrisProfileRosterOp,
        created_at: i64,
    ) -> Result<(), AccountError> {
        let signed =
            signed_profile_roster_op(self.device.keys(), self.state.profile_id, op, created_at)?;
        self.state.profile_roster_ops.push(signed);
        Ok(())
    }

    fn rotate_profile_dck_epoch(
        &mut self,
        dck: &[u8; 32],
        created_at: i64,
    ) -> Result<(), AccountError> {
        let signer = self.device.keys().clone();
        self.rotate_profile_dck_epoch_with_signer(&signer, dck, created_at)
    }

    fn rotate_profile_dck_epoch_with_signer(
        &mut self,
        signer_keys: &Keys,
        dck: &[u8; 32],
        created_at: i64,
    ) -> Result<(), AccountError> {
        let projection = self.state.profile_projection();
        let recipients = projection
            .active_facets
            .values()
            .filter(|facet| facet.capabilities.can_receive_key_wraps)
            .map(|facet| facet.pubkey.as_str())
            .collect::<Vec<_>>();
        let wrapped_dck = wrap_dck_for_pubkeys(signer_keys.secret_key(), recipients, dck)?;
        let epoch = projection
            .key_epochs
            .keys()
            .next_back()
            .map_or(1, |epoch| epoch + 1);
        let signed = signed_profile_roster_op(
            signer_keys,
            self.state.profile_id,
            IrisProfileRosterOp::RotateKeyEpoch { epoch, wrapped_dck },
            created_at,
        )?;
        self.state.profile_roster_ops.push(signed);
        Ok(())
    }

    /// Decrypt this device's DCK wrap from the current snapshot. Errors
    /// with `NoWrapForThisDevice` if the device has been revoked or
    /// never authorized.
    pub fn current_dck(&self) -> Result<[u8; 32], AccountError> {
        if !self.state.profile_roster_ops.is_empty() {
            let projection = self.state.profile_projection();
            let key_epoch = projection
                .key_epochs
                .values()
                .next_back()
                .ok_or(AccountError::NoCurrentSnapshot)?;
            let wrap = key_epoch
                .wrapped_dck
                .get(&self.state.device_pubkey)
                .ok_or(AccountError::NoWrapForThisDevice)?;
            let signer_pk = PublicKey::from_hex(&key_epoch.signed_by_pubkey)
                .map_err(|e| AccountError::InvalidOwnerPubkey(e.to_string()))?;
            let bytes = nip44::decrypt_to_bytes(self.device.keys().secret_key(), &signer_pk, wrap)
                .map_err(|e| AccountError::Unwrap(e.to_string()))?;
            let arr: [u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| AccountError::InvalidDckLength(bytes.len()))?;
            return Ok(arr);
        }
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

    pub fn current_dck_from_recovery_phrase(
        &self,
        recovery_phrase: &str,
    ) -> Result<[u8; 32], AccountError> {
        let profile_id = recovery_phrase_to_profile_id(recovery_phrase)?;
        if profile_id != self.state.profile_id {
            return Err(AccountError::RecoveryProfileMismatch {
                expected: self.state.profile_id,
                found: profile_id,
            });
        }
        let recovery_key =
            OwnerKey::from_recovery_phrase(recovery_phrase, owner_key_path_in(Path::new("")))?;
        self.current_dck_from_authority_keys(
            recovery_key.keys(),
            IrisProfileKeyPurpose::RecoveryPhrase,
        )
    }

    pub fn current_dck_from_nip46_keys(&self, nip46_keys: &Keys) -> Result<[u8; 32], AccountError> {
        self.current_dck_from_authority_keys(nip46_keys, IrisProfileKeyPurpose::Nip46Signer)
    }

    fn current_dck_from_authority_keys(
        &self,
        authority_keys: &Keys,
        expected_purpose: IrisProfileKeyPurpose,
    ) -> Result<[u8; 32], AccountError> {
        let authority_pubkey = authority_keys.public_key().to_hex();
        let projection = self.state.profile_projection();
        let Some(facet) = projection.active_facets.get(&authority_pubkey) else {
            return Err(AccountError::RecoveryAuthorityUnavailable);
        };
        if !facet.has_purpose(expected_purpose) || !facet.capabilities.can_decrypt_key_epochs {
            return Err(AccountError::RecoveryAuthorityUnavailable);
        }
        let key_epoch = projection
            .key_epochs
            .values()
            .next_back()
            .ok_or(AccountError::NoCurrentSnapshot)?;
        let wrap = key_epoch
            .wrapped_dck
            .get(&authority_pubkey)
            .ok_or(AccountError::NoWrapForThisDevice)?;
        let signer_pk = PublicKey::from_hex(&key_epoch.signed_by_pubkey)
            .map_err(|e| AccountError::InvalidOwnerPubkey(e.to_string()))?;
        let bytes = nip44::decrypt_to_bytes(authority_keys.secret_key(), &signer_pk, wrap)
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

fn default_device_link_secret() -> String {
    URL_SAFE_NO_PAD.encode(Uuid::new_v4().as_bytes())
}

fn wrap_dck_for_app_actors(
    owner_secret: &SecretKey,
    app_actors: &[AppActorEntry],
    dck: &[u8; 32],
) -> Result<BTreeMap<String, String>, AccountError> {
    wrap_dck_for_pubkeys(
        owner_secret,
        app_actors.iter().map(|actor| actor.pubkey.as_str()),
        dck,
    )
}

fn wrap_dck_for_pubkeys<'a, I>(
    owner_secret: &SecretKey,
    pubkeys: I,
    dck: &[u8; 32],
) -> Result<BTreeMap<String, String>, AccountError>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut wraps = BTreeMap::new();
    for pubkey in pubkeys {
        let pk = PublicKey::from_hex(pubkey)
            .map_err(|e| AccountError::InvalidOwnerPubkey(e.to_string()))?;
        let ct = nip44::encrypt(owner_secret, &pk, dck.as_slice(), Nip44Version::V2)
            .map_err(|e| AccountError::Wrap(e.to_string()))?;
        wraps.insert(pubkey.to_string(), ct);
    }
    Ok(wraps)
}

fn initial_profile_roster_ops(
    signer_keys: &Keys,
    profile_id: IrisProfileId,
    app_entry: &AppActorEntry,
    recovery_pubkey: Option<&str>,
    dck: &[u8; 32],
    created_at: i64,
) -> Result<Vec<SignedIrisProfileRosterOp>, AccountError> {
    let app_pubkey = app_entry.pubkey.clone();
    let app_label = app_entry.label.clone();
    let app_op = signed_profile_roster_op(
        signer_keys,
        profile_id,
        IrisProfileRosterOp::AddFacet {
            facet: IrisProfileFacet::app_key(
                app_pubkey.clone(),
                created_at,
                app_label,
                IrisProfileCapabilities::app_admin(),
            ),
        },
        created_at,
    )?;
    let mut ops = vec![app_op];
    let mut recipients = vec![app_pubkey.as_str()];
    let epoch_created_at = if let Some(recovery_pubkey) = recovery_pubkey {
        let recovery_op = signed_profile_roster_op(
            signer_keys,
            profile_id,
            IrisProfileRosterOp::AddFacet {
                facet: IrisProfileFacet::recovery_phrase(
                    recovery_pubkey.to_string(),
                    created_at + 1,
                ),
            },
            created_at + 1,
        )?;
        ops.push(recovery_op);
        recipients.push(recovery_pubkey);
        created_at + 2
    } else {
        created_at + 1
    };
    let wrapped_dck = wrap_dck_for_pubkeys(signer_keys.secret_key(), recipients, dck)?;
    let epoch_op = signed_profile_roster_op(
        signer_keys,
        profile_id,
        IrisProfileRosterOp::RotateKeyEpoch {
            epoch: 1,
            wrapped_dck,
        },
        epoch_created_at,
    )?;
    ops.push(epoch_op);
    Ok(ops)
}

fn signed_profile_roster_op(
    signer_keys: &Keys,
    profile_id: IrisProfileId,
    op: IrisProfileRosterOp,
    created_at: i64,
) -> Result<SignedIrisProfileRosterOp, AccountError> {
    let event = build_iris_profile_roster_op_event(
        signer_keys,
        profile_id,
        Vec::new(),
        None,
        op,
        created_at,
    )?;
    parse_iris_profile_roster_op_event(&event).map_err(AccountError::from)
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

fn next_profile_timestamp(state: &AccountState) -> i64 {
    let latest_profile_op = state
        .profile_roster_ops
        .iter()
        .map(|op| op.content.created_at)
        .max()
        .unwrap_or(0);
    let latest_snapshot = state
        .app_keys
        .as_ref()
        .map_or(0, |snapshot| snapshot.created_at);
    current_unix_seconds()
        .max(latest_profile_op)
        .max(latest_snapshot)
        + 1
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
