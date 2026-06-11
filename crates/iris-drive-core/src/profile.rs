//! Profile state machine.
//!
//! Wraps the profile roster model — stable `IrisProfile` id, this app install's
//! `AppKey`, whether the `AppKey` is an admin in the current roster, and the
//! current authorization state.
//!
//! Three creation paths mirror the iris-chat-rs onboarding flows:
//!
//! 1. **Create** — fresh `AppKey`. Single-install default; this `AppKey`
//!    is the first admin and signs the first roster.
//! 2. **Restore** — import recovery authority or an existing admin `AppKey`.
//! 3. **Link** — paste/scan an `IrisProfile`/admin `AppKey` invite. Generate a
//!    fresh `AppKey` and wait in `AwaitingApproval` until an admin accepts it.

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

use crate::app_keys::{AppActorEntry, AppActorRole, AppKeysProjection};
use crate::config::{AppConfig, ConfigError};
use crate::identity::{AppKey, IdentityError, RecoveryKey};
use crate::iris_profile::{
    IrisProfileCapabilities, IrisProfileError, IrisProfileFacet, IrisProfileId,
    IrisProfileKeyPurpose, IrisProfileRosterOp, IrisProfileRosterProjection,
    SignedIrisProfileRosterOp, build_iris_profile_roster_op_event, iris_profile_roster_parent_ids,
    parse_iris_profile_roster_op_event, project_iris_profile_roster,
};
use crate::paths::{config_path_in, key_path_in, recovery_phrase_path_in, sync_cache_path_in};
use crate::recovery_phrase::{RecoveryPhraseError, save_recovery_phrase, validate_recovery_phrase};

#[derive(Debug, Error)]
pub enum ProfileError {
    #[error("identity: {0}")]
    Identity(#[from] IdentityError),
    #[error("iris profile: {0}")]
    IrisProfile(#[from] IrisProfileError),
    #[error("recovery phrase: {0}")]
    RecoveryPhrase(#[from] RecoveryPhraseError),
    #[error("recovery authority is not active in this IrisProfile")]
    RecoveryAuthorityUnavailable,
    #[error("recovery authority cannot rotate key epochs")]
    RecoveryCannotRotateKeyEpochs,
    #[error("current app key was tombstoned and cannot be re-added")]
    CurrentAppKeyTombstoned,
    #[error("invalid AppKey pubkey: {0}")]
    InvalidAppKeyPubkey(String),
    #[error("this AppKey is not an admin")]
    NoAdminAuthority,
    #[error("AppKey already authorized")]
    AppKeyAlreadyAuthorized,
    #[error("AppKey not in roster")]
    AppKeyNotInRoster,
    #[error("cannot remove the last admin AppKey")]
    CannotRemoveLastAdmin,
    #[error("no AppKeys projection yet")]
    NoCurrentAppKeysProjection,
    #[error("no DCK wrap for this AppKey (revoked or never authorized)")]
    NoWrapForThisAppKey,
    #[error("current AppKey cannot repair key epoch signed by {signed_by_pubkey}")]
    CurrentAppKeyCannotRepairKeyEpoch { signed_by_pubkey: String },
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

/// Current `AppKey` authorization status relative to the `IrisProfile` roster.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct PendingAppKeyLinkRequest {
    pub admin_app_key_pubkey: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub link_secret: String,
    pub requested_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct InboundAppKeyLinkRequest {
    pub app_key_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub link_secret: String,
    pub requested_at: u64,
}

/// Persisted local profile state. Lives inside `AppConfig`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProfileState {
    pub profile_id: IrisProfileId,
    pub app_key_pubkey: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profile_roster_ops: Vec<SignedIrisProfileRosterOp>,
    #[serde(default = "default_app_key_link_secret")]
    pub app_key_link_secret: String,
    pub authorization_state: AppKeyAuthorizationState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_key_label: Option<String>,
    /// Runtime projection cache derived from `profile_roster_ops`.
    #[serde(skip)]
    pub app_keys: Option<AppKeysProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outbound_app_key_link_request: Option<PendingAppKeyLinkRequest>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inbound_app_key_link_requests: Vec<InboundAppKeyLinkRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyWrapRepairOutcome {
    pub epoch: u64,
    pub repaired_pubkeys: Vec<String>,
    pub projection: AppKeysProjection,
}

impl ProfileState {
    #[must_use]
    pub fn profile_projection(&self) -> IrisProfileRosterProjection {
        project_iris_profile_roster(self.profile_id, self.profile_roster_ops.clone())
    }

    #[must_use]
    pub fn root_scope_id(&self) -> String {
        self.profile_id.to_string()
    }

    #[must_use]
    pub fn app_keys_from_profile(&self) -> Option<AppKeysProjection> {
        app_keys_from_profile_roster(self.profile_id, &self.profile_roster_ops)
    }

    pub fn sync_app_keys_from_profile(&mut self) -> bool {
        let Some(projection) = self.app_keys_from_profile() else {
            return false;
        };
        let changed = self.app_keys.as_ref() != Some(&projection);
        self.app_keys = Some(projection);
        self.recompute_authorization();
        changed
    }

    /// Adopt the profile UUID carried by local roster evidence when that
    /// evidence unambiguously belongs to one `IrisProfile`.
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
        self.sync_app_keys_from_profile();
        self.recompute_authorization();
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

    /// Can this install's `AppKey` administer the `IrisProfile` roster?
    #[must_use]
    pub fn can_admin_profile(&self) -> bool {
        self.profile_projection()
            .can_admin_profile(&self.app_key_pubkey)
    }

    /// Can this `AppKey` publish mutable roots for this profile?
    #[must_use]
    pub fn can_write_roots(&self) -> bool {
        self.profile_projection()
            .can_write_roots(&self.app_key_pubkey)
    }

    /// Recompute `authorization_state` from the current profile roster projection.
    pub fn recompute_authorization(&mut self) {
        let projection = self.profile_projection();
        self.authorization_state = if projection.can_write_roots(&self.app_key_pubkey) {
            AppKeyAuthorizationState::Authorized
        } else if projection.tombstones.contains_key(&self.app_key_pubkey)
            || self.authorization_state == AppKeyAuthorizationState::Authorized
        {
            AppKeyAuthorizationState::Revoked
        } else {
            self.authorization_state
        };
        if self.authorization_state == AppKeyAuthorizationState::Authorized {
            self.outbound_app_key_link_request = None;
        }
    }

    pub fn queue_outbound_app_key_link_request(
        &mut self,
        admin_app_key_pubkey: String,
        link_secret: &str,
        requested_at: u64,
    ) -> Result<bool, ProfileError> {
        if !is_pubkey_hex(&admin_app_key_pubkey) {
            return Err(ProfileError::InvalidAppKeyPubkey(admin_app_key_pubkey));
        }
        if self.app_key_pubkey == admin_app_key_pubkey {
            return Ok(false);
        }
        let next = PendingAppKeyLinkRequest {
            admin_app_key_pubkey,
            link_secret: link_secret.trim().to_string(),
            requested_at,
        };
        let changed = self.outbound_app_key_link_request.as_ref() != Some(&next);
        self.outbound_app_key_link_request = Some(next);
        Ok(changed)
    }

    pub fn record_inbound_app_key_link_request(
        &mut self,
        profile_id: IrisProfileId,
        app_key_pubkey: &str,
        label: Option<String>,
        link_secret: &str,
        requested_at: u64,
    ) -> Result<bool, ProfileError> {
        if profile_id != self.profile_id || !self.can_admin_profile() {
            return Ok(false);
        }
        let link_secret = link_secret.trim();
        let expected_secret = self.app_key_link_secret.trim();
        if !expected_secret.is_empty() && link_secret != expected_secret {
            return Ok(false);
        }
        if !is_pubkey_hex(app_key_pubkey) {
            return Err(ProfileError::InvalidAppKeyPubkey(
                app_key_pubkey.to_string(),
            ));
        }
        if app_key_pubkey == self.app_key_pubkey
            || self
                .app_keys
                .as_ref()
                .is_some_and(|snap| snap.contains(app_key_pubkey))
        {
            self.inbound_app_key_link_requests
                .retain(|request| request.app_key_pubkey != app_key_pubkey);
            return Ok(false);
        }

        let label = label.and_then(|label| {
            let trimmed = label.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });
        let mut changed = false;
        if let Some(existing) = self
            .inbound_app_key_link_requests
            .iter_mut()
            .find(|request| request.app_key_pubkey == app_key_pubkey)
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
            self.inbound_app_key_link_requests
                .push(InboundAppKeyLinkRequest {
                    app_key_pubkey: app_key_pubkey.to_string(),
                    label,
                    link_secret: link_secret.to_string(),
                    requested_at,
                });
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
        let before = self.inbound_app_key_link_requests.len();
        self.inbound_app_key_link_requests
            .retain(|request| request.app_key_pubkey != app_key_pubkey);
        Ok(before != self.inbound_app_key_link_requests.len())
    }

    pub fn reset_app_key_link_secret(&mut self) -> bool {
        let previous = self.app_key_link_secret.clone();
        self.app_key_link_secret = default_app_key_link_secret();
        let had_requests = !self.inbound_app_key_link_requests.is_empty();
        self.inbound_app_key_link_requests.clear();
        had_requests || self.app_key_link_secret != previous
    }
}

#[must_use]
pub fn single_roster_profile_id(
    profile_roster_ops: &[SignedIrisProfileRosterOp],
) -> Option<IrisProfileId> {
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
    profile_id: IrisProfileId,
    profile_roster_ops: &[SignedIrisProfileRosterOp],
) -> Option<AppKeysProjection> {
    let projection = project_iris_profile_roster(profile_id, profile_roster_ops.iter().cloned());
    app_keys_from_profile_projection(&projection)
}

#[must_use]
pub fn app_keys_from_profile_projection(
    projection: &IrisProfileRosterProjection,
) -> Option<AppKeysProjection> {
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
    Some(AppKeysProjection {
        profile_id: projection.profile_id.to_string(),
        signed_by_pubkey: Some(key_epoch.signed_by_pubkey.clone()),
        created_at: key_epoch.created_at,
        app_actors,
        dck_generation: key_epoch.epoch,
        wrapped_dck,
    })
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

    /// **Create** flow — fresh `AppKey` saved to the config dir. The `AppKey` is
    /// auto-authorized as the first admin via a self-signed single-entry
    /// `IrisProfile` roster op log.
    pub fn create(config_dir: &Path, app_key_label: Option<String>) -> Result<Self, ProfileError> {
        let profile_id = IrisProfileId::new_v4();
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
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
        };

        let now = current_unix_seconds();
        let app_actor = AppActorEntry::admin(state.app_key_pubkey.clone(), now, app_key_label);
        let dck = generate_dck();
        state.profile_roster_ops =
            initial_profile_roster_ops(device.keys(), profile_id, &app_actor, None, &dck, now)?;
        state.sync_app_keys_from_profile();

        let profile = Self {
            state,
            app_key: device,
        };
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
        let profile_id = IrisProfileId::new_v4();
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
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
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
        state.sync_app_keys_from_profile();

        let profile = Self {
            state,
            app_key: device,
        };
        Ok(profile)
    }

    /// Restore an existing `IrisProfile` when the UUID and roster log came
    /// from verified evidence such as relay roster ops, an invite, or an
    /// export. The recovery secret proves authority; it does not determine the
    /// UUID.
    pub fn restore_with_profile_roster_ops(
        config_dir: &Path,
        recovery_secret: &str,
        profile_id: IrisProfileId,
        profile_roster_ops: Vec<SignedIrisProfileRosterOp>,
        app_key_label: Option<String>,
    ) -> Result<Self, ProfileError> {
        let recovery_phrase = validate_recovery_phrase(recovery_secret).ok();
        let recovery_key = if let Some(phrase) = recovery_phrase.as_deref() {
            RecoveryKey::from_recovery_phrase(phrase, PathBuf::new())?
        } else {
            RecoveryKey::from_secret(recovery_secret, PathBuf::new())?
        };
        let authority_pubkey = recovery_key.pubkey_hex();
        let projection = project_iris_profile_roster(profile_id, profile_roster_ops.clone());
        let expected_purpose = recovery_authority_purpose(
            &projection,
            &authority_pubkey,
            recovery_phrase
                .as_ref()
                .map(|_| IrisProfileKeyPurpose::RecoveryPhrase),
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
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
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
        profile_id: IrisProfileId,
        profile_roster_ops: Vec<SignedIrisProfileRosterOp>,
        label: Option<String>,
    ) -> Result<AppKeysProjection, ProfileError> {
        let recovery_phrase = validate_recovery_phrase(recovery_secret).ok();
        let recovery_key = if let Some(phrase) = recovery_phrase.as_deref() {
            RecoveryKey::from_recovery_phrase(phrase, PathBuf::new())?
        } else {
            RecoveryKey::from_secret(recovery_secret, PathBuf::new())?
        };
        let projection = project_iris_profile_roster(profile_id, profile_roster_ops.clone());
        let expected_purpose = recovery_authority_purpose(
            &projection,
            &recovery_key.pubkey_hex(),
            recovery_phrase
                .as_ref()
                .map(|_| IrisProfileKeyPurpose::RecoveryPhrase),
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
        profile_id: IrisProfileId,
        profile_roster_ops: Vec<SignedIrisProfileRosterOp>,
        label: Option<String>,
    ) -> Result<AppKeysProjection, ProfileError> {
        let projection = project_iris_profile_roster(profile_id, profile_roster_ops.clone());
        let expected_purpose = recovery_authority_purpose(
            &projection,
            &nip46_keys.public_key().to_hex(),
            Some(IrisProfileKeyPurpose::Nip46Signer),
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
        expected_purpose: IrisProfileKeyPurpose,
        profile_id: IrisProfileId,
        profile_roster_ops: Vec<SignedIrisProfileRosterOp>,
        label: Option<String>,
    ) -> Result<AppKeysProjection, ProfileError> {
        let original_state = self.state.clone();
        self.state.profile_id = profile_id;
        self.state.profile_roster_ops = profile_roster_ops;
        self.state.app_keys = None;
        self.state.authorization_state = AppKeyAuthorizationState::AwaitingApproval;
        self.state.outbound_app_key_link_request = None;
        self.state.inbound_app_key_link_requests.clear();
        self.state.sync_app_keys_from_profile();

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

    /// **Link** flow — generate a fresh `AppKey` for a known `IrisProfile`.
    /// The admin `AppKey` is used only as the request target; the new local
    /// `AppKey` starts in `AwaitingApproval` until a roster admin accepts it.
    pub fn link_to_profile(
        config_dir: &Path,
        profile_id: IrisProfileId,
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
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
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
    pub fn load(mut state: ProfileState, config_dir: &Path) -> Result<Self, ProfileError> {
        let device = AppKey::load(key_path_in(config_dir))?;
        state.sync_app_keys_from_profile();
        Ok(Self {
            state,
            app_key: device,
        })
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
        if !self.state.can_admin_profile() {
            return Err(ProfileError::NoAdminAuthority);
        }
        if let Some(snap) = &self.state.app_keys
            && snap.contains(app_key_pubkey_hex)
        {
            return Err(ProfileError::AppKeyAlreadyAuthorized);
        }
        let now = next_profile_timestamp(&self.state);
        self.append_profile_roster_op(
            IrisProfileRosterOp::AddFacet {
                facet: IrisProfileFacet::app_key(
                    app_key_pubkey_hex.to_string(),
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
            .inbound_app_key_link_requests
            .retain(|request| request.app_key_pubkey != app_key_pubkey_hex);
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
        self.append_profile_roster_op(
            IrisProfileRosterOp::TombstoneFacet {
                pubkey: app_key_pubkey_hex.to_string(),
                reason: None,
            },
            now,
        )?;
        let dck = generate_dck();
        self.rotate_profile_dck_epoch(&dck, now + 1)?;
        self.state.sync_app_keys_from_profile();
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
        self.state.sync_app_keys_from_profile();
        self.current_app_keys_projection()
    }

    /// Add a NIP-46 signer as an `IrisProfile` recovery authority. When
    /// `can_decrypt_key_epochs` is true, the current admin rotates the key
    /// epoch so the signer receives a DCK wrap immediately.
    pub fn add_nip46_recovery(
        &mut self,
        nip46_pubkey_hex: &str,
        label: Option<String>,
        can_decrypt_key_epochs: bool,
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

    /// Add a recovery phrase as an `IrisProfile` recovery authority by
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
            if facet.has_purpose(IrisProfileKeyPurpose::RecoveryPhrase) {
                return Ok(recovery_pubkey);
            }
            return Err(ProfileError::AppKeyAlreadyAuthorized);
        }
        if projection.tombstones.contains_key(&recovery_pubkey) {
            return Err(ProfileError::CurrentAppKeyTombstoned);
        }

        let now = next_profile_timestamp(&self.state);
        self.append_profile_roster_op(
            IrisProfileRosterOp::AddFacet {
                facet: IrisProfileFacet::recovery_phrase(recovery_pubkey.clone(), now),
            },
            now,
        )?;
        let dck = generate_dck();
        self.rotate_profile_dck_epoch(&dck, now + 1)?;
        self.state.sync_app_keys_from_profile();
        Ok(recovery_pubkey)
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
    ) -> Result<&AppKeysProjection, ProfileError> {
        let recovery_phrase = validate_recovery_phrase(recovery_phrase)?;
        let recovery_key = RecoveryKey::from_recovery_phrase(&recovery_phrase, PathBuf::new())?;
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
    ) -> Result<&AppKeysProjection, ProfileError> {
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
        let should_rotate_epoch = authority_facet.capabilities.can_decrypt_key_epochs;
        if should_rotate_epoch {
            self.current_dck_from_authority_keys(authority_keys, expected_purpose)?;
            if !authority_facet.capabilities.can_change_key_epochs() {
                return Err(ProfileError::RecoveryCannotRotateKeyEpochs);
            }
        }

        let now = next_profile_timestamp(&self.state);
        let parents = iris_profile_roster_parent_ids(&self.state.profile_roster_ops);
        let label = label.or_else(|| self.state.app_key_label.clone());
        let add_op = signed_profile_roster_op_with_parents(
            authority_keys,
            self.state.profile_id,
            parents,
            IrisProfileRosterOp::AddFacet {
                facet: IrisProfileFacet::app_key(
                    self.state.app_key_pubkey.clone(),
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
            AppActorRole::Admin => IrisProfileCapabilities::app_admin(),
            AppActorRole::Member => IrisProfileCapabilities::app_writer(),
        };
        let now = next_profile_timestamp(&self.state);
        self.append_profile_roster_op(
            IrisProfileRosterOp::SetCapabilities {
                pubkey: app_key_pubkey_hex.to_string(),
                capabilities,
            },
            now,
        )?;
        let dck = generate_dck();
        self.rotate_profile_dck_epoch(&dck, now + 1)?;
        self.state.sync_app_keys_from_profile();
        self.current_app_keys_projection()
    }

    fn append_profile_roster_op(
        &mut self,
        op: IrisProfileRosterOp,
        created_at: i64,
    ) -> Result<(), ProfileError> {
        let parents = iris_profile_roster_parent_ids(&self.state.profile_roster_ops);
        let signed = signed_profile_roster_op_with_parents(
            self.app_key.keys(),
            self.state.profile_id,
            parents,
            op,
            created_at,
        )?;
        self.state.profile_roster_ops.push(signed);
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
            .filter(|facet| facet.capabilities.can_receive_key_wraps)
            .map(|facet| facet.pubkey.as_str())
            .collect::<Vec<_>>();
        let wrapped_dck = wrap_dck_for_pubkeys(signer_keys.secret_key(), recipients, dck)?;
        let epoch = projection
            .key_epochs
            .keys()
            .next_back()
            .map_or(1, |epoch| epoch + 1);
        let parents = iris_profile_roster_parent_ids(&self.state.profile_roster_ops);
        let signed = signed_profile_roster_op_with_parents(
            signer_keys,
            self.state.profile_id,
            parents,
            IrisProfileRosterOp::RotateKeyEpoch { epoch, wrapped_dck },
            created_at,
        )?;
        self.state.profile_roster_ops.push(signed);
        Ok(())
    }

    /// Add missing DCK wraps for the current key epoch without rotating the
    /// DCK. Only the `AppKey` that signed the epoch may repair it, keeping the
    /// epoch's encryption authority unambiguous after divergent roster merges.
    pub fn repair_current_key_epoch_wraps(&mut self) -> Result<KeyWrapRepairOutcome, ProfileError> {
        let projection = self.state.profile_projection();
        let Some((epoch, key_epoch)) = projection.key_epochs.iter().next_back() else {
            return Err(ProfileError::NoCurrentAppKeysProjection);
        };
        if key_epoch.signed_by_pubkey != self.state.app_key_pubkey {
            return Err(ProfileError::CurrentAppKeyCannotRepairKeyEpoch {
                signed_by_pubkey: key_epoch.signed_by_pubkey.clone(),
            });
        }
        let Some(current_facet) = projection.active_facets.get(&self.state.app_key_pubkey) else {
            return Err(ProfileError::NoAdminAuthority);
        };
        if !current_facet.capabilities.can_change_key_epochs() {
            return Err(ProfileError::NoAdminAuthority);
        }

        let missing_pubkeys = projection.active_key_recipients_missing_wraps(*epoch);
        if missing_pubkeys.is_empty() {
            self.state.sync_app_keys_from_profile();
            return Ok(KeyWrapRepairOutcome {
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
        let parents = iris_profile_roster_parent_ids(&self.state.profile_roster_ops);
        let signed = signed_profile_roster_op_with_parents(
            self.app_key.keys(),
            self.state.profile_id,
            parents,
            IrisProfileRosterOp::RepairKeyWraps {
                epoch: *epoch,
                wrapped_dck,
            },
            next_profile_timestamp(&self.state),
        )?;
        self.state.profile_roster_ops.push(signed);
        self.state.sync_app_keys_from_profile();
        Ok(KeyWrapRepairOutcome {
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
            .key_epochs
            .values()
            .next_back()
            .ok_or(ProfileError::NoCurrentAppKeysProjection)?;
        let wrap = key_epoch
            .wrapped_dck
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
            IrisProfileKeyPurpose::RecoveryPhrase,
        )
    }

    pub fn current_dck_from_nip46_keys(&self, nip46_keys: &Keys) -> Result<[u8; 32], ProfileError> {
        self.current_dck_from_authority_keys(nip46_keys, IrisProfileKeyPurpose::Nip46Signer)
    }

    fn current_dck_from_authority_keys(
        &self,
        authority_keys: &Keys,
        expected_purpose: IrisProfileKeyPurpose,
    ) -> Result<[u8; 32], ProfileError> {
        let authority_pubkey = authority_keys.public_key().to_hex();
        let projection = self.state.profile_projection();
        let Some(facet) = projection.active_facets.get(&authority_pubkey) else {
            return Err(ProfileError::RecoveryAuthorityUnavailable);
        };
        if !facet.has_purpose(expected_purpose) || !facet.capabilities.can_decrypt_key_epochs {
            return Err(ProfileError::RecoveryAuthorityUnavailable);
        }
        let key_epoch = projection
            .key_epochs
            .values()
            .next_back()
            .ok_or(ProfileError::NoCurrentAppKeysProjection)?;
        let wrap = key_epoch
            .wrapped_dck
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

fn default_app_key_link_secret() -> String {
    URL_SAFE_NO_PAD.encode(Uuid::new_v4().as_bytes())
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
    profile_id: IrisProfileId,
    app_entry: &AppActorEntry,
    recovery_pubkey: Option<&str>,
    dck: &[u8; 32],
    created_at: i64,
) -> Result<Vec<SignedIrisProfileRosterOp>, ProfileError> {
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
        let recovery_op = signed_profile_roster_op_with_parents(
            signer_keys,
            profile_id,
            iris_profile_roster_parent_ids(&ops),
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
    let epoch_op = signed_profile_roster_op_with_parents(
        signer_keys,
        profile_id,
        iris_profile_roster_parent_ids(&ops),
        IrisProfileRosterOp::RotateKeyEpoch {
            epoch: 1,
            wrapped_dck,
        },
        epoch_created_at,
    )?;
    ops.push(epoch_op);
    Ok(ops)
}

fn recovery_authority_purpose(
    projection: &IrisProfileRosterProjection,
    authority_pubkey: &str,
    expected_purpose: Option<IrisProfileKeyPurpose>,
) -> Result<IrisProfileKeyPurpose, ProfileError> {
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
        IrisProfileKeyPurpose::RecoveryPhrase,
        IrisProfileKeyPurpose::Nip46Signer,
    ]
    .into_iter()
    .find(|purpose| facet.has_purpose(*purpose))
    .ok_or(ProfileError::RecoveryAuthorityUnavailable)
}

fn signed_profile_roster_op(
    signer_keys: &Keys,
    profile_id: IrisProfileId,
    op: IrisProfileRosterOp,
    created_at: i64,
) -> Result<SignedIrisProfileRosterOp, ProfileError> {
    signed_profile_roster_op_with_parents(signer_keys, profile_id, Vec::new(), op, created_at)
}

fn signed_profile_roster_op_with_parents(
    signer_keys: &Keys,
    profile_id: IrisProfileId,
    parents: Vec<String>,
    op: IrisProfileRosterOp,
    created_at: i64,
) -> Result<SignedIrisProfileRosterOp, ProfileError> {
    let event =
        build_iris_profile_roster_op_event(signer_keys, profile_id, parents, None, op, created_at)?;
    parse_iris_profile_roster_op_event(&event).map_err(ProfileError::from)
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
