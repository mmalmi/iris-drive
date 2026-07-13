//! Profile state machine.
//!
//! Wraps the profile roster model — stable `NostrIdentity` id, this app install's
//! `AppKey`, whether the `AppKey` is an admin in the current roster, and the
//! current authorization state.
//!
//! Three creation paths mirror the iris-chat-rs onboarding flows:
//!
//! 1. **Create** — fresh `AppKey`. Single-install default; this `AppKey`
//!    is the first admin and signs the first roster.
//! 2. **Restore** — import recovery authority or an existing admin `AppKey`.
//! 3. **Link** — paste/scan an `NostrIdentity`/admin `AppKey` invite. Generate a
//!    fresh `AppKey` and wait in `AwaitingApproval` until an admin accepts it.

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use nostr_sdk::nips::nip19::FromBech32;
use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{Event, JsonUtil, Keys, PublicKey, SecretKey};
use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::app_keys::{AppActorEntry, AppActorRole, AppKeysProjection};
use crate::config::{AppConfig, ConfigError};
use crate::device_labels::{
    decrypt_drive_device_labels_with_dck, drive_device_label_payload,
    encrypt_drive_device_labels_with_dck, normalize_drive_device_labels,
};
use crate::identity::{AppKey, IdentityError, RecoveryKey};
use crate::nostr_identity::{
    ApproveNostrIdentityDeviceApprovalBootstrapOptions,
    NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA, NostrIdentityCapabilities,
    NostrIdentityDeviceApprovalBootstrap, NostrIdentityDeviceApprovalReceipt, NostrIdentityError,
    NostrIdentityFacet, NostrIdentityId, NostrIdentityKeyPurpose, NostrIdentityRosterOp,
    NostrIdentityRosterProjection, SignedNostrIdentityRosterOp,
    approve_nostr_identity_device_approval_bootstrap,
    build_nostr_identity_device_approval_receipt_event,
    build_nostr_identity_roster_op_event_with_client_nonce,
    build_nostr_identity_roster_op_event_with_encrypted_device_labels,
    encrypted_device_label_payloads_from_nostr_identity_roster_op_event,
    nostr_identity_roster_parent_ids, parse_nostr_identity_roster_op_event,
    project_nostr_identity_roster,
};
use crate::paths::{config_path_in, key_path_in, recovery_phrase_path_in, sync_cache_path_in};
use crate::recovery_phrase::{RecoveryPhraseError, save_recovery_phrase, validate_recovery_phrase};

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("identity: {0}")]
    Identity(#[from] IdentityError),
    #[error("iris profile: {0}")]
    NostrIdentity(#[from] NostrIdentityError),
    #[error("recovery phrase: {0}")]
    RecoveryPhrase(#[from] RecoveryPhraseError),
    #[error("recovery authority is not active in this NostrIdentity")]
    RecoveryAuthorityUnavailable,
    #[error("recovery authority cannot rotate secret epochs")]
    RecoveryCannotRotateSecretEpochs,
    #[error("current app key was tombstoned and cannot be re-added")]
    CurrentAppKeyTombstoned,
    #[error("invalid AppKey pubkey: {0}")]
    InvalidAppKeyPubkey(String),
    #[error("invalid AppKey-link invite secret key: {0}")]
    InvalidAppKeyLinkInviteSecret(String),
    #[error("invalid app-key approval bootstrap: {0}")]
    InvalidAppKeyApprovalBootstrap(String),
    #[error("this AppKey is not an admin")]
    NoAdminAuthority,
    #[error("AppKey already authorized")]
    AppKeyAlreadyAuthorized,
    #[error("AppKey not in roster")]
    AppKeyNotInRoster,
    #[error("AppKey label is required")]
    InvalidAppKeyLabel,
    #[error("cannot remove the last admin AppKey")]
    CannotRemoveLastAdmin,
    #[error("no AppKeys projection yet")]
    NoCurrentAppKeysProjection,
    #[error("no DCK wrap for this AppKey (revoked or never authorized)")]
    NoWrapForThisAppKey,
    #[error("current AppKey cannot repair secret epoch signed by {signed_by_pubkey}")]
    CurrentAppKeyCannotRepairSecretEpoch { signed_by_pubkey: String },
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

/// Current `AppKey` authorization status relative to the `NostrIdentity` roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppKeyAuthorizationState {
    /// This `AppKey` is active in the latest roster projection.
    Authorized,
    /// This `AppKey` is waiting for a roster admin or recovery authority to admit it.
    AwaitingApproval,
    /// This `AppKey` was previously authorized and has since been removed.
    Revoked,
}

pub const MAX_INBOUND_APP_KEY_LINK_REQUESTS: usize = 32;
pub const MAX_HANDLED_APP_KEY_LINK_REQUESTS: usize = 128;
pub const MAX_PENDING_DEVICE_APPROVAL_RECEIPTS: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PendingAppKeyLinkRequest {
    #[serde(alias = "admin_device_pubkey")]
    pub admin_app_key_pubkey: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub invite_pubkey: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub request_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub request_key_secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_receipt_event: Option<String>,
    pub requested_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InboundAppKeyLinkRequest {
    #[serde(alias = "device_pubkey")]
    pub app_key_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub invite_pubkey: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub request_url: String,
    pub requested_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct HandledAppKeyLinkRequest {
    #[serde(alias = "device_pubkey")]
    pub app_key_pubkey: String,
    pub requested_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PendingDeviceApprovalReceipt {
    pub request_pubkey: String,
    pub device_app_key_pubkey: String,
    #[serde(
        default,
        alias = "request_relay",
        skip_serializing_if = "String::is_empty"
    )]
    pub relay_url: String,
    pub event_json: String,
}

/// Persisted local profile state. Lives inside `AppConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileState {
    #[serde(
        alias = "owner_pubkey",
        deserialize_with = "deserialize_profile_id_or_legacy_owner"
    )]
    pub profile_id: NostrIdentityId,
    #[serde(alias = "device_pubkey")]
    pub app_key_pubkey: String,
    #[serde(default, skip_serializing)]
    pub profile_roster_ops: Vec<SignedNostrIdentityRosterOp>,
    #[serde(default = "default_app_key_link_secret", alias = "device_link_secret")]
    pub app_key_link_secret: String,
    pub authorization_state: AppKeyAuthorizationState,
    #[serde(alias = "device_label")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_key_label: Option<String>,
    /// Runtime projection cache derived from `profile_roster_ops`.
    #[serde(default, skip_serializing)]
    pub app_keys: Option<AppKeysProjection>,
    /// Runtime roster projection cache derived from `profile_roster_ops`.
    #[serde(skip)]
    pub profile_roster_projection: Option<NostrIdentityRosterProjection>,
    /// Join bootstrap retained through authorization when it carries an applied
    /// receipt, allowing exact ACK replay if the owner retransmits that receipt.
    #[serde(alias = "outbound_device_link_request")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outbound_app_key_link_request: Option<PendingAppKeyLinkRequest>,
    #[serde(alias = "inbound_device_link_requests")]
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inbound_app_key_link_requests: Vec<InboundAppKeyLinkRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub handled_app_key_link_requests: Vec<HandledAppKeyLinkRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_device_approval_receipts: Vec<PendingDeviceApprovalReceipt>,
}

fn deserialize_profile_id_or_legacy_owner<'de, D>(
    deserializer: D,
) -> Result<NostrIdentityId, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if let Ok(profile_id) = NostrIdentityId::from_str(&value) {
        return Ok(profile_id);
    }
    legacy_profile_id_from_owner_pubkey(&value).ok_or_else(|| {
        serde::de::Error::custom("profile_id must be a UUID or legacy 64-character owner_pubkey")
    })
}

fn legacy_profile_id_from_owner_pubkey(value: &str) -> Option<NostrIdentityId> {
    if value.len() != 64 {
        return None;
    }
    let mut decoded = [0_u8; 32];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        let hex = std::str::from_utf8(chunk).ok()?;
        decoded[index] = u8::from_str_radix(hex, 16).ok()?;
    }
    let mut uuid_bytes = [0_u8; 16];
    for index in 0..16 {
        uuid_bytes[index] = decoded[index] ^ decoded[index + 16];
    }
    uuid_bytes[6] = (uuid_bytes[6] & 0x0f) | 0x80;
    uuid_bytes[8] = (uuid_bytes[8] & 0x3f) | 0x80;
    Some(NostrIdentityId::from_uuid(Uuid::from_bytes(uuid_bytes)))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretWrapRepairOutcome {
    pub epoch: u64,
    pub repaired_pubkeys: Vec<String>,
    pub projection: AppKeysProjection,
}

impl ProfileState {
    #[must_use]
    pub fn has_profile_roster_evidence(&self) -> bool {
        !self.profile_roster_ops.is_empty()
    }

    #[must_use]
    pub fn profile_projection(&self) -> NostrIdentityRosterProjection {
        if let Some(projection) = self.profile_roster_projection.as_ref() {
            return projection.clone();
        }
        project_nostr_identity_roster(self.profile_id, self.profile_roster_ops.clone())
    }

    #[must_use]
    pub fn root_scope_id(&self) -> String {
        self.profile_id.to_string()
    }

    #[must_use]
    pub fn app_keys_from_profile(&self) -> Option<AppKeysProjection> {
        app_keys_from_profile_projection_with_local_labels(
            &self.profile_projection(),
            self.app_keys.as_ref(),
            Some(&self.app_key_pubkey),
            self.app_key_label.as_deref(),
        )
    }

    #[must_use]
    pub fn current_app_keys_projection(&self) -> Option<AppKeysProjection> {
        if self.has_profile_roster_evidence() {
            return self
                .app_keys_from_profile()
                .or_else(|| self.app_keys.clone());
        }
        self.app_keys.clone()
    }

    #[must_use]
    pub fn active_root_writer_app_key_pubkeys(&self) -> Vec<String> {
        let mut app_keys = if self.has_profile_roster_evidence() {
            let projection = self.profile_projection();
            projection
                .active_facets
                .values()
                .filter(|facet| facet.is_app_key() && facet.capabilities.can_write_roots)
                .map(|facet| facet.pubkey.clone())
                .collect::<Vec<_>>()
        } else {
            self.app_keys
                .as_ref()
                .map(|projection| {
                    projection
                        .app_actors
                        .iter()
                        .map(|actor| actor.pubkey.clone())
                        .collect()
                })
                .unwrap_or_default()
        };

        if self.can_write_roots() && !app_keys.contains(&self.app_key_pubkey) {
            app_keys.push(self.app_key_pubkey.clone());
        }
        app_keys.sort();
        app_keys.dedup();
        app_keys
    }

    #[must_use]
    pub fn can_write_roots_for_app_key(&self, app_key_pubkey: &str) -> bool {
        if app_key_pubkey == self.app_key_pubkey
            && self.pending_device_approval_receipt_authorizes_current_app_key()
            && !self
                .profile_projection()
                .tombstones
                .contains_key(&self.app_key_pubkey)
        {
            return true;
        }
        if self.has_profile_roster_evidence() {
            return self.profile_projection().can_write_roots(app_key_pubkey);
        }
        self.app_keys
            .as_ref()
            .is_some_and(|projection| projection.contains(app_key_pubkey))
    }

    pub fn sync_app_keys_from_profile(&mut self) -> bool {
        let profile_projection =
            project_nostr_identity_roster(self.profile_id, self.profile_roster_ops.clone());
        let Some(projection) = app_keys_from_profile_projection_with_local_labels(
            &profile_projection,
            self.app_keys.as_ref(),
            Some(&self.app_key_pubkey),
            self.app_key_label.as_deref(),
        ) else {
            let request_count_before = self.inbound_app_key_link_requests.len();
            self.inbound_app_key_link_requests.retain(|request| {
                !request_is_handled_by_profile_projection(request, &profile_projection, None)
            });
            let requests_changed = self.inbound_app_key_link_requests.len() != request_count_before;
            self.profile_roster_projection = Some(profile_projection);
            self.recompute_authorization();
            return requests_changed;
        };
        let app_keys_changed = self.app_keys.as_ref() != Some(&projection);
        let request_count_before = self.inbound_app_key_link_requests.len();
        self.inbound_app_key_link_requests.retain(|request| {
            !request_is_handled_by_profile_projection(
                request,
                &profile_projection,
                Some(&projection),
            )
        });
        let requests_changed = self.inbound_app_key_link_requests.len() != request_count_before;
        let current_app_key_is_tombstoned = profile_projection
            .tombstones
            .contains_key(&self.app_key_pubkey);
        self.recompute_authorization_from_app_keys(&projection, current_app_key_is_tombstoned);
        self.profile_roster_projection = Some(profile_projection);
        self.app_keys = Some(projection);
        app_keys_changed || requests_changed
    }

    /// Adopt the profile UUID carried by local roster evidence when that
    /// evidence unambiguously belongs to one `NostrIdentity`.
    ///
    /// Offline restore creates a fresh temporary UUID because the recovery
    /// secret must not derive identity. Once signed roster ops arrive, their
    /// embedded `profile_id` is the durable identity breadcrumb.
    pub fn adopt_single_roster_profile_id(&mut self) -> bool {
        let Some(profile_id) = single_roster_profile_id(&self.profile_roster_ops) else {
            return false;
        };
        if profile_id == self.profile_id {
            return false;
        }
        self.profile_id = profile_id;
        self.app_keys = None;
        self.profile_roster_projection = None;
        self.sync_app_keys_from_profile();
        true
    }

    /// Is this install's `AppKey` active in the latest roster projection?
    #[must_use]
    pub fn is_authorized(&self) -> bool {
        matches!(
            self.authorization_state,
            AppKeyAuthorizationState::Authorized
        )
    }

    /// Can this install's `AppKey` administer the `NostrIdentity` roster?
    #[must_use]
    pub fn can_admin_profile(&self) -> bool {
        if self.has_profile_roster_evidence() {
            return self
                .profile_projection()
                .can_admin_profile(&self.app_key_pubkey);
        }
        if let Some(app_keys) = &self.app_keys {
            return app_keys.is_admin(&self.app_key_pubkey);
        }
        false
    }

    /// Can this `AppKey` publish mutable roots for this profile?
    #[must_use]
    pub fn can_write_roots(&self) -> bool {
        if self.has_profile_roster_evidence() {
            let projection = self.profile_projection();
            return projection.can_write_roots(&self.app_key_pubkey)
                || (self.pending_device_approval_receipt_authorizes_current_app_key()
                    && !projection.tombstones.contains_key(&self.app_key_pubkey));
        }
        if let Some(app_keys) = &self.app_keys {
            return app_keys.contains(&self.app_key_pubkey);
        }
        matches!(
            self.authorization_state,
            AppKeyAuthorizationState::Authorized
        )
    }

    /// Recompute `authorization_state` from the current profile roster projection.
    pub fn recompute_authorization(&mut self) {
        let projection =
            project_nostr_identity_roster(self.profile_id, self.profile_roster_ops.clone());
        let app_keys = app_keys_from_profile_projection(&projection);
        let current_app_key_has_usable_profile = app_keys
            .as_ref()
            .is_some_and(|keys| keys.contains(&self.app_key_pubkey))
            && projection.can_write_roots(&self.app_key_pubkey);
        let pending_receipt_authorizes_current_app_key =
            self.pending_device_approval_receipt_authorizes_current_app_key();
        self.authorization_state = if current_app_key_has_usable_profile
            || (pending_receipt_authorizes_current_app_key
                && !projection.tombstones.contains_key(&self.app_key_pubkey))
        {
            AppKeyAuthorizationState::Authorized
        } else if projection.tombstones.contains_key(&self.app_key_pubkey)
            || (self.authorization_state == AppKeyAuthorizationState::Authorized
                && app_keys.is_some())
        {
            AppKeyAuthorizationState::Revoked
        } else if self.authorization_state == AppKeyAuthorizationState::Authorized
            && self.has_profile_roster_evidence()
        {
            AppKeyAuthorizationState::AwaitingApproval
        } else {
            self.authorization_state
        };
        if current_app_key_has_usable_profile && !self.has_applied_device_approval_receipt() {
            self.outbound_app_key_link_request = None;
        }
        self.profile_roster_projection = Some(projection);
    }

    fn recompute_authorization_from_app_keys(
        &mut self,
        projection: &AppKeysProjection,
        current_app_key_is_tombstoned: bool,
    ) {
        let current_app_key_is_projected = projection.contains(&self.app_key_pubkey);
        self.authorization_state = if current_app_key_is_projected
            || (self.pending_device_approval_receipt_authorizes_current_app_key()
                && !current_app_key_is_tombstoned)
        {
            AppKeyAuthorizationState::Authorized
        } else if current_app_key_is_tombstoned
            || self.authorization_state == AppKeyAuthorizationState::Authorized
        {
            AppKeyAuthorizationState::Revoked
        } else {
            self.authorization_state
        };
        if current_app_key_is_projected && !self.has_applied_device_approval_receipt() {
            self.outbound_app_key_link_request = None;
        }
    }

    fn has_applied_device_approval_receipt(&self) -> bool {
        self.outbound_app_key_link_request
            .as_ref()
            .and_then(|pending| pending.approval_receipt_event.as_ref())
            .is_some()
    }

    fn pending_device_approval_receipt_authorizes_current_app_key(&self) -> bool {
        let Some(pending) = self.outbound_app_key_link_request.as_ref() else {
            return false;
        };
        crate::app_key_link_transport::pending_app_key_approval_receipt_authorizes_app_key(
            pending,
            &self.app_key_pubkey,
        )
    }

    pub fn queue_outbound_app_key_link_request(
        &mut self,
        admin_app_key_pubkey: String,
        invite_pubkey: &str,
        requested_at: u64,
        request_url: String,
        request_key_secret: String,
    ) -> Result<bool, ProfileError> {
        if !is_pubkey_hex(&admin_app_key_pubkey) {
            return Err(ProfileError::InvalidAppKeyPubkey(admin_app_key_pubkey));
        }
        let invite_pubkey = normalize_required_pubkey(invite_pubkey)?;
        if self.app_key_pubkey == admin_app_key_pubkey {
            return Ok(false);
        }
        let next = PendingAppKeyLinkRequest {
            admin_app_key_pubkey,
            invite_pubkey,
            request_url,
            request_key_secret,
            approval_receipt_event: None,
            requested_at,
        };
        let changed = self.outbound_app_key_link_request.as_ref() != Some(&next);
        self.outbound_app_key_link_request = Some(next);
        Ok(changed)
    }

    pub fn queue_unbound_app_key_join_request(
        &mut self,
        requested_at: u64,
        request_url: String,
        request_key_secret: String,
    ) -> bool {
        let next = PendingAppKeyLinkRequest {
            admin_app_key_pubkey: String::new(),
            invite_pubkey: String::new(),
            request_url,
            request_key_secret,
            approval_receipt_event: None,
            requested_at,
        };
        let changed = self.outbound_app_key_link_request.as_ref() != Some(&next);
        self.outbound_app_key_link_request = Some(next);
        changed
    }

    pub fn record_inbound_app_key_link_request(
        &mut self,
        profile_id: NostrIdentityId,
        app_key_pubkey: &str,
        label: Option<String>,
        invite_pubkey: &str,
        request_url: impl Into<String>,
        requested_at: u64,
    ) -> Result<bool, ProfileError> {
        if profile_id != self.profile_id || !self.can_admin_profile() {
            return Ok(false);
        }
        let invite_pubkey = normalize_required_pubkey(invite_pubkey)?;
        let expected_invite_pubkey = app_key_link_invite_pubkey(&self.app_key_link_secret)?;
        if invite_pubkey != expected_invite_pubkey {
            return Ok(false);
        }
        if !is_pubkey_hex(app_key_pubkey) {
            return Err(ProfileError::InvalidAppKeyPubkey(
                app_key_pubkey.to_string(),
            ));
        }
        let request_url = request_url.into();
        let bootstrap =
            crate::app_key_link_transport::parse_app_key_approval_bootstrap(&request_url)
                .map_err(|error| ProfileError::InvalidAppKeyApprovalBootstrap(error.to_string()))?
                .ok_or_else(|| {
                    ProfileError::InvalidAppKeyApprovalBootstrap(
                        "approval URL does not contain a compact bootstrap".to_string(),
                    )
                })?;
        let bootstrap_app_key = PublicKey::parse(&bootstrap.device_app_key_npub)
            .map_err(|error| ProfileError::InvalidAppKeyApprovalBootstrap(error.to_string()))?
            .to_hex();
        if bootstrap_app_key != app_key_pubkey {
            return Err(ProfileError::InvalidAppKeyApprovalBootstrap(
                "bootstrap device AppKey does not match the requesting peer".to_string(),
            ));
        }
        let request = InboundAppKeyLinkRequest {
            app_key_pubkey: app_key_pubkey.to_string(),
            label: label.and_then(|label| {
                let trimmed = label.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            }),
            invite_pubkey,
            request_url: request_url.trim().to_string(),
            requested_at,
        };
        let profile_projection = self.profile_projection();
        let projection = self
            .app_keys
            .clone()
            .or_else(|| app_keys_from_profile_projection(&profile_projection));
        if app_key_pubkey == self.app_key_pubkey
            || request_is_handled_by_profile_projection(
                &request,
                &profile_projection,
                projection.as_ref(),
            )
            || self.handled_app_key_link_requests.iter().any(|handled| {
                handled.app_key_pubkey == app_key_pubkey && handled.requested_at >= requested_at
            })
        {
            let before = self.inbound_app_key_link_requests.len();
            self.inbound_app_key_link_requests
                .retain(|request| request.app_key_pubkey != app_key_pubkey);
            return Ok(before != self.inbound_app_key_link_requests.len());
        }

        let mut changed = false;
        if let Some(existing) = self
            .inbound_app_key_link_requests
            .iter_mut()
            .find(|request| request.app_key_pubkey == app_key_pubkey)
        {
            let next_requested_at = existing.requested_at.max(requested_at);
            if existing.requested_at != next_requested_at
                || existing.label != request.label
                || existing.invite_pubkey != request.invite_pubkey
                || existing.request_url != request.request_url
            {
                existing.requested_at = next_requested_at;
                existing.label.clone_from(&request.label);
                existing.invite_pubkey.clone_from(&request.invite_pubkey);
                existing.request_url.clone_from(&request.request_url);
                changed = true;
            }
        } else {
            self.inbound_app_key_link_requests.push(request);
            changed = true;
        }

        if self.inbound_app_key_link_requests.len() > MAX_INBOUND_APP_KEY_LINK_REQUESTS {
            self.inbound_app_key_link_requests
                .sort_by_key(|request| request.requested_at);
            while self.inbound_app_key_link_requests.len() > MAX_INBOUND_APP_KEY_LINK_REQUESTS {
                self.inbound_app_key_link_requests.remove(0);
            }
            changed = true;
        }
        self.inbound_app_key_link_requests
            .sort_by(|left, right| left.app_key_pubkey.cmp(&right.app_key_pubkey));
        Ok(changed)
    }

    pub fn reject_inbound_app_key_link_request(
        &mut self,
        app_key_pubkey: &str,
    ) -> Result<bool, ProfileError> {
        if !is_pubkey_hex(app_key_pubkey) {
            return Err(ProfileError::InvalidAppKeyPubkey(
                app_key_pubkey.to_string(),
            ));
        }
        let handled_at = self
            .inbound_app_key_link_requests
            .iter()
            .filter(|request| request.app_key_pubkey == app_key_pubkey)
            .map(|request| request.requested_at)
            .max();
        let before = self.inbound_app_key_link_requests.len();
        self.inbound_app_key_link_requests
            .retain(|request| request.app_key_pubkey != app_key_pubkey);
        let removed = before != self.inbound_app_key_link_requests.len();
        if let Some(requested_at) = handled_at {
            self.remember_handled_app_key_link_request(app_key_pubkey, requested_at);
        }
        Ok(removed)
    }

    pub fn reset_app_key_link_secret(&mut self) -> bool {
        let previous = self.app_key_link_secret.clone();
        self.app_key_link_secret = default_app_key_link_secret();
        let had_requests = !self.inbound_app_key_link_requests.is_empty();
        self.inbound_app_key_link_requests.clear();
        self.handled_app_key_link_requests.clear();
        had_requests || self.app_key_link_secret != previous
    }

    fn remember_handled_app_key_link_request(&mut self, app_key_pubkey: &str, requested_at: u64) {
        if let Some(existing) = self
            .handled_app_key_link_requests
            .iter_mut()
            .find(|request| request.app_key_pubkey == app_key_pubkey)
        {
            existing.requested_at = existing.requested_at.max(requested_at);
        } else {
            self.handled_app_key_link_requests
                .push(HandledAppKeyLinkRequest {
                    app_key_pubkey: app_key_pubkey.to_string(),
                    requested_at,
                });
        }
        if self.handled_app_key_link_requests.len() > MAX_HANDLED_APP_KEY_LINK_REQUESTS {
            self.handled_app_key_link_requests
                .sort_by_key(|request| request.requested_at);
            while self.handled_app_key_link_requests.len() > MAX_HANDLED_APP_KEY_LINK_REQUESTS {
                self.handled_app_key_link_requests.remove(0);
            }
        }
        self.handled_app_key_link_requests
            .sort_by(|left, right| left.app_key_pubkey.cmp(&right.app_key_pubkey));
    }
}

fn request_is_handled_by_profile_projection(
    request: &InboundAppKeyLinkRequest,
    profile_projection: &NostrIdentityRosterProjection,
    app_keys_projection: Option<&AppKeysProjection>,
) -> bool {
    if app_keys_projection.is_some_and(|projection| projection.contains(&request.app_key_pubkey)) {
        return true;
    }
    profile_projection
        .tombstones
        .get(&request.app_key_pubkey)
        .is_some_and(|tombstone| {
            u64::try_from(tombstone.removed_at)
                .is_ok_and(|removed_at| removed_at >= request.requested_at)
        })
}

#[must_use]
pub fn single_roster_profile_id(
    profile_roster_ops: &[SignedNostrIdentityRosterOp],
) -> Option<NostrIdentityId> {
    let mut ids = profile_roster_ops
        .iter()
        .map(|op| op.content.profile_id)
        .collect::<BTreeSet<_>>();
    if ids.len() == 1 {
        ids.pop_first()
    } else {
        None
    }
}

#[must_use]
pub fn app_keys_from_profile_roster(
    profile_id: NostrIdentityId,
    profile_roster_ops: &[SignedNostrIdentityRosterOp],
) -> Option<AppKeysProjection> {
    let projection = project_nostr_identity_roster(profile_id, profile_roster_ops.iter().cloned());
    app_keys_from_profile_projection(&projection)
}

#[must_use]
pub fn app_keys_from_profile_projection(
    projection: &NostrIdentityRosterProjection,
) -> Option<AppKeysProjection> {
    app_keys_from_profile_projection_with_local_labels(projection, None, None, None)
}

#[must_use]
pub fn app_keys_from_profile_projection_with_local_labels(
    projection: &NostrIdentityRosterProjection,
    local_projection: Option<&AppKeysProjection>,
    current_app_key_pubkey: Option<&str>,
    current_app_key_label: Option<&str>,
) -> Option<AppKeysProjection> {
    let key_epoch = projection.secret_epochs.values().next_back()?;
    let app_key_pubkeys: BTreeSet<_> = projection.active_app_key_pubkeys().into_iter().collect();
    if app_key_pubkeys.is_empty() {
        return None;
    }
    if !projection
        .active_facets
        .values()
        .any(|facet| facet.is_app_key() && facet.capabilities.can_admin_profile)
    {
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
                label: local_app_key_label(
                    &facet.pubkey,
                    local_projection,
                    current_app_key_pubkey,
                    current_app_key_label,
                ),
                role,
            }
        })
        .collect::<Vec<_>>();
    app_actors.sort_by(|left, right| left.pubkey.cmp(&right.pubkey));
    let wrapped_dck = key_epoch
        .wrapped_secrets
        .iter()
        .filter(|(pubkey, _)| app_key_pubkeys.contains(*pubkey))
        .map(|(pubkey, wrap)| (pubkey.clone(), wrap.clone()))
        .collect();
    Some(AppKeysProjection {
        profile_id: projection.profile_id.to_string(),
        signed_by_pubkey: Some(key_epoch.signed_by_pubkey.clone()),
        created_at: key_epoch.created_at,
        app_actors,
        dck_generation: key_epoch.epoch,
        wrapped_dck,
    })
}

fn local_app_key_label(
    pubkey: &str,
    local_projection: Option<&AppKeysProjection>,
    current_app_key_pubkey: Option<&str>,
    current_app_key_label: Option<&str>,
) -> Option<String> {
    if current_app_key_pubkey == Some(pubkey)
        && let Some(label) = current_app_key_label.and_then(normalize_app_key_label)
    {
        return Some(label);
    }
    local_projection
        .and_then(|projection| {
            projection
                .app_actors
                .iter()
                .find(|actor| actor.pubkey == pubkey)
        })
        .and_then(|actor| actor.label.as_deref())
        .and_then(normalize_app_key_label)
}

fn app_key_labels_from_projection(
    projection: Option<&AppKeysProjection>,
) -> BTreeMap<String, String> {
    projection.map_or_else(BTreeMap::new, |projection| {
        projection
            .app_actors
            .iter()
            .filter_map(|actor| {
                actor
                    .label
                    .as_deref()
                    .and_then(normalize_app_key_label)
                    .map(|label| (actor.pubkey.clone(), label))
            })
            .collect()
    })
}

fn apply_app_key_labels_to_projection(
    projection: &mut AppKeysProjection,
    labels: &BTreeMap<String, String>,
) -> bool {
    let mut changed = false;
    for actor in &mut projection.app_actors {
        let Some(label) = labels
            .get(&actor.pubkey)
            .and_then(|label| normalize_app_key_label(label))
        else {
            continue;
        };
        if actor.label.as_deref() != Some(label.as_str()) {
            actor.label = Some(label);
            changed = true;
        }
    }
    changed
}

/// In-memory profile: persisted state + the keypairs it references.
pub struct Profile {
    pub state: ProfileState,
    pub app_key: AppKey,
}

impl Profile {
    fn current_app_keys_projection(&self) -> Result<&AppKeysProjection, ProfileError> {
        self.state
            .app_keys
            .as_ref()
            .ok_or(ProfileError::NoCurrentAppKeysProjection)
    }

    fn sync_app_keys_from_profile_with_device_labels(&mut self) -> bool {
        let changed = self.state.sync_app_keys_from_profile();
        let Ok(dck) = self.current_dck() else {
            return changed;
        };
        let labels = self.decrypted_device_labels_with_dck(&dck);
        let labels_changed = self
            .state
            .app_keys
            .as_mut()
            .is_some_and(|projection| apply_app_key_labels_to_projection(projection, &labels));
        changed || labels_changed
    }

    fn decrypted_device_labels_with_dck(&self, dck: &[u8; 32]) -> BTreeMap<String, String> {
        let mut payloads = self
            .state
            .profile_roster_ops
            .iter()
            .flat_map(|op| {
                Event::from_json(&op.event_json)
                    .ok()
                    .into_iter()
                    .flat_map(|event| {
                        encrypted_device_label_payloads_from_nostr_identity_roster_op_event(&event)
                    })
                    .filter_map(|ciphertext| {
                        decrypt_drive_device_labels_with_dck(&ciphertext, dck).ok()
                    })
            })
            .filter(|payload| payload.profile_id == self.state.profile_id)
            .collect::<Vec<_>>();
        payloads.sort_by(|left, right| {
            left.updated_at
                .cmp(&right.updated_at)
                .then_with(|| left.secret_epoch.cmp(&right.secret_epoch))
        });

        let mut labels = BTreeMap::new();
        for payload in payloads {
            labels.extend(payload.labels);
        }
        normalize_drive_device_labels(labels)
    }

    fn app_key_labels_for_payload(
        &self,
        dck: Option<&[u8; 32]>,
        extra_labels: BTreeMap<String, String>,
        removed_pubkeys: &[&str],
    ) -> BTreeMap<String, String> {
        let mut labels = dck.map_or_else(BTreeMap::new, |dck| {
            self.decrypted_device_labels_with_dck(dck)
        });
        labels.extend(app_key_labels_from_projection(self.state.app_keys.as_ref()));
        if let Some(label) = self
            .state
            .app_key_label
            .as_deref()
            .and_then(normalize_app_key_label)
        {
            labels.insert(self.state.app_key_pubkey.clone(), label);
        }
        let extra_pubkeys = extra_labels.keys().cloned().collect::<BTreeSet<_>>();
        labels.extend(extra_labels);
        for pubkey in removed_pubkeys {
            labels.remove(*pubkey);
        }
        let active_pubkeys = self
            .state
            .profile_projection()
            .active_app_key_pubkeys()
            .into_iter()
            .collect::<BTreeSet<_>>();
        labels
            .retain(|pubkey, _| active_pubkeys.contains(pubkey) || extra_pubkeys.contains(pubkey));
        normalize_drive_device_labels(labels)
    }

    fn encrypted_device_labels_for_dck(
        &self,
        dck: &[u8; 32],
        secret_epoch: u64,
        updated_at: i64,
        extra_labels: BTreeMap<String, String>,
        removed_pubkeys: &[&str],
    ) -> Result<Option<String>, ProfileError> {
        let labels = self.app_key_labels_for_payload(Some(dck), extra_labels, removed_pubkeys);
        encrypted_device_labels_from_map(
            self.state.profile_id,
            secret_epoch,
            labels,
            dck,
            updated_at,
        )
    }

    fn encrypted_current_device_labels(
        &self,
        updated_at: i64,
    ) -> Result<Option<String>, ProfileError> {
        let Ok(dck) = self.current_dck() else {
            return Ok(None);
        };
        let secret_epoch =
            next_profile_secret_epoch(&self.state.profile_projection()).saturating_sub(1);
        self.encrypted_device_labels_for_dck(&dck, secret_epoch, updated_at, BTreeMap::new(), &[])
    }

    /// **Create** flow — fresh `AppKey` saved to the config dir. The `AppKey` is
    /// auto-authorized as the first admin via a self-signed single-entry
    /// `NostrIdentity` roster op log.
    pub fn create(config_dir: &Path, app_key_label: Option<String>) -> Result<Self, ProfileError> {
        let profile_id = NostrIdentityId::new_v4();
        let device = AppKey::generate(key_path_in(config_dir));
        device.save()?;
        let app_key_label = resolve_app_key_label(app_key_label, &device.pubkey_hex());

        let mut state = ProfileState {
            profile_id,
            app_key_pubkey: device.pubkey_hex(),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: default_app_key_link_secret(),
            authorization_state: AppKeyAuthorizationState::AwaitingApproval,
            app_key_label: app_key_label.clone(),
            app_keys: None,
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
            pending_device_approval_receipts: Vec::new(),
        };

        let now = current_unix_seconds();
        let app_actor = AppActorEntry::admin(state.app_key_pubkey.clone(), now, app_key_label);
        let dck = generate_dck();
        state.profile_roster_ops =
            initial_profile_roster_ops(device.keys(), profile_id, &app_actor, None, &dck, now)?;

        let mut profile = Self {
            state,
            app_key: device,
        };
        profile.sync_app_keys_from_profile_with_device_labels();
        Ok(profile)
    }

    /// **Restore** flow — use a recovery phrase or recovery secret key to
    /// recover profile authority while generating a fresh per-install `AppKey`.
    pub fn restore(
        config_dir: &Path,
        recovery_secret: &str,
        app_key_label: Option<String>,
    ) -> Result<Self, ProfileError> {
        let recovery_phrase = validate_recovery_phrase(recovery_secret).ok();
        let recovery_key = if let Some(phrase) = recovery_phrase.as_deref() {
            RecoveryKey::from_recovery_phrase(phrase, PathBuf::new())?
        } else {
            RecoveryKey::from_secret(recovery_secret, PathBuf::new())?
        };
        let profile_id = NostrIdentityId::new_v4();
        let device = AppKey::generate(key_path_in(config_dir));
        device.save()?;
        if let Some(phrase) = recovery_phrase.as_deref() {
            save_recovery_phrase(recovery_phrase_path_in(config_dir), phrase)?;
        }
        let app_key_label = resolve_app_key_label(app_key_label, &device.pubkey_hex());

        let mut state = ProfileState {
            profile_id,
            app_key_pubkey: device.pubkey_hex(),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: default_app_key_link_secret(),
            authorization_state: AppKeyAuthorizationState::AwaitingApproval,
            app_key_label: app_key_label.clone(),
            app_keys: None,
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
            pending_device_approval_receipts: Vec::new(),
        };

        let now = current_unix_seconds();
        let app_actor = AppActorEntry::admin(state.app_key_pubkey.clone(), now, app_key_label);
        let dck = generate_dck();
        let recovery_pubkey = recovery_key.pubkey_hex();
        state.profile_roster_ops = initial_profile_roster_ops(
            device.keys(),
            profile_id,
            &app_actor,
            Some(&recovery_pubkey),
            &dck,
            now,
        )?;
        let mut profile = Self {
            state,
            app_key: device,
        };
        profile.sync_app_keys_from_profile_with_device_labels();
        Ok(profile)
    }

    /// Restore an existing `NostrIdentity` when the UUID and roster log came
    /// from verified evidence such as relay roster ops, an invite, or an
    /// export. The recovery secret proves authority; it does not determine the
    /// UUID.
    pub fn restore_with_profile_roster_ops(
        config_dir: &Path,
        recovery_secret: &str,
        profile_id: NostrIdentityId,
        profile_roster_ops: Vec<SignedNostrIdentityRosterOp>,
        app_key_label: Option<String>,
    ) -> Result<Self, ProfileError> {
        let recovery_phrase = validate_recovery_phrase(recovery_secret).ok();
        let recovery_key = if let Some(phrase) = recovery_phrase.as_deref() {
            RecoveryKey::from_recovery_phrase(phrase, PathBuf::new())?
        } else {
            RecoveryKey::from_secret(recovery_secret, PathBuf::new())?
        };
        let authority_pubkey = recovery_key.pubkey_hex();
        let projection = project_nostr_identity_roster(profile_id, profile_roster_ops.clone());
        let expected_purpose = recovery_authority_purpose(
            &projection,
            &authority_pubkey,
            recovery_phrase
                .as_ref()
                .map(|_| NostrIdentityKeyPurpose::RecoveryPhrase),
        )?;

        let device = AppKey::generate(key_path_in(config_dir));
        device.save()?;
        if let Some(phrase) = recovery_phrase.as_deref() {
            save_recovery_phrase(recovery_phrase_path_in(config_dir), phrase)?;
        }
        let app_key_label = resolve_app_key_label(app_key_label, &device.pubkey_hex());
        let mut state = ProfileState {
            profile_id,
            app_key_pubkey: device.pubkey_hex(),
            profile_roster_ops,
            app_key_link_secret: default_app_key_link_secret(),
            authorization_state: AppKeyAuthorizationState::AwaitingApproval,
            app_key_label: app_key_label.clone(),
            app_keys: None,
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
            pending_device_approval_receipts: Vec::new(),
        };
        state.sync_app_keys_from_profile();

        let mut profile = Self {
            state,
            app_key: device,
        };
        profile.admit_current_app_key_with_authority_keys(
            recovery_key.keys(),
            expected_purpose,
            app_key_label,
        )?;
        Ok(profile)
    }

    /// Reconcile a fresh fallback restore with later-discovered roster
    /// evidence. The current install keeps its fresh `AppKey`; the recovery
    /// secret signs admission of that `AppKey` into the discovered profile.
    pub fn reconcile_with_profile_roster_ops(
        &mut self,
        recovery_secret: &str,
        profile_id: NostrIdentityId,
        profile_roster_ops: Vec<SignedNostrIdentityRosterOp>,
        label: Option<String>,
    ) -> Result<AppKeysProjection, ProfileError> {
        let recovery_phrase = validate_recovery_phrase(recovery_secret).ok();
        let recovery_key = if let Some(phrase) = recovery_phrase.as_deref() {
            RecoveryKey::from_recovery_phrase(phrase, PathBuf::new())?
        } else {
            RecoveryKey::from_secret(recovery_secret, PathBuf::new())?
        };
        let projection = project_nostr_identity_roster(profile_id, profile_roster_ops.clone());
        let expected_purpose = recovery_authority_purpose(
            &projection,
            &recovery_key.pubkey_hex(),
            recovery_phrase
                .as_ref()
                .map(|_| NostrIdentityKeyPurpose::RecoveryPhrase),
        )?;
        self.reconcile_with_profile_roster_ops_and_authority_keys(
            recovery_key.keys(),
            expected_purpose,
            profile_id,
            profile_roster_ops,
            label,
        )
    }

    /// Reconcile a fresh fallback restore with later-discovered NIP-46 roster
    /// evidence. This is the local-keys test surface for the same authority
    /// path a remote NIP-46 signer will drive.
    pub fn reconcile_with_profile_roster_ops_using_nip46_keys(
        &mut self,
        nip46_keys: &Keys,
        profile_id: NostrIdentityId,
        profile_roster_ops: Vec<SignedNostrIdentityRosterOp>,
        label: Option<String>,
    ) -> Result<AppKeysProjection, ProfileError> {
        let projection = project_nostr_identity_roster(profile_id, profile_roster_ops.clone());
        let expected_purpose = recovery_authority_purpose(
            &projection,
            &nip46_keys.public_key().to_hex(),
            Some(NostrIdentityKeyPurpose::Nip46Signer),
        )?;
        self.reconcile_with_profile_roster_ops_and_authority_keys(
            nip46_keys,
            expected_purpose,
            profile_id,
            profile_roster_ops,
            label,
        )
    }

    fn reconcile_with_profile_roster_ops_and_authority_keys(
        &mut self,
        authority_keys: &Keys,
        expected_purpose: NostrIdentityKeyPurpose,
        profile_id: NostrIdentityId,
        profile_roster_ops: Vec<SignedNostrIdentityRosterOp>,
        label: Option<String>,
    ) -> Result<AppKeysProjection, ProfileError> {
        let original_state = self.state.clone();
        self.state.profile_id = profile_id;
        self.state.profile_roster_ops = profile_roster_ops;
        self.state.app_keys = None;
        self.state.profile_roster_projection = None;
        self.state.authorization_state = AppKeyAuthorizationState::AwaitingApproval;
        self.state.outbound_app_key_link_request = None;
        self.state.inbound_app_key_link_requests.clear();
        self.state.handled_app_key_link_requests.clear();
        self.sync_app_keys_from_profile_with_device_labels();

        if self
            .state
            .profile_projection()
            .can_write_roots(&self.state.app_key_pubkey)
        {
            return self
                .state
                .app_keys
                .clone()
                .ok_or(ProfileError::NoCurrentAppKeysProjection);
        }

        match self.admit_current_app_key_with_authority_keys(
            authority_keys,
            expected_purpose,
            label,
        ) {
            Ok(snapshot) => Ok(snapshot.clone()),
            Err(error) => {
                self.state = original_state;
                Err(error)
            }
        }
    }

    /// **Link** flow — generate a fresh `AppKey` for a known `NostrIdentity`.
    /// The admin `AppKey` is used only as the request target; the new local
    /// `AppKey` starts in `AwaitingApproval` until a roster admin accepts it.
    pub fn link_to_profile(
        config_dir: &Path,
        profile_id: NostrIdentityId,
        admin_app_key_hex: String,
        app_key_label: Option<String>,
    ) -> Result<Self, ProfileError> {
        if !is_pubkey_hex(&admin_app_key_hex) {
            return Err(ProfileError::InvalidAppKeyPubkey(admin_app_key_hex));
        }
        let device = AppKey::generate(key_path_in(config_dir));
        device.save()?;
        let app_key_label = resolve_app_key_label(app_key_label, &device.pubkey_hex());

        let state = ProfileState {
            profile_id,
            app_key_pubkey: device.pubkey_hex(),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: default_app_key_link_secret(),
            authorization_state: AppKeyAuthorizationState::AwaitingApproval,
            app_key_label,
            app_keys: None,
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
            pending_device_approval_receipts: Vec::new(),
        };

        Ok(Self {
            state,
            app_key: device,
        })
    }

    /// Start a manual join request when the joining device does not yet know
    /// which profile/admin will approve it. The first signed roster returned
    /// by an admin binds this install to the real profile id.
    pub fn start_join_request(
        config_dir: &Path,
        app_key_label: Option<String>,
    ) -> Result<Self, ProfileError> {
        let profile_id = NostrIdentityId::new_v4();
        let device = AppKey::generate(key_path_in(config_dir));
        device.save()?;
        let app_key_label = resolve_app_key_label(app_key_label, &device.pubkey_hex());

        let state = ProfileState {
            profile_id,
            app_key_pubkey: device.pubkey_hex(),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: default_app_key_link_secret(),
            authorization_state: AppKeyAuthorizationState::AwaitingApproval,
            app_key_label,
            app_keys: None,
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
            pending_device_approval_receipts: Vec::new(),
        };

        Ok(Self {
            state,
            app_key: device,
        })
    }

    /// Load a profile from its config dir. Reconstructs the in-memory
    /// view from the persisted `ProfileState` plus the on-disk key
    /// files. Errors if the app key is missing — caller should run a
    /// create/restore/link flow first.
    pub fn load(state: ProfileState, config_dir: &Path) -> Result<Self, ProfileError> {
        let device = AppKey::load(key_path_in(config_dir))?;
        let mut profile = Self {
            state,
            app_key: device,
        };
        profile.sync_app_keys_from_profile_with_device_labels();
        Ok(profile)
    }

    /// Approve a new `AppKey` by appending it to the roster
    /// and rotating the DCK so the new `AppKey` gets a fresh wrap.
    /// Bumps `created_at` and `dck_generation`. Callers should fan the
    /// new signed roster ops out over Nostr.
    pub fn approve_app_key(
        &mut self,
        app_key_pubkey_hex: &str,
        label: Option<String>,
    ) -> Result<&AppKeysProjection, ProfileError> {
        self.approve_app_key_with_client_nonce(app_key_pubkey_hex, label, None)
    }

    fn approve_app_key_with_client_nonce(
        &mut self,
        app_key_pubkey_hex: &str,
        label: Option<String>,
        client_nonce: Option<String>,
    ) -> Result<&AppKeysProjection, ProfileError> {
        if !self.state.can_admin_profile() {
            return Err(ProfileError::NoAdminAuthority);
        }
        let already_authorized = self
            .state
            .app_keys
            .as_ref()
            .is_some_and(|snap| snap.contains(app_key_pubkey_hex));
        if already_authorized {
            let before = self.state.inbound_app_key_link_requests.len();
            self.state
                .inbound_app_key_link_requests
                .retain(|request| request.app_key_pubkey != app_key_pubkey_hex);
            if before != self.state.inbound_app_key_link_requests.len() {
                return self.current_app_keys_projection();
            }
            return Err(ProfileError::AppKeyAlreadyAuthorized);
        }
        let now = next_profile_timestamp(&self.state);
        let dck = generate_dck();
        let secret_epoch = next_profile_secret_epoch(&self.state.profile_projection());
        let app_key_label = label.and_then(|label| normalize_app_key_label(&label));
        let encrypted_device_labels = self.encrypted_device_labels_for_dck(
            &dck,
            secret_epoch,
            now,
            app_key_label
                .map(|label| BTreeMap::from([(app_key_pubkey_hex.to_string(), label)]))
                .unwrap_or_default(),
            &[],
        )?;
        let parents = nostr_identity_roster_parent_ids(&self.state.profile_roster_ops);
        let op = NostrIdentityRosterOp::AddFacet {
            facet: NostrIdentityFacet::app_key(
                app_key_pubkey_hex.to_string(),
                now,
                None,
                NostrIdentityCapabilities::app_writer(),
            ),
        };
        let signed = if let Some(client_nonce) = client_nonce {
            let event = build_nostr_identity_roster_op_event_with_client_nonce(
                self.app_key.keys(),
                self.state.profile_id,
                parents,
                None,
                op,
                now,
                client_nonce,
                encrypted_device_labels,
            )?;
            parse_nostr_identity_roster_op_event(&event)?
        } else {
            signed_profile_roster_op_with_parents_and_device_labels(
                self.app_key.keys(),
                self.state.profile_id,
                parents,
                op,
                now,
                encrypted_device_labels,
            )?
        };
        self.state.profile_roster_ops.push(signed);
        self.state.profile_roster_projection = None;
        self.rotate_profile_dck_epoch(&dck, now + 1)?;
        self.sync_app_keys_from_profile_with_device_labels();
        self.state
            .inbound_app_key_link_requests
            .retain(|request| request.app_key_pubkey != app_key_pubkey_hex);
        self.current_app_keys_projection()
    }

    pub fn approve_device_bootstrap(
        &mut self,
        bootstrap: &NostrIdentityDeviceApprovalBootstrap,
        label: Option<String>,
    ) -> Result<&AppKeysProjection, ProfileError> {
        if !self.state.can_admin_profile() {
            return Err(ProfileError::NoAdminAuthority);
        }
        let approval = approve_nostr_identity_device_approval_bootstrap(
            ApproveNostrIdentityDeviceApprovalBootstrapOptions {
                bootstrap: bootstrap.clone(),
                profile_id: self.state.profile_id,
                roster_ops: self.state.profile_roster_ops.clone(),
                approved_by_pubkey: self.state.app_key_pubkey.clone(),
                approved_at: next_profile_timestamp(&self.state),
                client_nonce: None,
                capabilities: Some(NostrIdentityCapabilities::app_writer()),
            },
        )?;
        let NostrIdentityRosterOp::AddFacet { facet } = &approval.op else {
            return Err(ProfileError::InvalidAppKeyApprovalBootstrap(
                "canonical approval did not add an AppKey facet".to_string(),
            ));
        };
        let device_app_key_pubkey = facet.pubkey.clone();
        let label = label.or_else(|| bootstrap.label.clone());
        self.approve_app_key_with_client_nonce(
            &device_app_key_pubkey,
            label,
            Some(approval.client_nonce),
        )?;
        let approval_op = self
            .state
            .profile_roster_ops
            .iter()
            .rev()
            .find(|signed| {
                matches!(
                    &signed.content.op,
                    NostrIdentityRosterOp::AddFacet { facet }
                        if facet.pubkey == device_app_key_pubkey
                )
            })
            .cloned()
            .ok_or(ProfileError::AppKeyNotInRoster)?;
        let request_pubkey = PublicKey::parse(&bootstrap.request_npub)
            .map_err(|error| ProfileError::InvalidAppKeyApprovalBootstrap(error.to_string()))?
            .to_hex();
        let receipt = NostrIdentityDeviceApprovalReceipt {
            schema: NOSTR_IDENTITY_DEVICE_APPROVAL_RECEIPT_SCHEMA,
            profile_id: self.state.profile_id,
            request_pubkey: request_pubkey.clone(),
            device_app_key_pubkey: device_app_key_pubkey.clone(),
            approved_by_pubkey: self.state.app_key_pubkey.clone(),
            approved_at: approval_op.content.created_at,
            request_secret: bootstrap.request_secret.clone(),
            subject_pubkey: Some(self.state.app_key_pubkey.clone()),
            roster_op_id: Some(approval_op.op_id.clone()),
            signed_roster_event: Some(approval_op.event_json.clone()),
        };
        let event =
            build_nostr_identity_device_approval_receipt_event(self.app_key.keys(), receipt)?;
        self.state
            .pending_device_approval_receipts
            .retain(|pending| pending.request_pubkey != request_pubkey);
        self.state
            .pending_device_approval_receipts
            .push(PendingDeviceApprovalReceipt {
                request_pubkey,
                device_app_key_pubkey,
                relay_url: String::new(),
                event_json: event.as_json(),
            });
        let overflow = self
            .state
            .pending_device_approval_receipts
            .len()
            .saturating_sub(MAX_PENDING_DEVICE_APPROVAL_RECEIPTS);
        if overflow > 0 {
            self.state
                .pending_device_approval_receipts
                .drain(..overflow);
        }
        self.current_app_keys_projection()
    }

    /// Revoke an `AppKey` from the roster and rotate the DCK so the
    /// revoked `AppKey` cannot decrypt any subsequent content. Bumps
    /// `created_at` and `dck_generation`.
    pub fn revoke_app_key(
        &mut self,
        app_key_pubkey_hex: &str,
    ) -> Result<&AppKeysProjection, ProfileError> {
        if !self.state.can_admin_profile() {
            return Err(ProfileError::NoAdminAuthority);
        }
        let snap = self
            .state
            .app_keys
            .as_ref()
            .ok_or(ProfileError::AppKeyNotInRoster)?;
        if !snap.contains(app_key_pubkey_hex) {
            return Err(ProfileError::AppKeyNotInRoster);
        }
        if snap.is_admin(app_key_pubkey_hex)
            && snap.app_actors.iter().filter(|d| d.is_admin()).count() <= 1
        {
            return Err(ProfileError::CannotRemoveLastAdmin);
        }
        let now = next_profile_timestamp(&self.state);
        let dck = generate_dck();
        let secret_epoch = next_profile_secret_epoch(&self.state.profile_projection());
        let encrypted_device_labels = self.encrypted_device_labels_for_dck(
            &dck,
            secret_epoch,
            now,
            BTreeMap::new(),
            &[app_key_pubkey_hex],
        )?;
        let parents = nostr_identity_roster_parent_ids(&self.state.profile_roster_ops);
        let signed = signed_profile_roster_op_with_parents_and_device_labels(
            self.app_key.keys(),
            self.state.profile_id,
            parents,
            NostrIdentityRosterOp::TombstoneFacet {
                pubkey: app_key_pubkey_hex.to_string(),
                reason: None,
            },
            now,
            encrypted_device_labels,
        )?;
        self.state.profile_roster_ops.push(signed);
        self.state.profile_roster_projection = None;
        self.rotate_profile_dck_epoch(&dck, now + 1)?;
        self.sync_app_keys_from_profile_with_device_labels();
        self.current_app_keys_projection()
    }

    /// Revoke an `AppKey` with a recovery phrase, then rotate the DCK so the
    /// removed `AppKey` loses access to future content.
    pub fn revoke_app_key_with_recovery_phrase(
        &mut self,
        recovery_phrase: &str,
        app_key_pubkey_hex: &str,
    ) -> Result<&AppKeysProjection, ProfileError> {
        let recovery_phrase = validate_recovery_phrase(recovery_phrase)?;
        let recovery_key = RecoveryKey::from_recovery_phrase(&recovery_phrase, PathBuf::new())?;
        self.revoke_app_key_with_authority_keys(
            recovery_key.keys(),
            NostrIdentityKeyPurpose::RecoveryPhrase,
            app_key_pubkey_hex,
        )
    }

    /// Revoke an `AppKey` with a recovery secret. Accepts a 12-word recovery
    /// phrase, nsec1, or 64-char hex secret and matches the active recovery
    /// authority purpose already recorded in the profile.
    pub fn revoke_app_key_with_recovery_secret(
        &mut self,
        recovery_secret: &str,
        app_key_pubkey_hex: &str,
    ) -> Result<&AppKeysProjection, ProfileError> {
        let recovery_phrase = validate_recovery_phrase(recovery_secret).ok();
        let recovery_key = if let Some(phrase) = recovery_phrase.as_deref() {
            RecoveryKey::from_recovery_phrase(phrase, PathBuf::new())?
        } else {
            RecoveryKey::from_secret(recovery_secret, PathBuf::new())?
        };
        let authority_pubkey = recovery_key.pubkey_hex();
        let expected_purpose = recovery_authority_purpose(
            &self.state.profile_projection(),
            &authority_pubkey,
            recovery_phrase
                .as_ref()
                .map(|_| NostrIdentityKeyPurpose::RecoveryPhrase),
        )?;
        self.revoke_app_key_with_authority_keys(
            recovery_key.keys(),
            expected_purpose,
            app_key_pubkey_hex,
        )
    }

    /// Revoke an `AppKey` with a configured NIP-46 recovery authority. The
    /// caller owns the actual remote-signer transport; this method receives the
    /// already-authorized signing/decryption keys used by tests and local flows.
    pub fn revoke_app_key_with_nip46_keys(
        &mut self,
        nip46_keys: &Keys,
        app_key_pubkey_hex: &str,
    ) -> Result<&AppKeysProjection, ProfileError> {
        self.revoke_app_key_with_authority_keys(
            nip46_keys,
            NostrIdentityKeyPurpose::Nip46Signer,
            app_key_pubkey_hex,
        )
    }

    fn revoke_app_key_with_authority_keys(
        &mut self,
        authority_keys: &Keys,
        expected_purpose: NostrIdentityKeyPurpose,
        app_key_pubkey_hex: &str,
    ) -> Result<&AppKeysProjection, ProfileError> {
        let target_pubkey = PublicKey::from_hex(app_key_pubkey_hex)
            .map_err(|e| ProfileError::InvalidAppKeyPubkey(e.to_string()))?
            .to_hex();
        let authority_pubkey = authority_keys.public_key().to_hex();
        {
            let projection = self.state.profile_projection();
            let Some(authority_facet) = projection.active_facets.get(&authority_pubkey) else {
                return Err(ProfileError::RecoveryAuthorityUnavailable);
            };
            if !authority_facet.has_purpose(expected_purpose)
                || !authority_facet.capabilities.can_recover_app_keys
            {
                return Err(ProfileError::RecoveryAuthorityUnavailable);
            }
            if !authority_facet.capabilities.can_decrypt_secret_epochs {
                return Err(ProfileError::RecoveryAuthorityUnavailable);
            }
            if !authority_facet.capabilities.can_change_secret_epochs() {
                return Err(ProfileError::RecoveryCannotRotateSecretEpochs);
            }
            let Some(target_facet) = projection.active_facets.get(&target_pubkey) else {
                return Err(ProfileError::AppKeyNotInRoster);
            };
            if !target_facet.is_app_key() {
                return Err(ProfileError::AppKeyNotInRoster);
            }
            if target_facet.capabilities.can_admin_profile {
                let active_admin_count = projection
                    .active_facets
                    .values()
                    .filter(|facet| facet.is_app_key() && facet.capabilities.can_admin_profile)
                    .count();
                if active_admin_count <= 1 {
                    return Err(ProfileError::CannotRemoveLastAdmin);
                }
            }
        }

        self.current_dck_from_authority_keys(authority_keys, expected_purpose)?;
        let now = next_profile_timestamp(&self.state);
        let parents = nostr_identity_roster_parent_ids(&self.state.profile_roster_ops);
        let remove_op = signed_profile_roster_op_with_parents(
            authority_keys,
            self.state.profile_id,
            parents,
            NostrIdentityRosterOp::TombstoneFacet {
                pubkey: target_pubkey,
                reason: None,
            },
            now,
        )?;
        self.state.profile_roster_ops.push(remove_op);
        self.state.profile_roster_projection = None;

        let dck = generate_dck();
        self.rotate_profile_dck_epoch_with_signer(authority_keys, &dck, now + 1)?;
        self.sync_app_keys_from_profile_with_device_labels();
        self.current_app_keys_projection()
    }

    /// Rotate the DCK without changing the `AppKey` roster. Useful for
    /// periodic key freshness ("rotate weekly even with no membership
    /// churn"). Admin-only.
    pub fn rotate_dck(&mut self) -> Result<&AppKeysProjection, ProfileError> {
        if !self.state.can_admin_profile() {
            return Err(ProfileError::NoAdminAuthority);
        }
        let snap = self
            .state
            .app_keys
            .as_ref()
            .ok_or(ProfileError::NoCurrentAppKeysProjection)?;
        let dck = generate_dck();
        let now = next_profile_timestamp(&self.state).max(next_local_timestamp(Some(snap)));
        self.rotate_profile_dck_epoch(&dck, now)?;
        self.sync_app_keys_from_profile_with_device_labels();
        self.current_app_keys_projection()
    }

    /// Add a NIP-46 signer as an `NostrIdentity` recovery authority. When
    /// `can_decrypt_secret_epochs` is true, the current admin rotates the key
    /// epoch so the signer receives a DCK wrap immediately.
    pub fn add_nip46_recovery(
        &mut self,
        nip46_pubkey_hex: &str,
        label: Option<String>,
        can_decrypt_secret_epochs: bool,
    ) -> Result<(), ProfileError> {
        if !self.state.can_admin_profile() {
            return Err(ProfileError::NoAdminAuthority);
        }
        PublicKey::from_hex(nip46_pubkey_hex)
            .map_err(|e| ProfileError::InvalidAppKeyPubkey(e.to_string()))?;
        let projection = self.state.profile_projection();
        if projection.active_facets.contains_key(nip46_pubkey_hex) {
            return Err(ProfileError::AppKeyAlreadyAuthorized);
        }
        if projection.tombstones.contains_key(nip46_pubkey_hex) {
            return Err(ProfileError::CurrentAppKeyTombstoned);
        }

        let now = next_profile_timestamp(&self.state);
        self.append_profile_roster_op(
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::nip46(
                    nip46_pubkey_hex.to_string(),
                    now,
                    label,
                    can_decrypt_secret_epochs,
                ),
            },
            now,
        )?;
        if can_decrypt_secret_epochs {
            let dck = generate_dck();
            self.rotate_profile_dck_epoch(&dck, now + 1)?;
        }
        self.sync_app_keys_from_profile_with_device_labels();
        Ok(())
    }

    /// Add a recovery phrase as an `NostrIdentity` recovery authority by
    /// deriving its public key. This does not save the phrase.
    pub fn add_recovery_phrase(&mut self, recovery_phrase: &str) -> Result<String, ProfileError> {
        let recovery_phrase = validate_recovery_phrase(recovery_phrase)?;
        let recovery_key = RecoveryKey::from_recovery_phrase(&recovery_phrase, PathBuf::new())?;
        self.add_recovery_pubkey(&recovery_key.pubkey_hex())
    }

    /// Add an already-generated recovery public key. The seed/private material
    /// never needs to be stored by Iris Drive after the user writes it down.
    pub fn add_recovery_pubkey(
        &mut self,
        recovery_pubkey_hex: &str,
    ) -> Result<String, ProfileError> {
        if !self.state.can_admin_profile() {
            return Err(ProfileError::NoAdminAuthority);
        }
        let recovery_pubkey = PublicKey::from_hex(recovery_pubkey_hex)
            .map_err(|e| ProfileError::InvalidAppKeyPubkey(e.to_string()))?
            .to_hex();
        let projection = self.state.profile_projection();
        if let Some(facet) = projection.active_facets.get(&recovery_pubkey) {
            if facet.has_purpose(NostrIdentityKeyPurpose::RecoveryPhrase) {
                return Ok(recovery_pubkey);
            }
            return Err(ProfileError::AppKeyAlreadyAuthorized);
        }
        if projection.tombstones.contains_key(&recovery_pubkey) {
            return Err(ProfileError::CurrentAppKeyTombstoned);
        }

        let now = next_profile_timestamp(&self.state);
        self.append_profile_roster_op(
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::recovery_phrase(recovery_pubkey.clone(), now),
            },
            now,
        )?;
        let dck = generate_dck();
        self.rotate_profile_dck_epoch(&dck, now + 1)?;
        self.sync_app_keys_from_profile_with_device_labels();
        Ok(recovery_pubkey)
    }

    /// Use the profile's recovery phrase authority to admit this install's
    /// fresh `AppKey` into an already-known `NostrIdentity` roster.
    ///
    /// The recovery phrase stays a recovery/admin facet only: it proves it can
    /// decrypt the current epoch, signs the `AppKey` admission, then signs a
    /// coherent new key epoch that rewraps the same DCK to every active recipient.
    pub fn admit_current_app_key_with_recovery_phrase(
        &mut self,
        recovery_phrase: &str,
        label: Option<String>,
    ) -> Result<&AppKeysProjection, ProfileError> {
        let recovery_phrase = validate_recovery_phrase(recovery_phrase)?;
        let recovery_key = RecoveryKey::from_recovery_phrase(&recovery_phrase, PathBuf::new())?;
        self.admit_current_app_key_with_authority_keys(
            recovery_key.keys(),
            NostrIdentityKeyPurpose::RecoveryPhrase,
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
    ) -> Result<&AppKeysProjection, ProfileError> {
        self.admit_current_app_key_with_authority_keys(
            nip46_keys,
            NostrIdentityKeyPurpose::Nip46Signer,
            label,
        )
    }

    fn admit_current_app_key_with_authority_keys(
        &mut self,
        authority_keys: &Keys,
        expected_purpose: NostrIdentityKeyPurpose,
        label: Option<String>,
    ) -> Result<&AppKeysProjection, ProfileError> {
        let authority_pubkey = authority_keys.public_key().to_hex();
        let projection = self.state.profile_projection();
        let Some(authority_facet) = projection.active_facets.get(&authority_pubkey) else {
            return Err(ProfileError::RecoveryAuthorityUnavailable);
        };
        if !authority_facet.has_purpose(expected_purpose)
            || !authority_facet.capabilities.can_recover_app_keys
        {
            return Err(ProfileError::RecoveryAuthorityUnavailable);
        }
        if projection
            .tombstones
            .contains_key(&self.state.app_key_pubkey)
        {
            return Err(ProfileError::CurrentAppKeyTombstoned);
        }
        if projection.can_write_roots(&self.state.app_key_pubkey) {
            return Err(ProfileError::AppKeyAlreadyAuthorized);
        }
        let dck_to_rewrap = if authority_facet.capabilities.can_decrypt_secret_epochs {
            let dck = self.current_dck_from_authority_keys(authority_keys, expected_purpose)?;
            if !authority_facet.capabilities.can_change_secret_epochs() {
                return Err(ProfileError::RecoveryCannotRotateSecretEpochs);
            }
            Some(dck)
        } else {
            None
        };

        let now = next_profile_timestamp(&self.state);
        let parents = nostr_identity_roster_parent_ids(&self.state.profile_roster_ops);
        let label = label.or_else(|| self.state.app_key_label.clone());
        self.state.app_key_label.clone_from(&label);
        let encrypted_device_labels = dck_to_rewrap
            .as_ref()
            .map(|dck| {
                self.encrypted_device_labels_for_dck(
                    dck,
                    next_profile_secret_epoch(&projection),
                    now,
                    label
                        .as_deref()
                        .and_then(normalize_app_key_label)
                        .map(|label| BTreeMap::from([(self.state.app_key_pubkey.clone(), label)]))
                        .unwrap_or_default(),
                    &[],
                )
            })
            .transpose()?
            .flatten();
        let add_op = signed_profile_roster_op_with_parents_and_device_labels(
            authority_keys,
            self.state.profile_id,
            parents,
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::app_key(
                    self.state.app_key_pubkey.clone(),
                    now,
                    None,
                    NostrIdentityCapabilities::app_admin(),
                ),
            },
            now,
            encrypted_device_labels,
        )?;
        self.state.profile_roster_ops.push(add_op);
        self.state.profile_roster_projection = None;

        if let Some(dck) = dck_to_rewrap {
            self.rotate_profile_dck_epoch_with_signer(authority_keys, &dck, now + 1)?;
        }
        self.sync_app_keys_from_profile_with_device_labels();
        self.current_app_keys_projection()
    }

    pub fn appoint_admin(
        &mut self,
        app_key_pubkey_hex: &str,
    ) -> Result<&AppKeysProjection, ProfileError> {
        self.set_device_role(app_key_pubkey_hex, AppActorRole::Admin)
    }

    pub fn demote_admin(
        &mut self,
        app_key_pubkey_hex: &str,
    ) -> Result<&AppKeysProjection, ProfileError> {
        self.set_device_role(app_key_pubkey_hex, AppActorRole::Member)
    }

    pub fn rename_app_key(
        &mut self,
        app_key_pubkey_hex: &str,
        label: &str,
    ) -> Result<&AppKeysProjection, ProfileError> {
        if !self.state.can_admin_profile() {
            return Err(ProfileError::NoAdminAuthority);
        }
        let label = normalize_app_key_label(label).ok_or(ProfileError::InvalidAppKeyLabel)?;
        let (already_has_label, role) = {
            let snap = self
                .state
                .app_keys
                .as_ref()
                .ok_or(ProfileError::NoCurrentAppKeysProjection)?;
            let current = snap
                .app_actor(app_key_pubkey_hex)
                .ok_or(ProfileError::AppKeyNotInRoster)?;
            (
                current.label.as_deref() == Some(label.as_str()),
                current.role,
            )
        };
        let current_app_key_label_changed = app_key_pubkey_hex == self.state.app_key_pubkey
            && self.state.app_key_label.as_deref() != Some(label.as_str());
        if already_has_label && !current_app_key_label_changed {
            return self.current_app_keys_projection();
        }

        let capabilities = match role {
            AppActorRole::Admin => NostrIdentityCapabilities::app_admin(),
            AppActorRole::Member => NostrIdentityCapabilities::app_writer(),
        };
        let now = next_profile_timestamp(&self.state);
        let dck = self.current_dck()?;
        let secret_epoch =
            next_profile_secret_epoch(&self.state.profile_projection()).saturating_sub(1);
        let encrypted_device_labels = self.encrypted_device_labels_for_dck(
            &dck,
            secret_epoch,
            now,
            BTreeMap::from([(app_key_pubkey_hex.to_string(), label.clone())]),
            &[],
        )?;
        self.append_profile_roster_op_with_device_labels(
            NostrIdentityRosterOp::SetCapabilities {
                pubkey: app_key_pubkey_hex.to_string(),
                capabilities,
            },
            now,
            encrypted_device_labels,
        )?;
        if app_key_pubkey_hex == self.state.app_key_pubkey {
            self.state.app_key_label = Some(label);
        }
        self.sync_app_keys_from_profile_with_device_labels();
        self.current_app_keys_projection()
    }

    fn set_device_role(
        &mut self,
        app_key_pubkey_hex: &str,
        role: AppActorRole,
    ) -> Result<&AppKeysProjection, ProfileError> {
        if !self.state.can_admin_profile() {
            return Err(ProfileError::NoAdminAuthority);
        }
        let (already_has_role, would_remove_last_admin) = {
            let snap = self
                .state
                .app_keys
                .as_ref()
                .ok_or(ProfileError::NoCurrentAppKeysProjection)?;
            let current = snap
                .app_actor(app_key_pubkey_hex)
                .ok_or(ProfileError::AppKeyNotInRoster)?;
            (
                current.role == role,
                current.is_admin()
                    && role != AppActorRole::Admin
                    && snap
                        .app_actors
                        .iter()
                        .filter(|device| device.is_admin())
                        .count()
                        <= 1,
            )
        };
        if already_has_role {
            return self.current_app_keys_projection();
        }
        if would_remove_last_admin {
            return Err(ProfileError::CannotRemoveLastAdmin);
        }
        let capabilities = match role {
            AppActorRole::Admin => NostrIdentityCapabilities::app_admin(),
            AppActorRole::Member => NostrIdentityCapabilities::app_writer(),
        };
        let now = next_profile_timestamp(&self.state);
        self.append_profile_roster_op(
            NostrIdentityRosterOp::SetCapabilities {
                pubkey: app_key_pubkey_hex.to_string(),
                capabilities,
            },
            now,
        )?;
        let dck = generate_dck();
        self.rotate_profile_dck_epoch(&dck, now + 1)?;
        self.sync_app_keys_from_profile_with_device_labels();
        self.current_app_keys_projection()
    }

    fn append_profile_roster_op(
        &mut self,
        op: NostrIdentityRosterOp,
        created_at: i64,
    ) -> Result<(), ProfileError> {
        let encrypted_device_labels = self.encrypted_current_device_labels(created_at)?;
        self.append_profile_roster_op_with_device_labels(op, created_at, encrypted_device_labels)
    }

    fn append_profile_roster_op_with_device_labels(
        &mut self,
        op: NostrIdentityRosterOp,
        created_at: i64,
        encrypted_device_labels: Option<String>,
    ) -> Result<(), ProfileError> {
        let parents = nostr_identity_roster_parent_ids(&self.state.profile_roster_ops);
        let signed = signed_profile_roster_op_with_parents_and_device_labels(
            self.app_key.keys(),
            self.state.profile_id,
            parents,
            op,
            created_at,
            encrypted_device_labels,
        )?;
        self.state.profile_roster_ops.push(signed);
        self.state.profile_roster_projection = None;
        Ok(())
    }

    fn rotate_profile_dck_epoch(
        &mut self,
        dck: &[u8; 32],
        created_at: i64,
    ) -> Result<(), ProfileError> {
        let signer = self.app_key.keys().clone();
        self.rotate_profile_dck_epoch_with_signer(&signer, dck, created_at)
    }

    fn rotate_profile_dck_epoch_with_signer(
        &mut self,
        signer_keys: &Keys,
        dck: &[u8; 32],
        created_at: i64,
    ) -> Result<(), ProfileError> {
        let projection = self.state.profile_projection();
        let recipients = projection
            .active_facets
            .values()
            .filter(|facet| facet.capabilities.can_receive_secret_wraps)
            .map(|facet| facet.pubkey.as_str())
            .collect::<Vec<_>>();
        let wrapped_dck = wrap_dck_for_pubkeys(signer_keys.secret_key(), recipients, dck)?;
        let epoch = projection
            .secret_epochs
            .keys()
            .next_back()
            .map_or(1, |epoch| epoch + 1);
        let parents = nostr_identity_roster_parent_ids(&self.state.profile_roster_ops);
        let encrypted_device_labels =
            self.encrypted_device_labels_for_dck(dck, epoch, created_at, BTreeMap::new(), &[])?;
        let signed = signed_profile_roster_op_with_parents_and_device_labels(
            signer_keys,
            self.state.profile_id,
            parents,
            NostrIdentityRosterOp::RotateSecretEpoch {
                epoch,
                wrapped_secrets: wrapped_dck,
            },
            created_at,
            encrypted_device_labels,
        )?;
        self.state.profile_roster_ops.push(signed);
        self.state.profile_roster_projection = None;
        Ok(())
    }

    /// Add missing DCK wraps for the current secret epoch without rotating the
    /// DCK. Only the `AppKey` that signed the epoch may repair it, keeping the
    /// epoch's encryption authority unambiguous after divergent roster merges.
    pub fn repair_current_secret_epoch_wraps(
        &mut self,
    ) -> Result<SecretWrapRepairOutcome, ProfileError> {
        let projection = self.state.profile_projection();
        let Some((epoch, key_epoch)) = projection.secret_epochs.iter().next_back() else {
            return Err(ProfileError::NoCurrentAppKeysProjection);
        };
        if key_epoch.signed_by_pubkey != self.state.app_key_pubkey {
            return Err(ProfileError::CurrentAppKeyCannotRepairSecretEpoch {
                signed_by_pubkey: key_epoch.signed_by_pubkey.clone(),
            });
        }
        let Some(current_facet) = projection.active_facets.get(&self.state.app_key_pubkey) else {
            return Err(ProfileError::NoAdminAuthority);
        };
        if !current_facet.capabilities.can_change_secret_epochs() {
            return Err(ProfileError::NoAdminAuthority);
        }

        let missing_pubkeys = projection.active_key_recipients_missing_wraps(*epoch);
        if missing_pubkeys.is_empty() {
            self.sync_app_keys_from_profile_with_device_labels();
            return Ok(SecretWrapRepairOutcome {
                epoch: *epoch,
                repaired_pubkeys: Vec::new(),
                projection: self
                    .state
                    .app_keys
                    .as_ref()
                    .ok_or(ProfileError::NoCurrentAppKeysProjection)?
                    .clone(),
            });
        }

        let dck = self.current_dck()?;
        let wrapped_dck = wrap_dck_for_pubkeys(
            self.app_key.keys().secret_key(),
            missing_pubkeys.iter().map(String::as_str),
            &dck,
        )?;
        let parents = nostr_identity_roster_parent_ids(&self.state.profile_roster_ops);
        let signed = signed_profile_roster_op_with_parents(
            self.app_key.keys(),
            self.state.profile_id,
            parents,
            NostrIdentityRosterOp::RepairSecretWraps {
                epoch: *epoch,
                wrapped_secrets: wrapped_dck,
            },
            next_profile_timestamp(&self.state),
        )?;
        self.state.profile_roster_ops.push(signed);
        self.state.profile_roster_projection = None;
        self.sync_app_keys_from_profile_with_device_labels();
        Ok(SecretWrapRepairOutcome {
            epoch: *epoch,
            repaired_pubkeys: missing_pubkeys,
            projection: self
                .state
                .app_keys
                .as_ref()
                .ok_or(ProfileError::NoCurrentAppKeysProjection)?
                .clone(),
        })
    }

    /// Decrypt this `AppKey`'s DCK wrap from the current profile key epoch. Errors
    /// with `NoWrapForThisAppKey` if the `AppKey` has been revoked or
    /// never authorized.
    pub fn current_dck(&self) -> Result<[u8; 32], ProfileError> {
        let projection = self.state.profile_projection();
        let key_epoch = projection
            .secret_epochs
            .values()
            .next_back()
            .ok_or(ProfileError::NoCurrentAppKeysProjection)?;
        let wrap = key_epoch
            .wrapped_secrets
            .get(&self.state.app_key_pubkey)
            .ok_or(ProfileError::NoWrapForThisAppKey)?;
        let signer_pk = PublicKey::from_hex(&key_epoch.signed_by_pubkey)
            .map_err(|e| ProfileError::InvalidAppKeyPubkey(e.to_string()))?;
        let bytes = nip44::decrypt_to_bytes(self.app_key.keys().secret_key(), &signer_pk, wrap)
            .map_err(|e| ProfileError::Unwrap(e.to_string()))?;
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| ProfileError::InvalidDckLength(bytes.len()))?;
        Ok(arr)
    }

    pub fn current_dck_from_recovery_phrase(
        &self,
        recovery_phrase: &str,
    ) -> Result<[u8; 32], ProfileError> {
        let recovery_phrase = validate_recovery_phrase(recovery_phrase)?;
        let recovery_key = RecoveryKey::from_recovery_phrase(&recovery_phrase, PathBuf::new())?;
        self.current_dck_from_authority_keys(
            recovery_key.keys(),
            NostrIdentityKeyPurpose::RecoveryPhrase,
        )
    }

    pub fn current_dck_from_nip46_keys(&self, nip46_keys: &Keys) -> Result<[u8; 32], ProfileError> {
        self.current_dck_from_authority_keys(nip46_keys, NostrIdentityKeyPurpose::Nip46Signer)
    }

    fn current_dck_from_authority_keys(
        &self,
        authority_keys: &Keys,
        expected_purpose: NostrIdentityKeyPurpose,
    ) -> Result<[u8; 32], ProfileError> {
        let authority_pubkey = authority_keys.public_key().to_hex();
        let projection = self.state.profile_projection();
        let Some(facet) = projection.active_facets.get(&authority_pubkey) else {
            return Err(ProfileError::RecoveryAuthorityUnavailable);
        };
        if !facet.has_purpose(expected_purpose) || !facet.capabilities.can_decrypt_secret_epochs {
            return Err(ProfileError::RecoveryAuthorityUnavailable);
        }
        let key_epoch = projection
            .secret_epochs
            .values()
            .next_back()
            .ok_or(ProfileError::NoCurrentAppKeysProjection)?;
        let wrap = key_epoch
            .wrapped_secrets
            .get(&authority_pubkey)
            .ok_or(ProfileError::NoWrapForThisAppKey)?;
        let signer_pk = PublicKey::from_hex(&key_epoch.signed_by_pubkey)
            .map_err(|e| ProfileError::InvalidAppKeyPubkey(e.to_string()))?;
        let bytes = nip44::decrypt_to_bytes(authority_keys.secret_key(), &signer_pk, wrap)
            .map_err(|e| ProfileError::Unwrap(e.to_string()))?;
        let arr: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| ProfileError::InvalidDckLength(bytes.len()))?;
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

pub fn app_key_link_invite_keys(invite_secret_key: &str) -> Result<Keys, ProfileError> {
    let trimmed = invite_secret_key.trim();
    if trimmed.is_empty() {
        return Err(ProfileError::InvalidAppKeyLinkInviteSecret(
            "empty invite secret key".to_string(),
        ));
    }
    let secret = if trimmed.starts_with("nsec1") {
        SecretKey::from_bech32(trimmed).map_err(|error| error.to_string())
    } else {
        SecretKey::from_hex(trimmed).map_err(|error| error.to_string())
    }
    .map_err(ProfileError::InvalidAppKeyLinkInviteSecret)?;
    Ok(Keys::new(secret))
}

pub fn app_key_link_invite_pubkey(invite_secret_key: &str) -> Result<String, ProfileError> {
    Ok(app_key_link_invite_keys(invite_secret_key)?
        .public_key()
        .to_hex())
}

fn normalize_required_pubkey(value: &str) -> Result<String, ProfileError> {
    let trimmed = value.trim();
    if is_pubkey_hex(trimmed) {
        return Ok(trimmed.to_ascii_lowercase());
    }
    Err(ProfileError::InvalidAppKeyPubkey(trimmed.to_string()))
}

fn default_app_key_link_secret() -> String {
    Keys::generate().secret_key().to_secret_hex()
}

fn wrap_dck_for_pubkeys<'a, I>(
    owner_secret: &SecretKey,
    pubkeys: I,
    dck: &[u8; 32],
) -> Result<BTreeMap<String, String>, ProfileError>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut wraps = BTreeMap::new();
    for pubkey in pubkeys {
        let pk = PublicKey::from_hex(pubkey)
            .map_err(|e| ProfileError::InvalidAppKeyPubkey(e.to_string()))?;
        let ct = nip44::encrypt(owner_secret, &pk, dck.as_slice(), Nip44Version::V2)
            .map_err(|e| ProfileError::Wrap(e.to_string()))?;
        wraps.insert(pubkey.to_string(), ct);
    }
    Ok(wraps)
}

fn initial_profile_roster_ops(
    signer_keys: &Keys,
    profile_id: NostrIdentityId,
    app_entry: &AppActorEntry,
    recovery_pubkey: Option<&str>,
    dck: &[u8; 32],
    created_at: i64,
) -> Result<Vec<SignedNostrIdentityRosterOp>, ProfileError> {
    let app_pubkey = app_entry.pubkey.clone();
    let encrypted_device_labels = app_entry
        .label
        .as_deref()
        .and_then(normalize_app_key_label)
        .map(|label| BTreeMap::from([(app_pubkey.clone(), label)]))
        .map(|labels| encrypted_device_labels_from_map(profile_id, 1, labels, dck, created_at))
        .transpose()?
        .flatten();
    let app_op = signed_profile_roster_op_with_parents_and_device_labels(
        signer_keys,
        profile_id,
        Vec::new(),
        NostrIdentityRosterOp::AddFacet {
            facet: NostrIdentityFacet::app_key(
                app_pubkey.clone(),
                created_at,
                None,
                NostrIdentityCapabilities::app_admin(),
            ),
        },
        created_at,
        encrypted_device_labels,
    )?;
    let mut ops = vec![app_op];
    let mut recipients = vec![app_pubkey.as_str()];
    let epoch_created_at = if let Some(recovery_pubkey) = recovery_pubkey {
        let recovery_op = signed_profile_roster_op_with_parents(
            signer_keys,
            profile_id,
            nostr_identity_roster_parent_ids(&ops),
            NostrIdentityRosterOp::AddFacet {
                facet: NostrIdentityFacet::recovery_phrase(
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
    let epoch_op = signed_profile_roster_op_with_parents(
        signer_keys,
        profile_id,
        nostr_identity_roster_parent_ids(&ops),
        NostrIdentityRosterOp::RotateSecretEpoch {
            epoch: 1,
            wrapped_secrets: wrapped_dck,
        },
        epoch_created_at,
    )?;
    ops.push(epoch_op);
    Ok(ops)
}

fn recovery_authority_purpose(
    projection: &NostrIdentityRosterProjection,
    authority_pubkey: &str,
    expected_purpose: Option<NostrIdentityKeyPurpose>,
) -> Result<NostrIdentityKeyPurpose, ProfileError> {
    let Some(facet) = projection.active_facets.get(authority_pubkey) else {
        return Err(ProfileError::RecoveryAuthorityUnavailable);
    };
    if !facet.capabilities.can_recover_app_keys {
        return Err(ProfileError::RecoveryAuthorityUnavailable);
    }
    if let Some(expected_purpose) = expected_purpose {
        return if facet.has_purpose(expected_purpose) {
            Ok(expected_purpose)
        } else {
            Err(ProfileError::RecoveryAuthorityUnavailable)
        };
    }
    [
        NostrIdentityKeyPurpose::RecoveryPhrase,
        NostrIdentityKeyPurpose::Nip46Signer,
    ]
    .into_iter()
    .find(|purpose| facet.has_purpose(*purpose))
    .ok_or(ProfileError::RecoveryAuthorityUnavailable)
}

fn signed_profile_roster_op_with_parents(
    signer_keys: &Keys,
    profile_id: NostrIdentityId,
    parents: Vec<String>,
    op: NostrIdentityRosterOp,
    created_at: i64,
) -> Result<SignedNostrIdentityRosterOp, ProfileError> {
    signed_profile_roster_op_with_parents_and_device_labels(
        signer_keys,
        profile_id,
        parents,
        op,
        created_at,
        None,
    )
}

fn signed_profile_roster_op_with_parents_and_device_labels(
    signer_keys: &Keys,
    profile_id: NostrIdentityId,
    parents: Vec<String>,
    op: NostrIdentityRosterOp,
    created_at: i64,
    encrypted_device_labels: Option<String>,
) -> Result<SignedNostrIdentityRosterOp, ProfileError> {
    let event = build_nostr_identity_roster_op_event_with_encrypted_device_labels(
        signer_keys,
        profile_id,
        parents,
        None,
        op,
        created_at,
        encrypted_device_labels,
    )?;
    parse_nostr_identity_roster_op_event(&event).map_err(ProfileError::from)
}

fn next_profile_secret_epoch(projection: &NostrIdentityRosterProjection) -> u64 {
    projection
        .secret_epochs
        .keys()
        .next_back()
        .map_or(1, |epoch| epoch + 1)
}

fn encrypted_device_labels_from_map(
    profile_id: NostrIdentityId,
    secret_epoch: u64,
    labels: BTreeMap<String, String>,
    dck: &[u8; 32],
    updated_at: i64,
) -> Result<Option<String>, ProfileError> {
    let labels = normalize_drive_device_labels(labels);
    if labels.is_empty() {
        return Ok(None);
    }
    let payload = drive_device_label_payload(profile_id, secret_epoch, labels, updated_at);
    encrypt_drive_device_labels_with_dck(&payload, dck)
        .map(Some)
        .map_err(|error| ProfileError::Wrap(error.to_string()))
}

fn current_unix_seconds() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Pick a `created_at` for a local mutation that is strictly greater
/// than the current key epoch's. Roster-op merges can be additive, so locally
/// we need to bypass same-second collisions; otherwise rapid approve+revoke
/// cycles would be no-ops.
fn next_local_timestamp(current: Option<&AppKeysProjection>) -> i64 {
    let now = current_unix_seconds();
    match current {
        Some(snap) if snap.created_at >= now => snap.created_at + 1,
        _ => now,
    }
}

fn next_profile_timestamp(state: &ProfileState) -> i64 {
    let latest_profile_op = state
        .profile_roster_ops
        .iter()
        .map(|op| op.content.created_at)
        .max()
        .unwrap_or(0);
    let latest_projection = state
        .app_keys
        .as_ref()
        .map_or(0, |projection| projection.created_at);
    current_unix_seconds()
        .max(latest_profile_op)
        .max(latest_projection)
        + 1
}

fn resolve_app_key_label(label: Option<String>, pubkey_hex: &str) -> Option<String> {
    let hostname = detected_hostname();
    resolve_app_key_label_with_hostname(label, hostname.as_deref(), pubkey_hex)
}

fn resolve_app_key_label_with_hostname(
    label: Option<String>,
    hostname: Option<&str>,
    pubkey_hex: &str,
) -> Option<String> {
    label
        .and_then(|value| normalize_app_key_label(&value))
        .or_else(|| hostname.and_then(normalize_hostname_label))
        .or_else(|| Some(default_app_key_label_for_pubkey(pubkey_hex)))
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
        .find_map(normalize_app_key_label)?;
    let lower = first_label.to_ascii_lowercase();
    if lower == "localhost" || looks_like_generated_hex_label(&lower) {
        return None;
    }
    Some(first_label)
}

fn normalize_app_key_label(value: &str) -> Option<String> {
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

fn default_app_key_label_for_pubkey(pubkey_hex: &str) -> String {
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

/// Convenience: load just the local profile secret paths under a config dir.
#[must_use]
pub fn profile_paths(config_dir: &Path) -> ProfilePaths {
    ProfilePaths {
        app_key: key_path_in(config_dir),
        recovery_phrase: recovery_phrase_path_in(config_dir),
    }
}

pub struct ProfilePaths {
    pub app_key: PathBuf,
    pub recovery_phrase: PathBuf,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)]
pub struct ProfileLogoutReport {
    pub removed_key: bool,
    pub removed_recovery_phrase: bool,
    pub removed_sync_cache: bool,
    pub cleared_profile: bool,
    pub cleared_user_profile: bool,
    pub cleared_drives: bool,
    pub cleared_backup_targets: bool,
}

impl ProfileLogoutReport {
    #[must_use]
    pub fn changed(&self) -> bool {
        self.removed_key
            || self.removed_recovery_phrase
            || self.removed_sync_cache
            || self.cleared_profile
            || self.cleared_user_profile
            || self.cleared_drives
            || self.cleared_backup_targets
    }
}

pub fn logout_local_profile(config_dir: &Path) -> Result<ProfileLogoutReport, ProfileError> {
    let config_path = config_path_in(config_dir);
    let mut config = AppConfig::load_or_default(&config_path)?;

    let cleared_profile = config.profile.take().is_some();
    let cleared_user_profile = config.user_profile.take().is_some();
    let cleared_drives = !config.drives.is_empty();
    config.drives.clear();
    let cleared_backup_targets = !config.backup_targets.is_empty();
    config.backup_targets.clear();
    let mut report = ProfileLogoutReport {
        cleared_profile,
        cleared_user_profile,
        cleared_drives,
        cleared_backup_targets,
        ..ProfileLogoutReport::default()
    };

    if config_path.exists()
        || report.cleared_profile
        || report.cleared_user_profile
        || report.cleared_drives
        || report.cleared_backup_targets
    {
        config.save(&config_path)?;
    }

    report.removed_key = remove_file_if_present(&key_path_in(config_dir))?;
    report.removed_recovery_phrase = remove_file_if_present(&recovery_phrase_path_in(config_dir))?;
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
