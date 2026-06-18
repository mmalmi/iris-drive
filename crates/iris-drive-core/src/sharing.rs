use std::collections::{BTreeMap, BTreeSet};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use nostr_sdk::nips::nip19::FromBech32;
use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{
    Alphabet, Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, SingleLetterTag, Tag, TagKind,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::app_key_summary::pubkey_npub;
use crate::config::AppKeyRootRef;
use crate::iris_profile::{
    IrisProfileCapabilities, IrisProfileError, IrisProfileFacet, IrisProfileId,
    IrisProfileKeyEpoch, IrisProfileKeyPurpose, IrisProfileRosterOp, IrisProfileRosterProjection,
    IrisProfileTombstone, SignedIrisProfileFacetAcceptance, SignedIrisProfileRosterOp,
    build_iris_profile_facet_acceptance_event, iris_profile_tag_kind,
    parse_iris_profile_facet_acceptance_event, project_iris_profile_roster,
};
use crate::provider::{normalize_provider_path, sanitized_provider_file_name};

pub const SHARED_WITH_ME_DIR: &str = "Shared with me";
pub const SHARE_INVITE_SCHEMA: u32 = 1;
pub const SHARE_ACCESS_SNAPSHOT_SCHEMA: u32 = 1;
pub const KIND_SHARE_ACCESS_SNAPSHOT: u16 = 30_078;
pub const SHARE_ACCESS_LABEL: &str = "iris-drive/share-access";
pub const SHARE_INVITE_PREFIX: &str = "iris-drive://share-invite/";

#[derive(Debug, Error)]
pub enum SharingError {
    #[error("share path: {0}")]
    Path(String),
    #[error("invalid recipient pubkey: {0}")]
    InvalidPubkey(String),
    #[error("iris profile: {0}")]
    IrisProfile(#[from] IrisProfileError),
    #[error("no share key epoch")]
    NoKeyEpoch,
    #[error("no share key wrap for the current AppKey")]
    NoWrapForCurrentAppKey,
    #[error("current AppKey cannot repair share key epoch signed by {signed_by_pubkey}")]
    CurrentAppKeyCannotRepairKeyEpoch { signed_by_pubkey: String },
    #[error("current AppKey cannot repair share key epochs")]
    CurrentAppKeyCannotRepairKeyEpochs,
    #[error("current AppKey cannot administer this share")]
    CurrentAppKeyCannotAdminShare,
    #[error("share member not found: {0}")]
    ShareMemberNotFound(IrisProfileId),
    #[error("share member is revoked: {0}")]
    ShareMemberRevoked(IrisProfileId),
    #[error("share invite is not for IrisProfile {local_profile_id}")]
    ShareInviteNotForLocalProfile { local_profile_id: IrisProfileId },
    #[error("current AppKey cannot revoke its own share member")]
    CannotRevokeCurrentShareMember,
    #[error("current AppKey cannot change its own share member role")]
    CannotChangeCurrentShareMemberRole,
    #[error("share invite: {0}")]
    Invite(String),
    #[error("share access snapshot: {0}")]
    AccessSnapshot(String),
    #[error("recipient resolution: {0}")]
    RecipientResolution(String),
    #[error("failed to wrap share key: {0}")]
    Wrap(String),
    #[error("failed to unwrap share key: {0}")]
    Unwrap(String),
    #[error("decrypted share key has wrong length: expected 32 bytes, got {0}")]
    InvalidShareKeyLength(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareRole {
    Admin,
    Editor,
    Reader,
}

impl ShareRole {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Editor => "editor",
            Self::Reader => "reader",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Admin => "Admin",
            Self::Editor => "Editor",
            Self::Reader => "Reader",
        }
    }

    #[must_use]
    pub fn parse_user_input(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "admin" => Some(Self::Admin),
            "editor" | "writer" => Some(Self::Editor),
            "reader" | "read" => Some(Self::Reader),
            _ => None,
        }
    }

    #[must_use]
    pub fn capabilities(self) -> IrisProfileCapabilities {
        match self {
            Self::Admin => IrisProfileCapabilities::app_admin(),
            Self::Editor => IrisProfileCapabilities::app_writer(),
            Self::Reader => IrisProfileCapabilities::app_reader(),
        }
    }

    #[must_use]
    fn rank(self) -> u8 {
        match self {
            Self::Reader => 0,
            Self::Editor => 1,
            Self::Admin => 2,
        }
    }

    #[must_use]
    fn strongest(left: Self, right: Self) -> Self {
        if left.rank() >= right.rank() {
            left
        } else {
            right
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareRecipient {
    pub profile_id: IrisProfileId,
    pub app_pubkey: String,
    pub role: ShareRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representative_npub_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedShareRecipient {
    pub profile_id: IrisProfileId,
    pub representative_pubkey: String,
    pub representative_npub: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub app_pubkeys: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub linked_social_pubkeys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareRecipientProfileEvidence {
    pub profile_id: IrisProfileId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representative_pubkey: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representative_npub: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roster_ops: Vec<SignedIrisProfileRosterOp>,
    #[serde(
        default,
        alias = "facet_acceptances",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub acceptances: Vec<SignedIrisProfileFacetAcceptance>,
}

impl ResolvedShareRecipient {
    #[must_use]
    pub fn share_recipients(&self, role: ShareRole) -> Vec<ShareRecipient> {
        self.app_pubkeys
            .iter()
            .map(|app_pubkey| ShareRecipient {
                profile_id: self.profile_id,
                app_pubkey: app_pubkey.clone(),
                role,
                label: None,
                representative_npub_hint: Some(self.representative_npub.clone()),
                display_name: self.display_name.clone(),
            })
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareMemberStatus {
    Pending,
    Active,
    Revoked,
}

impl ShareMemberStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Revoked => "revoked",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pending => "Pending",
            Self::Active => "Active",
            Self::Revoked => "Revoked",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareMember {
    pub profile_id: IrisProfileId,
    pub role: ShareRole,
    pub status: ShareMemberStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representative_npub_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingShareInvite {
    pub representative_npub_hint: String,
    pub role: ShareRole,
    pub status: ShareMemberStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub created_at: i64,
}

impl PendingShareInvite {
    #[must_use]
    pub fn new(
        representative_npub_hint: String,
        role: ShareRole,
        display_name: Option<String>,
        created_at: i64,
    ) -> Self {
        Self {
            representative_npub_hint,
            role,
            status: ShareMemberStatus::Pending,
            display_name,
            created_at,
        }
    }
}

impl ShareMember {
    #[must_use]
    pub fn active(
        profile_id: IrisProfileId,
        role: ShareRole,
        representative_npub_hint: Option<String>,
        display_name: Option<String>,
    ) -> Self {
        Self {
            profile_id,
            role,
            status: ShareMemberStatus::Active,
            representative_npub_hint,
            display_name,
        }
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self.status, ShareMemberStatus::Active)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ShareAccessTarget {
    Id { id: IrisProfileId },
    Pubkey { pubkey: String },
}

impl ShareAccessTarget {
    #[must_use]
    pub const fn id(id: IrisProfileId) -> Self {
        Self::Id { id }
    }

    #[must_use]
    pub fn pubkey(pubkey: impl Into<String>) -> Self {
        Self::Pubkey {
            pubkey: pubkey.into(),
        }
    }

    #[must_use]
    pub fn key(&self) -> String {
        match self {
            Self::Id { id } => id.to_string(),
            Self::Pubkey { pubkey } => pubkey.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareAccessGrant {
    pub target: ShareAccessTarget,
    pub role: ShareRole,
    pub status: ShareMemberStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representative_npub_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

impl ShareAccessGrant {
    #[must_use]
    pub fn active_id(
        profile_id: IrisProfileId,
        role: ShareRole,
        representative_npub_hint: Option<String>,
        display_name: Option<String>,
    ) -> Self {
        Self {
            target: ShareAccessTarget::id(profile_id),
            role,
            status: ShareMemberStatus::Active,
            representative_npub_hint,
            display_name,
        }
    }

    #[must_use]
    pub fn to_member(&self) -> Option<ShareMember> {
        let ShareAccessTarget::Id { id } = self.target else {
            return None;
        };
        Some(ShareMember {
            profile_id: id,
            role: self.role,
            status: self.status,
            representative_npub_hint: self.representative_npub_hint.clone(),
            display_name: self.display_name.clone(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareAccessDevice {
    pub pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<IrisProfileId>,
    pub added_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareAccessSnapshot {
    pub schema: u32,
    pub resource_id: IrisProfileId,
    pub updated_at: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<ShareAccessGrant>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub devices: BTreeMap<String, ShareAccessDevice>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub tombstones: BTreeMap<String, IrisProfileTombstone>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub key_epochs: BTreeMap<u64, IrisProfileKeyEpoch>,
}

impl ShareAccessSnapshot {
    #[must_use]
    pub fn new(resource_id: IrisProfileId, updated_at: i64) -> Self {
        Self {
            schema: SHARE_ACCESS_SNAPSHOT_SCHEMA,
            resource_id,
            updated_at,
            grants: Vec::new(),
            devices: BTreeMap::new(),
            tombstones: BTreeMap::new(),
            key_epochs: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedShareAccessSnapshot {
    pub snapshot_id: String,
    pub signer_pubkey: String,
    pub content: ShareAccessSnapshot,
    pub event_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareAccessProjection {
    pub share_id: IrisProfileId,
    pub members: BTreeMap<String, ShareMember>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedFolder {
    pub share_id: IrisProfileId,
    pub owner_profile_id: IrisProfileId,
    pub source_path: String,
    pub display_name: String,
    pub local_role: ShareRole,
    pub access: ShareAccessSnapshot,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub pending_invites: BTreeMap<String, PendingShareInvite>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub app_key_roots: BTreeMap<String, AppKeyRootRef>,
}

impl SharedFolder {
    #[must_use]
    pub fn projection(&self) -> IrisProfileRosterProjection {
        project_share_access_snapshot(&self.access)
    }

    #[must_use]
    pub fn access_projection(&self) -> ShareAccessProjection {
        project_share_access(&self.access)
    }

    #[must_use]
    pub fn shared_with_me_path(&self) -> String {
        shared_with_me_path(&self.display_name)
    }

    #[must_use]
    pub fn member_for_app_key(&self, app_pubkey: &str) -> Option<ShareMember> {
        let projection = self.projection();
        member_for_app_key_with_projection(self, &projection, app_pubkey)
    }

    #[must_use]
    pub fn active_member_for_app_key(&self, app_pubkey: &str) -> Option<ShareMember> {
        let projection = self.projection();
        active_member_for_app_key_with_projection(self, &projection, app_pubkey)
    }
}

fn shared_folder_profile_id_for_app_key_with_projection(
    projection: &IrisProfileRosterProjection,
    app_pubkey: &str,
) -> Option<IrisProfileId> {
    projection
        .active_facets
        .get(app_pubkey)
        .and_then(|facet| facet.profile_id)
        .or_else(|| {
            projection
                .tombstones
                .get(app_pubkey)
                .and_then(|tombstone| tombstone.profile_id)
        })
}

fn shared_folder_participant_profiles_with_projection(
    projection: &IrisProfileRosterProjection,
) -> BTreeMap<String, IrisProfileId> {
    let mut participant_profiles = BTreeMap::new();
    for (app_pubkey, facet) in &projection.active_facets {
        if let Some(profile_id) = facet.profile_id {
            participant_profiles.insert(app_pubkey.clone(), profile_id);
        }
    }
    for (app_pubkey, tombstone) in &projection.tombstones {
        if let Some(profile_id) = tombstone.profile_id {
            participant_profiles.insert(app_pubkey.clone(), profile_id);
        }
    }
    participant_profiles
}

fn merge_share_member_grant(members: &mut BTreeMap<String, ShareMember>, granted: ShareMember) {
    members
        .entry(granted.profile_id.to_string())
        .and_modify(|existing| {
            existing.role = ShareRole::strongest(existing.role, granted.role);
            if existing.status == ShareMemberStatus::Pending
                && granted.status == ShareMemberStatus::Active
            {
                existing.status = ShareMemberStatus::Active;
            }
            if existing.representative_npub_hint.is_none() {
                existing
                    .representative_npub_hint
                    .clone_from(&granted.representative_npub_hint);
            }
            if existing.display_name.is_none() {
                existing.display_name.clone_from(&granted.display_name);
            }
        })
        .or_insert(granted);
}

#[must_use]
pub fn project_share_access(snapshot: &ShareAccessSnapshot) -> ShareAccessProjection {
    let mut members = BTreeMap::new();
    for grant in &snapshot.grants {
        if let Some(member) = grant.to_member() {
            merge_share_member_grant(&mut members, member);
        }
    }
    ShareAccessProjection {
        share_id: snapshot.resource_id,
        members,
    }
}

#[must_use]
pub fn project_share_access_snapshot(
    snapshot: &ShareAccessSnapshot,
) -> IrisProfileRosterProjection {
    let members = project_share_access(snapshot).members;
    let mut active_facets = BTreeMap::new();
    for (pubkey, device) in &snapshot.devices {
        if snapshot.tombstones.contains_key(pubkey) {
            continue;
        }
        let role = effective_role_for_device(snapshot, &members, pubkey, device);
        let Some(role) = role else {
            continue;
        };
        let mut facet = IrisProfileFacet::app_key(
            pubkey.clone(),
            device.added_at,
            device.label.clone(),
            role.capabilities(),
        );
        facet.profile_id = device.profile_id;
        active_facets.insert(pubkey.clone(), facet);
    }
    IrisProfileRosterProjection {
        profile_id: snapshot.resource_id,
        active_facets,
        tombstones: snapshot.tombstones.clone(),
        key_epochs: snapshot.key_epochs.clone(),
        accepted_op_ids: Vec::new(),
        rejected_op_ids: Vec::new(),
    }
}

fn effective_role_for_device(
    snapshot: &ShareAccessSnapshot,
    members: &BTreeMap<String, ShareMember>,
    pubkey: &str,
    device: &ShareAccessDevice,
) -> Option<ShareRole> {
    let id_role = device
        .profile_id
        .and_then(|profile_id| members.get(&profile_id.to_string()))
        .filter(|member| member.is_active())
        .map(|member| member.role);
    let pubkey_role = snapshot
        .grants
        .iter()
        .filter(|grant| {
            matches!(&grant.target, ShareAccessTarget::Pubkey { pubkey: target } if target == pubkey)
                && grant.status == ShareMemberStatus::Active
        })
        .map(|grant| grant.role)
        .max_by_key(|role| role.rank());
    match (id_role, pubkey_role) {
        (Some(left), Some(right)) => Some(ShareRole::strongest(left, right)),
        (Some(role), None) | (None, Some(role)) => Some(role),
        (None, None) => None,
    }
}

fn shared_folder_members_with_projection(
    folder: &SharedFolder,
    _projection: &IrisProfileRosterProjection,
) -> BTreeMap<String, ShareMember> {
    project_share_access(&folder.access).members
}

fn next_share_access_update_time(folder: &SharedFolder, requested_at: i64) -> i64 {
    requested_at.max(folder.access.updated_at.saturating_add(1))
}

fn member_for_app_key_with_projection(
    folder: &SharedFolder,
    projection: &IrisProfileRosterProjection,
    app_pubkey: &str,
) -> Option<ShareMember> {
    let profile_id = shared_folder_profile_id_for_app_key_with_projection(projection, app_pubkey)?;
    shared_folder_members_with_projection(folder, projection)
        .get(&profile_id.to_string())
        .cloned()
}

fn active_member_for_app_key_with_projection(
    folder: &SharedFolder,
    projection: &IrisProfileRosterProjection,
    app_pubkey: &str,
) -> Option<ShareMember> {
    member_for_app_key_with_projection(folder, projection, app_pubkey)
        .filter(ShareMember::is_active)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareShortcut {
    pub share_id: IrisProfileId,
    pub path: String,
    pub target_path: String,
}

impl ShareShortcut {
    pub fn new(
        share_id: IrisProfileId,
        path: &str,
        target_path: &str,
    ) -> Result<Self, SharingError> {
        let path =
            normalize_provider_path(path).map_err(|error| SharingError::Path(error.to_string()))?;
        let target_path = if target_path.trim_matches('/').is_empty() {
            String::new()
        } else {
            normalize_provider_path(target_path)
                .map_err(|error| SharingError::Path(error.to_string()))?
        };
        Ok(Self {
            share_id,
            path,
            target_path,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
#[serde(deny_unknown_fields)]
pub struct SharedFolderView {
    pub share_id: IrisProfileId,
    pub display_name: String,
    pub source_path: String,
    pub shared_with_me_path: String,
    pub local_role: ShareRole,
    pub current_app_pubkey: String,
    pub key_status: SharedFolderKeyStatus,
    pub write_authorization: ShareRootWriteAuthorization,
    pub can_write: bool,
    pub can_admin: bool,
    pub current_key_epoch: Option<u64>,
    pub has_current_key_wrap: bool,
    pub key_unavailable: bool,
    pub repair_needed: bool,
    pub missing_key_wrap_count: usize,
    pub missing_key_wrap_pubkeys: Vec<String>,
    pub participant_count: usize,
    pub app_key_count: usize,
    pub members: Vec<SharedFolderMemberView>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_invites: Vec<PendingShareInviteView>,
    pub shortcut_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingShareInviteView {
    pub representative_npub_hint: String,
    pub role: ShareRole,
    pub status: ShareMemberStatus,
    pub display_name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedFolderMemberView {
    pub profile_id: IrisProfileId,
    pub role: ShareRole,
    pub status: ShareMemberStatus,
    pub display_name: String,
    pub representative_npub_hint: Option<String>,
    pub app_key_count: usize,
    #[serde(default)]
    pub can_revoke: bool,
    #[serde(default)]
    pub can_change_role: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareKeyRepairOutcome {
    pub share_id: IrisProfileId,
    pub epoch: u64,
    pub repaired_pubkeys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareMemberRevokeOutcome {
    pub share_id: IrisProfileId,
    pub profile_id: IrisProfileId,
    pub epoch: u64,
    pub revoked_app_pubkeys: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareMemberRoleOutcome {
    pub share_id: IrisProfileId,
    pub profile_id: IrisProfileId,
    pub role: ShareRole,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareInviteOutcome {
    pub share_id: IrisProfileId,
    pub profile_id: IrisProfileId,
    pub epoch: u64,
    pub invite_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareInviteBundle {
    pub schema: u32,
    pub shared_folder: SharedFolder,
    pub recipient_profile_id: IrisProfileId,
    pub role: ShareRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub representative_npub_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub access_snapshot: Option<SignedShareAccessSnapshot>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SharedFolderKeyStatus {
    Available,
    RepairNeeded,
    KeyUnavailable,
    NoKeyEpoch,
    NotARecipient,
    Revoked,
}

impl SharedFolderKeyStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::RepairNeeded => "repair_needed",
            Self::KeyUnavailable => "key_unavailable",
            Self::NoKeyEpoch => "no_key_epoch",
            Self::NotARecipient => "not_a_recipient",
            Self::Revoked => "revoked",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Available => "Available",
            Self::RepairNeeded => "Repair needed",
            Self::KeyUnavailable => "Key unavailable",
            Self::NoKeyEpoch => "No share key",
            Self::NotARecipient => "No access",
            Self::Revoked => "Revoked",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareRootWriteAuthorization {
    Authorized,
    UnknownAppKey,
    UnknownMember,
    PendingMember,
    RevokedMember,
    InsufficientShareRole,
    AppKeyNotActive,
    NotAnAppKey,
    AppKeyCannotWriteRoots,
}

impl ShareRootWriteAuthorization {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Authorized => "authorized",
            Self::UnknownAppKey => "unknown_app_key",
            Self::UnknownMember => "unknown_member",
            Self::PendingMember => "pending_member",
            Self::RevokedMember => "revoked_member",
            Self::InsufficientShareRole => "insufficient_share_role",
            Self::AppKeyNotActive => "app_key_not_active",
            Self::NotAnAppKey => "not_an_app_key",
            Self::AppKeyCannotWriteRoots => "app_key_cannot_write_roots",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Authorized => "Authorized",
            Self::UnknownAppKey => "Unknown AppKey",
            Self::UnknownMember => "Unknown member",
            Self::PendingMember => "Pending member",
            Self::RevokedMember => "Revoked member",
            Self::InsufficientShareRole => "Insufficient share role",
            Self::AppKeyNotActive => "AppKey not active",
            Self::NotAnAppKey => "Not an AppKey",
            Self::AppKeyCannotWriteRoots => "AppKey cannot write roots",
        }
    }

    #[must_use]
    pub const fn is_authorized(self) -> bool {
        matches!(self, Self::Authorized)
    }
}

#[must_use]
pub fn shared_folder_views(
    shared_folders: &[SharedFolder],
    share_shortcuts: &[ShareShortcut],
    current_app_pubkey: &str,
) -> Vec<SharedFolderView> {
    let mut views = shared_folders
        .iter()
        .map(|folder| shared_folder_view(folder, share_shortcuts, current_app_pubkey))
        .collect::<Vec<_>>();
    views.sort_by(|left, right| {
        left.shared_with_me_path
            .cmp(&right.shared_with_me_path)
            .then_with(|| left.share_id.cmp(&right.share_id))
    });
    views
}

#[must_use]
pub fn shared_folder_view(
    folder: &SharedFolder,
    share_shortcuts: &[ShareShortcut],
    current_app_pubkey: &str,
) -> SharedFolderView {
    let projection = folder.projection();
    let current_key_epoch = projection.key_epochs.keys().next_back().copied();
    let missing_key_wrap_pubkeys = current_key_epoch.map_or_else(Vec::new, |epoch| {
        active_share_key_recipients_missing_wraps(folder, &projection, epoch)
    });
    let key_status = share_key_status(
        folder,
        &projection,
        current_app_pubkey,
        current_key_epoch,
        &missing_key_wrap_pubkeys,
    );
    let has_current_key_wrap = key_status_has_current_wrap(key_status);
    let key_unavailable = key_status == SharedFolderKeyStatus::KeyUnavailable;
    let repair_needed = !missing_key_wrap_pubkeys.is_empty();
    let write_authorization = shared_folder_app_key_write_authorization_with_projection(
        folder,
        &projection,
        current_app_pubkey,
    );
    let can_admin =
        shared_folder_app_key_can_admin_with_projection(folder, &projection, current_app_pubkey);
    let local_role = share_role_for_authorization(can_admin, write_authorization);
    let mut shortcut_paths = share_shortcuts
        .iter()
        .filter(|shortcut| shortcut.share_id == folder.share_id)
        .map(|shortcut| shortcut.path.clone())
        .collect::<Vec<_>>();
    shortcut_paths.sort();
    SharedFolderView {
        share_id: folder.share_id,
        display_name: folder.display_name.clone(),
        source_path: folder.source_path.clone(),
        shared_with_me_path: folder.shared_with_me_path(),
        local_role,
        current_app_pubkey: current_app_pubkey.to_string(),
        key_status,
        write_authorization,
        can_write: write_authorization.is_authorized(),
        can_admin,
        current_key_epoch,
        has_current_key_wrap,
        key_unavailable,
        repair_needed,
        missing_key_wrap_count: missing_key_wrap_pubkeys.len(),
        missing_key_wrap_pubkeys,
        participant_count: active_share_member_count(folder),
        app_key_count: projection.active_facets.len(),
        members: shared_folder_member_views(folder, &projection, current_app_pubkey, can_admin),
        pending_invites: pending_share_invite_views(folder),
        shortcut_paths,
    }
}

fn pending_share_invite_views(folder: &SharedFolder) -> Vec<PendingShareInviteView> {
    let mut invites = folder
        .pending_invites
        .values()
        .map(|invite| PendingShareInviteView {
            representative_npub_hint: invite.representative_npub_hint.clone(),
            role: invite.role,
            status: invite.status,
            display_name: invite
                .display_name
                .clone()
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| invite.representative_npub_hint.clone()),
            created_at: invite.created_at,
        })
        .collect::<Vec<_>>();
    invites.sort_by(|left, right| {
        left.display_name.cmp(&right.display_name).then_with(|| {
            left.representative_npub_hint
                .cmp(&right.representative_npub_hint)
        })
    });
    invites
}

pub fn default_share_shortcut_path(
    share_shortcuts: &[ShareShortcut],
    display_name: &str,
    parent_path: &str,
) -> Result<String, SharingError> {
    let parent_path = if parent_path.trim_matches('/').is_empty() {
        String::new()
    } else {
        normalize_provider_path(parent_path)
            .map_err(|error| SharingError::Path(error.to_string()))?
    };
    let name = sanitized_provider_file_name(display_name);
    Ok(unique_share_shortcut_path(
        share_shortcuts,
        &parent_path,
        &name,
    ))
}

#[must_use]
pub fn shared_with_me_path(display_name: &str) -> String {
    format!(
        "{SHARED_WITH_ME_DIR}/{}",
        sanitized_provider_file_name(display_name)
    )
}

fn share_role_for_authorization(
    can_admin: bool,
    write_authorization: ShareRootWriteAuthorization,
) -> ShareRole {
    if can_admin {
        ShareRole::Admin
    } else if write_authorization.is_authorized() {
        ShareRole::Editor
    } else {
        ShareRole::Reader
    }
}

fn share_key_status(
    folder: &SharedFolder,
    projection: &IrisProfileRosterProjection,
    current_app_pubkey: &str,
    current_key_epoch: Option<u64>,
    missing_key_wrap_pubkeys: &[String],
) -> SharedFolderKeyStatus {
    match member_for_app_key_with_projection(folder, projection, current_app_pubkey) {
        Some(member) if member.status == ShareMemberStatus::Revoked => {
            return SharedFolderKeyStatus::Revoked;
        }
        Some(member) if !member.is_active() => return SharedFolderKeyStatus::NotARecipient,
        Some(_) => {}
        None => return SharedFolderKeyStatus::NotARecipient,
    }
    let Some(epoch) = current_key_epoch else {
        return SharedFolderKeyStatus::NoKeyEpoch;
    };
    match projection.key_wrap_status(current_app_pubkey, epoch) {
        crate::KeyWrapStatus::Available if missing_key_wrap_pubkeys.is_empty() => {
            SharedFolderKeyStatus::Available
        }
        crate::KeyWrapStatus::Available => SharedFolderKeyStatus::RepairNeeded,
        crate::KeyWrapStatus::RepairNeeded => SharedFolderKeyStatus::KeyUnavailable,
        crate::KeyWrapStatus::NotAKeyRecipient | crate::KeyWrapStatus::NoSuchFacet => {
            SharedFolderKeyStatus::NotARecipient
        }
        crate::KeyWrapStatus::Tombstoned => SharedFolderKeyStatus::Revoked,
        crate::KeyWrapStatus::NoSuchEpoch => SharedFolderKeyStatus::NoKeyEpoch,
    }
}

#[must_use]
pub fn shared_folder_app_key_can_write_roots(folder: &SharedFolder, app_pubkey: &str) -> bool {
    shared_folder_app_key_write_authorization(folder, app_pubkey).is_authorized()
}

#[must_use]
pub fn shared_folder_app_key_write_authorization(
    folder: &SharedFolder,
    app_pubkey: &str,
) -> ShareRootWriteAuthorization {
    let projection = folder.projection();
    shared_folder_app_key_write_authorization_with_projection(folder, &projection, app_pubkey)
}

#[must_use]
pub fn shared_folder_authorized_writer_pubkeys(folder: &SharedFolder) -> Vec<String> {
    let projection = folder.projection();
    shared_folder_participant_profiles_with_projection(&projection)
        .into_keys()
        .filter(|pubkey| {
            shared_folder_app_key_write_authorization_with_projection(folder, &projection, pubkey)
                .is_authorized()
        })
        .collect()
}

#[must_use]
pub fn shared_folder_app_key_can_admin(folder: &SharedFolder, app_pubkey: &str) -> bool {
    let projection = folder.projection();
    shared_folder_app_key_can_admin_with_projection(folder, &projection, app_pubkey)
}

#[must_use]
pub fn shared_folder_key_recipient_pubkeys(folder: &SharedFolder) -> Vec<String> {
    let projection = folder.projection();
    active_share_key_recipients(folder, &projection)
}

#[must_use]
pub fn shared_folder_missing_key_wrap_pubkeys(folder: &SharedFolder, epoch: u64) -> Vec<String> {
    let projection = folder.projection();
    active_share_key_recipients_missing_wraps(folder, &projection, epoch)
}

pub fn resolve_share_recipient_from_profile_evidence(
    profile_id: IrisProfileId,
    representative_pubkey: &str,
    roster_ops: &[SignedIrisProfileRosterOp],
    acceptances: &[SignedIrisProfileFacetAcceptance],
    display_name: Option<String>,
) -> Result<ResolvedShareRecipient, SharingError> {
    PublicKey::from_hex(representative_pubkey)
        .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
    let projection = project_iris_profile_roster(profile_id, roster_ops.to_vec());
    let representative_facet = projection
        .active_facets
        .get(representative_pubkey)
        .ok_or_else(|| {
            SharingError::RecipientResolution(format!(
                "representative pubkey is not active in IrisProfile {profile_id}"
            ))
        })?;
    if !representative_has_active_self_link(
        &projection,
        representative_pubkey,
        representative_facet,
        acceptances,
    ) {
        return Err(SharingError::RecipientResolution(
            "representative pubkey has no active self-signed profile link".to_string(),
        ));
    }

    let app_pubkeys = accepted_share_app_pubkeys(&projection, acceptances);
    if app_pubkeys.is_empty() {
        return Err(SharingError::RecipientResolution(
            "resolved IrisProfile has no accepted AppKeys for sharing".to_string(),
        ));
    }
    Ok(ResolvedShareRecipient {
        profile_id,
        representative_pubkey: representative_pubkey.to_string(),
        representative_npub: pubkey_npub(representative_pubkey),
        display_name: display_name.or_else(|| representative_facet.label.clone()),
        app_pubkeys,
        linked_social_pubkeys: accepted_social_pubkeys(&projection, acceptances),
    })
}

pub fn resolve_share_recipient_from_evidence(
    evidence: &ShareRecipientProfileEvidence,
    display_name: Option<String>,
) -> Result<ResolvedShareRecipient, SharingError> {
    let representative_pubkey = evidence_representative_pubkey(evidence)?;
    resolve_share_recipient_from_profile_evidence(
        evidence.profile_id,
        &representative_pubkey,
        &evidence.roster_ops,
        &evidence.acceptances,
        display_name.or_else(|| evidence.display_name.clone()),
    )
}

pub fn share_recipient_profile_evidence_for_app_key(
    profile_id: IrisProfileId,
    roster_ops: &[SignedIrisProfileRosterOp],
    app_key_keys: &Keys,
    display_name: Option<String>,
    accepted_at: i64,
) -> Result<ShareRecipientProfileEvidence, SharingError> {
    let app_pubkey = app_key_keys.public_key().to_hex();
    let projection = project_iris_profile_roster(profile_id, roster_ops.to_vec());
    let facet = projection.active_facets.get(&app_pubkey).ok_or_else(|| {
        SharingError::RecipientResolution(format!(
            "current AppKey is not active in IrisProfile {profile_id}"
        ))
    })?;
    if !facet.has_purpose(IrisProfileKeyPurpose::AppKey) {
        return Err(SharingError::RecipientResolution(
            "current key is not an AppKey facet".to_string(),
        ));
    }
    if !facet.capabilities.can_receive_key_wraps {
        return Err(SharingError::RecipientResolution(
            "current AppKey cannot receive share key wraps".to_string(),
        ));
    }

    let roster_op_id = active_app_key_add_facet_op_id(roster_ops, profile_id, &app_pubkey);
    let acceptance_event = build_iris_profile_facet_acceptance_event(
        app_key_keys,
        profile_id,
        [IrisProfileKeyPurpose::AppKey],
        roster_op_id,
        accepted_at,
    )?;
    let acceptance = parse_iris_profile_facet_acceptance_event(&acceptance_event)?;

    Ok(ShareRecipientProfileEvidence {
        profile_id,
        representative_pubkey: Some(app_pubkey.clone()),
        representative_npub: Some(pubkey_npub(&app_pubkey)),
        display_name: display_name.or_else(|| facet.label.clone()),
        roster_ops: roster_ops.to_vec(),
        acceptances: vec![acceptance],
    })
}

pub fn sign_share_access_snapshot(
    signer_keys: &Keys,
    folder: &SharedFolder,
    created_at: i64,
) -> Result<SignedShareAccessSnapshot, SharingError> {
    let signer_pubkey = signer_keys.public_key().to_hex();
    if !shared_folder_app_key_can_admin(folder, &signer_pubkey) {
        return Err(SharingError::CurrentAppKeyCannotAdminShare);
    }
    let mut content = folder.access.clone();
    content.updated_at = created_at;
    validate_share_access_snapshot_content(folder.share_id, &content)?;
    let content_json =
        serde_json::to_string(&content).map_err(|error| SharingError::Invite(error.to_string()))?;
    let ts = u64::try_from(created_at).unwrap_or(0);
    let event = EventBuilder::new(Kind::from(KIND_SHARE_ACCESS_SNAPSHOT), content_json)
        .tags(vec![
            Tag::identifier(share_access_snapshot_d_tag(folder.share_id)),
            Tag::custom(share_access_label_tag_kind(), [SHARE_ACCESS_LABEL]),
            Tag::custom(iris_profile_tag_kind(), [folder.share_id.to_string()]),
            Tag::public_key(signer_keys.public_key()),
        ])
        .custom_created_at(nostr_sdk::Timestamp::from(ts))
        .sign_with_keys(signer_keys)
        .map_err(|error| SharingError::AccessSnapshot(error.to_string()))?;
    parse_share_access_snapshot_event(&event)
}

pub fn parse_share_access_snapshot_event(
    event: &Event,
) -> Result<SignedShareAccessSnapshot, SharingError> {
    let kind = event.kind.as_u16();
    if kind != KIND_SHARE_ACCESS_SNAPSHOT {
        return Err(SharingError::AccessSnapshot(format!(
            "invalid kind: expected {KIND_SHARE_ACCESS_SNAPSHOT}, got {kind}"
        )));
    }
    let d_tag = event
        .tags
        .identifier()
        .ok_or_else(|| SharingError::AccessSnapshot("missing d tag".to_string()))?;
    if !has_share_access_label(event) {
        return Err(SharingError::AccessSnapshot(
            "missing share access label".to_string(),
        ));
    }
    let d_tag_share_id = parse_share_access_snapshot_d_tag(d_tag)?;
    event
        .verify()
        .map_err(|error| SharingError::AccessSnapshot(error.to_string()))?;
    let content: ShareAccessSnapshot = serde_json::from_str(&event.content)
        .map_err(|error| SharingError::AccessSnapshot(error.to_string()))?;
    if content.schema != SHARE_ACCESS_SNAPSHOT_SCHEMA {
        return Err(SharingError::AccessSnapshot(format!(
            "unsupported schema {}",
            content.schema
        )));
    }
    if content.resource_id != d_tag_share_id {
        return Err(SharingError::AccessSnapshot(format!(
            "d-tag share {} does not match content resource {}",
            d_tag_share_id, content.resource_id
        )));
    }
    let event_created_at = i64::try_from(event.created_at.as_secs()).unwrap_or(i64::MAX);
    if content.updated_at != event_created_at {
        return Err(SharingError::AccessSnapshot(format!(
            "event created_at {} does not match content updated_at {}",
            event_created_at, content.updated_at
        )));
    }
    let signer_pubkey = event.pubkey.to_hex();
    PublicKey::from_hex(&signer_pubkey)
        .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
    validate_share_access_snapshot_content(d_tag_share_id, &content)?;
    Ok(SignedShareAccessSnapshot {
        snapshot_id: event.id.to_hex(),
        signer_pubkey,
        content,
        event_json: event.as_json(),
    })
}

pub fn validate_signed_share_access_snapshot(
    folder: &SharedFolder,
    snapshot: &SignedShareAccessSnapshot,
) -> Result<(), SharingError> {
    validate_shared_folder_access_snapshot(folder)?;
    let event = Event::from_json(&snapshot.event_json)
        .map_err(|error| SharingError::AccessSnapshot(error.to_string()))?;
    let parsed = parse_share_access_snapshot_event(&event)?;
    if parsed.snapshot_id != snapshot.snapshot_id
        || parsed.signer_pubkey != snapshot.signer_pubkey
        || parsed.content != snapshot.content
    {
        return Err(SharingError::AccessSnapshot(
            "snapshot event_json does not match snapshot fields".to_string(),
        ));
    }
    if !shared_folder_app_key_can_admin(folder, &snapshot.signer_pubkey) {
        return Err(SharingError::CurrentAppKeyCannotAdminShare);
    }
    if snapshot.content != folder.access {
        return Err(SharingError::AccessSnapshot(
            "signed snapshot does not match folder access snapshot".to_string(),
        ));
    }
    Ok(())
}

fn validate_shared_folder_access_snapshot(folder: &SharedFolder) -> Result<(), SharingError> {
    validate_share_access_snapshot_content(folder.share_id, &folder.access)
}

fn validate_share_access_snapshot_content(
    share_id: IrisProfileId,
    snapshot: &ShareAccessSnapshot,
) -> Result<(), SharingError> {
    if snapshot.schema != SHARE_ACCESS_SNAPSHOT_SCHEMA {
        return Err(SharingError::AccessSnapshot(format!(
            "unsupported schema {}",
            snapshot.schema
        )));
    }
    if snapshot.resource_id != share_id {
        return Err(SharingError::AccessSnapshot(format!(
            "snapshot resource {} does not match share {}",
            snapshot.resource_id, share_id
        )));
    }
    for grant in &snapshot.grants {
        if let ShareAccessTarget::Pubkey { pubkey } = &grant.target {
            PublicKey::from_hex(pubkey)
                .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
        }
    }
    for (pubkey, device) in &snapshot.devices {
        PublicKey::from_hex(pubkey)
            .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
        if device.pubkey != *pubkey {
            return Err(SharingError::AccessSnapshot(format!(
                "device map key {} does not match device pubkey {}",
                pubkey, device.pubkey
            )));
        }
    }
    for tombstone in snapshot.tombstones.values() {
        PublicKey::from_hex(&tombstone.pubkey)
            .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
        PublicKey::from_hex(&tombstone.removed_by_pubkey)
            .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
    }
    for epoch in snapshot.key_epochs.values() {
        PublicKey::from_hex(&epoch.signed_by_pubkey)
            .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
        for pubkey in epoch.wrapped_dck.keys() {
            PublicKey::from_hex(pubkey)
                .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
        }
    }
    Ok(())
}

fn evidence_representative_pubkey(
    evidence: &ShareRecipientProfileEvidence,
) -> Result<String, SharingError> {
    if let Some(pubkey) = evidence
        .representative_pubkey
        .as_deref()
        .map(str::trim)
        .filter(|pubkey| !pubkey.is_empty())
    {
        return PublicKey::from_hex(pubkey)
            .map(|pubkey| pubkey.to_hex())
            .map_err(|error| SharingError::InvalidPubkey(error.to_string()));
    }
    if let Some(npub) = evidence
        .representative_npub
        .as_deref()
        .map(str::trim)
        .filter(|npub| !npub.is_empty())
    {
        return PublicKey::from_bech32(npub)
            .map(|pubkey| pubkey.to_hex())
            .map_err(|error| SharingError::InvalidPubkey(error.to_string()));
    }
    Err(SharingError::RecipientResolution(
        "recipient evidence is missing representative pubkey".to_string(),
    ))
}

fn active_app_key_add_facet_op_id(
    roster_ops: &[SignedIrisProfileRosterOp],
    profile_id: IrisProfileId,
    app_pubkey: &str,
) -> Option<String> {
    roster_ops
        .iter()
        .filter(|signed| signed.content.profile_id == profile_id)
        .filter(|signed| match &signed.content.op {
            IrisProfileRosterOp::AddFacet { facet } => facet.pubkey == app_pubkey,
            _ => false,
        })
        .max_by(|left, right| {
            left.content
                .created_at
                .cmp(&right.content.created_at)
                .then_with(|| left.op_id.cmp(&right.op_id))
        })
        .map(|signed| signed.op_id.clone())
}

fn shared_folder_app_key_write_authorization_with_projection(
    folder: &SharedFolder,
    projection: &IrisProfileRosterProjection,
    app_pubkey: &str,
) -> ShareRootWriteAuthorization {
    let direct_grant = active_direct_pubkey_grant(&folder.access, app_pubkey);
    let member = if let Some(profile_id) =
        shared_folder_profile_id_for_app_key_with_projection(projection, app_pubkey)
    {
        let members = shared_folder_members_with_projection(folder, projection);
        let Some(member) = members.get(&profile_id.to_string()).cloned() else {
            return ShareRootWriteAuthorization::UnknownMember;
        };
        member
    } else if let Some(grant) = direct_grant {
        ShareMember {
            profile_id: folder.share_id,
            role: grant.role,
            status: grant.status,
            representative_npub_hint: grant.representative_npub_hint.clone(),
            display_name: grant.display_name.clone(),
        }
    } else {
        return ShareRootWriteAuthorization::UnknownAppKey;
    };
    match member.status {
        ShareMemberStatus::Pending => return ShareRootWriteAuthorization::PendingMember,
        ShareMemberStatus::Revoked => return ShareRootWriteAuthorization::RevokedMember,
        ShareMemberStatus::Active => {}
    }
    if member.role.rank() < ShareRole::Editor.rank() {
        return ShareRootWriteAuthorization::InsufficientShareRole;
    }
    let Some(facet) = projection.active_facets.get(app_pubkey) else {
        return ShareRootWriteAuthorization::AppKeyNotActive;
    };
    if !facet.is_app_key() {
        return ShareRootWriteAuthorization::NotAnAppKey;
    }
    if !facet.capabilities.can_write_roots {
        return ShareRootWriteAuthorization::AppKeyCannotWriteRoots;
    }
    ShareRootWriteAuthorization::Authorized
}

fn shared_folder_app_key_can_admin_with_projection(
    folder: &SharedFolder,
    projection: &IrisProfileRosterProjection,
    app_pubkey: &str,
) -> bool {
    (active_member_for_app_key_with_projection(folder, projection, app_pubkey)
        .is_some_and(|member| member.role == ShareRole::Admin)
        || active_direct_pubkey_grant(&folder.access, app_pubkey)
            .is_some_and(|grant| grant.role == ShareRole::Admin))
        && projection.can_admin_profile(app_pubkey)
}

fn active_direct_pubkey_grant<'a>(
    snapshot: &'a ShareAccessSnapshot,
    app_pubkey: &str,
) -> Option<&'a ShareAccessGrant> {
    snapshot.grants.iter().find(|grant| {
        matches!(&grant.target, ShareAccessTarget::Pubkey { pubkey } if pubkey == app_pubkey)
            && grant.status == ShareMemberStatus::Active
    })
}

fn active_share_key_recipients_missing_wraps(
    folder: &SharedFolder,
    projection: &IrisProfileRosterProjection,
    epoch: u64,
) -> Vec<String> {
    let Some(key_epoch) = projection.key_epochs.get(&epoch) else {
        return Vec::new();
    };
    active_share_key_recipients(folder, projection)
        .into_iter()
        .filter(|pubkey| !key_epoch.wrapped_dck.contains_key(pubkey))
        .collect()
}

fn active_share_key_recipients(
    folder: &SharedFolder,
    projection: &IrisProfileRosterProjection,
) -> Vec<String> {
    projection
        .active_facets
        .values()
        .filter(|facet| facet.capabilities.can_receive_key_wraps)
        .filter(|facet| {
            active_member_for_app_key_with_projection(folder, projection, &facet.pubkey).is_some()
        })
        .map(|facet| facet.pubkey.clone())
        .collect()
}

fn representative_has_active_self_link(
    projection: &IrisProfileRosterProjection,
    representative_pubkey: &str,
    facet: &IrisProfileFacet,
    acceptances: &[SignedIrisProfileFacetAcceptance],
) -> bool {
    (facet.has_purpose(IrisProfileKeyPurpose::SocialProfile)
        && has_active_facet_acceptance(
            projection,
            representative_pubkey,
            IrisProfileKeyPurpose::SocialProfile,
            acceptances,
        ))
        || (facet.has_purpose(IrisProfileKeyPurpose::AppKey)
            && has_active_facet_acceptance(
                projection,
                representative_pubkey,
                IrisProfileKeyPurpose::AppKey,
                acceptances,
            ))
}

fn accepted_share_app_pubkeys(
    projection: &IrisProfileRosterProjection,
    acceptances: &[SignedIrisProfileFacetAcceptance],
) -> Vec<String> {
    projection
        .active_facets
        .values()
        .filter(|facet| {
            facet.is_app_key()
                && facet.capabilities.can_receive_key_wraps
                && has_active_facet_acceptance(
                    projection,
                    &facet.pubkey,
                    IrisProfileKeyPurpose::AppKey,
                    acceptances,
                )
        })
        .map(|facet| facet.pubkey.clone())
        .collect()
}

fn accepted_social_pubkeys(
    projection: &IrisProfileRosterProjection,
    acceptances: &[SignedIrisProfileFacetAcceptance],
) -> Vec<String> {
    projection
        .active_facets
        .values()
        .filter(|facet| {
            facet.has_purpose(IrisProfileKeyPurpose::SocialProfile)
                && has_active_facet_acceptance(
                    projection,
                    &facet.pubkey,
                    IrisProfileKeyPurpose::SocialProfile,
                    acceptances,
                )
        })
        .map(|facet| facet.pubkey.clone())
        .collect()
}

fn has_active_facet_acceptance(
    projection: &IrisProfileRosterProjection,
    facet_pubkey: &str,
    purpose: IrisProfileKeyPurpose,
    acceptances: &[SignedIrisProfileFacetAcceptance],
) -> bool {
    acceptances.iter().any(|acceptance| {
        acceptance.content.facet_pubkey == facet_pubkey
            && acceptance.content.profile_id == projection.profile_id
            && acceptance.content.purposes.contains(&purpose)
            && acceptance.is_active_in_roster(projection)
    })
}

#[must_use]
pub fn share_access_snapshot_d_tag(share_id: IrisProfileId) -> String {
    share_id.to_string()
}

#[must_use]
pub fn is_share_access_snapshot_event_coordinate(event: &Event) -> bool {
    event.kind.as_u16() == KIND_SHARE_ACCESS_SNAPSHOT
        && has_share_access_label(event)
        && event
            .tags
            .identifier()
            .is_some_and(|d_tag| parse_share_access_snapshot_d_tag(d_tag).is_ok())
}

#[must_use]
fn share_access_label_tag_kind() -> TagKind<'static> {
    TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::L))
}

#[must_use]
fn has_share_access_label(event: &Event) -> bool {
    event.tags.iter().any(|tag| {
        let parts = tag.as_slice();
        parts.len() >= 2 && parts[0] == "l" && parts[1] == SHARE_ACCESS_LABEL
    })
}

fn parse_share_access_snapshot_d_tag(d_tag: &str) -> Result<IrisProfileId, SharingError> {
    d_tag
        .parse::<IrisProfileId>()
        .map_err(|error| SharingError::AccessSnapshot(format!("invalid share UUID: {error}")))
}

fn active_share_member_count(folder: &SharedFolder) -> usize {
    let projection = folder.projection();
    shared_folder_members_with_projection(folder, &projection)
        .values()
        .filter(|member| member.is_active())
        .count()
}

fn shared_folder_member_views(
    folder: &SharedFolder,
    projection: &IrisProfileRosterProjection,
    current_app_pubkey: &str,
    current_app_key_can_admin: bool,
) -> Vec<SharedFolderMemberView> {
    let mut app_key_counts = BTreeMap::<IrisProfileId, usize>::new();
    for profile_id in shared_folder_participant_profiles_with_projection(projection).values() {
        *app_key_counts.entry(*profile_id).or_default() += 1;
    }
    let current_profile_id =
        shared_folder_profile_id_for_app_key_with_projection(projection, current_app_pubkey);
    let mut members = shared_folder_members_with_projection(folder, projection)
        .values()
        .map(|member| SharedFolderMemberView {
            profile_id: member.profile_id,
            role: member.role,
            status: member.status,
            display_name: share_member_display_name(member),
            representative_npub_hint: member.representative_npub_hint.clone(),
            app_key_count: app_key_counts
                .get(&member.profile_id)
                .copied()
                .unwrap_or_default(),
            can_revoke: current_app_key_can_admin
                && member.status != ShareMemberStatus::Revoked
                && current_profile_id != Some(member.profile_id),
            can_change_role: current_app_key_can_admin
                && member.status != ShareMemberStatus::Revoked
                && current_profile_id != Some(member.profile_id),
        })
        .collect::<Vec<_>>();
    members.sort_by(|left, right| {
        left.display_name
            .cmp(&right.display_name)
            .then_with(|| left.profile_id.cmp(&right.profile_id))
    });
    members
}

fn share_member_display_name(member: &ShareMember) -> String {
    member
        .display_name
        .as_deref()
        .or(member.representative_npub_hint.as_deref())
        .map_or_else(|| member.profile_id.to_string(), ToOwned::to_owned)
}

fn key_status_has_current_wrap(status: SharedFolderKeyStatus) -> bool {
    matches!(
        status,
        SharedFolderKeyStatus::Available | SharedFolderKeyStatus::RepairNeeded
    )
}

fn unique_share_shortcut_path(
    share_shortcuts: &[ShareShortcut],
    parent_path: &str,
    name: &str,
) -> String {
    let prefix = if parent_path.is_empty() {
        String::new()
    } else {
        format!("{parent_path}/")
    };
    let existing = share_shortcuts
        .iter()
        .map(|shortcut| shortcut.path.as_str())
        .collect::<BTreeSet<_>>();
    let mut candidate = format!("{prefix}{name}");
    if !existing.contains(candidate.as_str()) {
        return candidate;
    }

    let path = std::path::Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Shared folder");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let mut index = 2;
    while existing.contains(candidate.as_str()) {
        candidate = format!("{prefix}{stem} ({index}){extension}");
        index += 1;
    }
    candidate
}

pub fn create_shared_folder(
    owner_keys: &nostr_sdk::Keys,
    owner_profile_id: IrisProfileId,
    source_path: &str,
    display_name: &str,
    local_label: Option<String>,
    recipients: Vec<ShareRecipient>,
    created_at: i64,
) -> Result<SharedFolder, SharingError> {
    let source_path = normalize_provider_path(source_path)
        .map_err(|error| SharingError::Path(error.to_string()))?;
    let display_name = sanitized_provider_file_name(display_name);
    let share_id = IrisProfileId::new_v4();
    let owner_pubkey = owner_keys.public_key().to_hex();
    let participants = collect_share_participants(recipients)?;
    let mut access = ShareAccessSnapshot::new(share_id, created_at);
    upsert_access_id_grant(
        &mut access,
        ShareMember::active(
            owner_profile_id,
            ShareRole::Admin,
            None,
            local_label.clone(),
        ),
    );
    upsert_access_device(
        &mut access,
        owner_pubkey,
        Some(owner_profile_id),
        local_label,
        created_at,
    );
    for recipient in participants.recipients_by_app_key.values() {
        upsert_access_id_grant(
            &mut access,
            ShareMember::active(
                recipient.profile_id,
                recipient.role,
                recipient.representative_npub_hint.clone(),
                recipient.display_name.clone(),
            ),
        );
        upsert_access_device(
            &mut access,
            recipient.app_pubkey.clone(),
            Some(recipient.profile_id),
            recipient.label.clone(),
            created_at,
        );
    }
    let share_key = generate_share_key();
    let projection = project_share_access_snapshot(&access);
    let wrapped_dck = wrap_share_key(
        owner_keys,
        active_key_recipient_refs(&projection),
        &share_key,
    )?;
    upsert_share_key_epoch(&mut access, owner_keys, 1, created_at, wrapped_dck);

    Ok(SharedFolder {
        share_id,
        owner_profile_id,
        source_path,
        display_name,
        local_role: ShareRole::Admin,
        access,
        pending_invites: BTreeMap::new(),
        app_key_roots: BTreeMap::new(),
    })
}

struct ShareParticipants {
    recipients_by_app_key: BTreeMap<String, ShareRecipient>,
}

fn active_key_recipient_refs(
    projection: &IrisProfileRosterProjection,
) -> impl Iterator<Item = &str> {
    projection
        .active_facets
        .values()
        .filter(|facet| facet.capabilities.can_receive_key_wraps)
        .map(|facet| facet.pubkey.as_str())
}

fn collect_share_participants(
    recipients: Vec<ShareRecipient>,
) -> Result<ShareParticipants, SharingError> {
    let mut recipients_by_app_key = BTreeMap::<String, ShareRecipient>::new();

    for recipient in recipients {
        PublicKey::from_hex(&recipient.app_pubkey)
            .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
        recipients_by_app_key
            .entry(recipient.app_pubkey.clone())
            .and_modify(|existing| {
                existing.role = ShareRole::strongest(existing.role, recipient.role);
                if existing.label.is_none() {
                    existing.label.clone_from(&recipient.label);
                }
            })
            .or_insert(recipient);
    }

    Ok(ShareParticipants {
        recipients_by_app_key,
    })
}

fn upsert_access_id_grant(snapshot: &mut ShareAccessSnapshot, member: ShareMember) {
    if let Some(grant) = snapshot.grants.iter_mut().find(
        |grant| matches!(grant.target, ShareAccessTarget::Id { id } if id == member.profile_id),
    ) {
        grant.role = ShareRole::strongest(grant.role, member.role);
        if grant.status == ShareMemberStatus::Pending && member.status == ShareMemberStatus::Active
        {
            grant.status = ShareMemberStatus::Active;
        }
        if grant.representative_npub_hint.is_none() {
            grant
                .representative_npub_hint
                .clone_from(&member.representative_npub_hint);
        }
        if grant.display_name.is_none() {
            grant.display_name.clone_from(&member.display_name);
        }
        return;
    }
    snapshot.grants.push(ShareAccessGrant {
        target: ShareAccessTarget::id(member.profile_id),
        role: member.role,
        status: member.status,
        representative_npub_hint: member.representative_npub_hint,
        display_name: member.display_name,
    });
}

fn set_access_id_grant_role(
    snapshot: &mut ShareAccessSnapshot,
    profile_id: IrisProfileId,
    role: ShareRole,
) -> bool {
    if let Some(grant) = snapshot
        .grants
        .iter_mut()
        .find(|grant| matches!(grant.target, ShareAccessTarget::Id { id } if id == profile_id))
    {
        grant.role = role;
        return true;
    }
    false
}

fn revoke_access_id_grant(snapshot: &mut ShareAccessSnapshot, profile_id: IrisProfileId) -> bool {
    if let Some(grant) = snapshot
        .grants
        .iter_mut()
        .find(|grant| matches!(grant.target, ShareAccessTarget::Id { id } if id == profile_id))
    {
        grant.status = ShareMemberStatus::Revoked;
        return true;
    }
    false
}

fn upsert_access_device(
    snapshot: &mut ShareAccessSnapshot,
    pubkey: String,
    profile_id: Option<IrisProfileId>,
    label: Option<String>,
    added_at: i64,
) {
    snapshot
        .devices
        .entry(pubkey.clone())
        .and_modify(|device| {
            if profile_id.is_some() {
                device.profile_id = profile_id;
            }
            if device.label.is_none() {
                device.label.clone_from(&label);
            }
        })
        .or_insert(ShareAccessDevice {
            pubkey,
            profile_id,
            added_at,
            label,
        });
}

fn upsert_share_key_epoch(
    snapshot: &mut ShareAccessSnapshot,
    signer_keys: &Keys,
    epoch: u64,
    created_at: i64,
    wrapped_dck: BTreeMap<String, String>,
) {
    snapshot.key_epochs.insert(
        epoch,
        IrisProfileKeyEpoch {
            epoch,
            created_at,
            signed_by_pubkey: signer_keys.public_key().to_hex(),
            wrapped_dck,
        },
    );
    snapshot.updated_at = snapshot.updated_at.max(created_at);
}

fn upsert_share_member(members: &mut BTreeMap<String, ShareMember>, recipient: &ShareRecipient) {
    members
        .entry(recipient.profile_id.to_string())
        .and_modify(|existing| {
            existing.role = ShareRole::strongest(existing.role, recipient.role);
            if existing.representative_npub_hint.is_none() {
                existing
                    .representative_npub_hint
                    .clone_from(&recipient.representative_npub_hint);
            }
            if existing.display_name.is_none() {
                existing.display_name.clone_from(&recipient.display_name);
            }
        })
        .or_insert_with(|| {
            ShareMember::active(
                recipient.profile_id,
                recipient.role,
                recipient.representative_npub_hint.clone(),
                recipient.display_name.clone(),
            )
        });
}

pub fn current_shared_folder_key(
    folder: &SharedFolder,
    app_keys: &Keys,
) -> Result<[u8; 32], SharingError> {
    let projection = folder.projection();
    let current_pubkey = app_keys.public_key().to_hex();
    let Some(member) = member_for_app_key_with_projection(folder, &projection, &current_pubkey)
    else {
        return Err(SharingError::NoWrapForCurrentAppKey);
    };
    if member.status == ShareMemberStatus::Revoked {
        return Err(SharingError::ShareMemberRevoked(member.profile_id));
    }
    if !member.is_active() {
        return Err(SharingError::NoWrapForCurrentAppKey);
    }
    let Some(facet) = projection.active_facets.get(&current_pubkey) else {
        return Err(SharingError::NoWrapForCurrentAppKey);
    };
    if !facet.is_app_key()
        || !facet.capabilities.can_receive_key_wraps
        || !facet.capabilities.can_decrypt_key_epochs
    {
        return Err(SharingError::NoWrapForCurrentAppKey);
    }
    let key_epoch = projection
        .key_epochs
        .values()
        .next_back()
        .ok_or(SharingError::NoKeyEpoch)?;
    let wrap = key_epoch
        .wrapped_dck
        .get(&current_pubkey)
        .ok_or(SharingError::NoWrapForCurrentAppKey)?;
    let signer_pubkey = PublicKey::from_hex(&key_epoch.signed_by_pubkey)
        .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
    let bytes = nip44::decrypt_to_bytes(app_keys.secret_key(), &signer_pubkey, wrap)
        .map_err(|error| SharingError::Unwrap(error.to_string()))?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| SharingError::InvalidShareKeyLength(bytes.len()))
}

pub fn invite_shared_folder_member(
    folder: &mut SharedFolder,
    signer_keys: &Keys,
    recipient: ShareRecipient,
    created_at: i64,
) -> Result<ShareInviteOutcome, SharingError> {
    invite_shared_folder_recipients(folder, signer_keys, vec![recipient], created_at)
}

pub fn record_pending_share_invite(
    folder: &mut SharedFolder,
    signer_keys: &Keys,
    representative_npub_hint: &str,
    role: ShareRole,
    display_name: Option<String>,
    created_at: i64,
) -> Result<PendingShareInvite, SharingError> {
    let signer_pubkey = signer_keys.public_key().to_hex();
    if !shared_folder_app_key_can_admin(folder, &signer_pubkey) {
        return Err(SharingError::CurrentAppKeyCannotAdminShare);
    }
    let representative_npub_hint = normalize_representative_npub_hint(representative_npub_hint)?;
    let display_name = display_name
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let invite = PendingShareInvite::new(
        representative_npub_hint.clone(),
        role,
        display_name,
        created_at,
    );
    let stored = folder
        .pending_invites
        .entry(representative_npub_hint)
        .and_modify(|existing| {
            existing.role = ShareRole::strongest(existing.role, invite.role);
            existing.status = ShareMemberStatus::Pending;
            if invite.display_name.is_some() {
                existing.display_name.clone_from(&invite.display_name);
            }
            existing.created_at = invite.created_at;
        })
        .or_insert_with(|| invite.clone())
        .clone();
    Ok(stored)
}

pub fn invite_shared_folder_resolved_recipient(
    folder: &mut SharedFolder,
    signer_keys: &Keys,
    recipient: &ResolvedShareRecipient,
    role: ShareRole,
    created_at: i64,
) -> Result<ShareInviteOutcome, SharingError> {
    invite_shared_folder_recipients(
        folder,
        signer_keys,
        recipient.share_recipients(role),
        created_at,
    )
}

fn invite_shared_folder_recipients(
    folder: &mut SharedFolder,
    signer_keys: &Keys,
    recipients: Vec<ShareRecipient>,
    created_at: i64,
) -> Result<ShareInviteOutcome, SharingError> {
    let signer_pubkey = signer_keys.public_key().to_hex();
    if !shared_folder_app_key_can_admin(folder, &signer_pubkey) {
        return Err(SharingError::CurrentAppKeyCannotAdminShare);
    }
    let recipient_count = recipients.len();
    let mut invited = None;
    for (index, recipient) in recipients.into_iter().enumerate() {
        let update_time = created_at.saturating_add(i64::try_from(index).unwrap_or(i64::MAX));
        invited = Some(add_invited_share_member(folder, recipient, update_time)?);
    }
    let Some(invited) = invited else {
        return Err(SharingError::RecipientResolution(
            "share invite requires at least one recipient AppKey".to_string(),
        ));
    };
    let projection = folder.projection();
    let next_epoch = projection
        .key_epochs
        .keys()
        .next_back()
        .map_or(1, |epoch| epoch.saturating_add(1));
    let op_offset = i64::try_from(recipient_count).unwrap_or(i64::MAX);
    let rotate_time = next_share_access_update_time(folder, created_at.saturating_add(op_offset));
    let share_key = generate_share_key();
    let wrapped_dck = wrap_share_key(
        signer_keys,
        active_key_recipient_refs(&projection),
        &share_key,
    )?;
    upsert_share_key_epoch(
        &mut folder.access,
        signer_keys,
        next_epoch,
        rotate_time,
        wrapped_dck,
    );
    let bundle = ShareInviteBundle {
        schema: SHARE_INVITE_SCHEMA,
        shared_folder: folder.clone(),
        recipient_profile_id: invited.profile_id,
        role: invited.role,
        representative_npub_hint: invited.representative_npub_hint,
        access_snapshot: Some(sign_share_access_snapshot(
            signer_keys,
            folder,
            folder.access.updated_at,
        )?),
        created_at,
    };
    let invite_url = encode_share_invite(&bundle)?;
    Ok(ShareInviteOutcome {
        share_id: folder.share_id,
        profile_id: bundle.recipient_profile_id,
        epoch: next_epoch,
        invite_url,
    })
}

struct InvitedShareMember {
    profile_id: IrisProfileId,
    role: ShareRole,
    representative_npub_hint: Option<String>,
}

fn add_invited_share_member(
    folder: &mut SharedFolder,
    recipient: ShareRecipient,
    created_at: i64,
) -> Result<InvitedShareMember, SharingError> {
    PublicKey::from_hex(&recipient.app_pubkey)
        .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
    let member_key = recipient.profile_id.to_string();
    let projection = folder.projection();
    let mut members = shared_folder_members_with_projection(folder, &projection);
    if members
        .get(&member_key)
        .is_some_and(|member| member.status == ShareMemberStatus::Revoked)
    {
        return Err(SharingError::ShareMemberRevoked(recipient.profile_id));
    }
    upsert_share_member(&mut members, &recipient);
    let member = members.get(&member_key).cloned().unwrap_or_else(|| {
        ShareMember::active(
            recipient.profile_id,
            recipient.role,
            recipient.representative_npub_hint.clone(),
            recipient.display_name.clone(),
        )
    });
    upsert_access_id_grant(&mut folder.access, member.clone());
    upsert_access_device(
        &mut folder.access,
        recipient.app_pubkey,
        Some(recipient.profile_id),
        recipient.label,
        created_at,
    );
    folder.access.updated_at = folder.access.updated_at.max(created_at);
    if let Some(hint) = &recipient.representative_npub_hint
        && let Ok(normalized_hint) = normalize_representative_npub_hint(hint)
    {
        folder.pending_invites.remove(&normalized_hint);
    }
    Ok(InvitedShareMember {
        profile_id: recipient.profile_id,
        role: member.role,
        representative_npub_hint: recipient.representative_npub_hint,
    })
}

fn normalize_representative_npub_hint(input: &str) -> Result<String, SharingError> {
    let trimmed = input.trim();
    if trimmed.starts_with("npub1") {
        let pubkey = PublicKey::from_bech32(trimmed)
            .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
        return Ok(pubkey_npub(&pubkey.to_hex()));
    }
    if trimmed.len() == 64 && trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Ok(pubkey_npub(&trimmed.to_ascii_lowercase()));
    }
    Err(SharingError::InvalidPubkey(trimmed.to_string()))
}

pub fn encode_share_invite(bundle: &ShareInviteBundle) -> Result<String, SharingError> {
    if bundle.schema != SHARE_INVITE_SCHEMA {
        return Err(SharingError::Invite(format!(
            "unsupported schema {}",
            bundle.schema
        )));
    }
    let json =
        serde_json::to_vec(bundle).map_err(|error| SharingError::Invite(error.to_string()))?;
    Ok(format!(
        "{SHARE_INVITE_PREFIX}{}",
        URL_SAFE_NO_PAD.encode(json)
    ))
}

pub fn parse_share_invite(input: &str) -> Result<ShareInviteBundle, SharingError> {
    let trimmed = input.trim();
    let encoded = trimmed.strip_prefix(SHARE_INVITE_PREFIX).unwrap_or(trimmed);
    let bytes = URL_SAFE_NO_PAD
        .decode(encoded)
        .map_err(|error| SharingError::Invite(error.to_string()))?;
    let bundle: ShareInviteBundle =
        serde_json::from_slice(&bytes).map_err(|error| SharingError::Invite(error.to_string()))?;
    if bundle.schema != SHARE_INVITE_SCHEMA {
        return Err(SharingError::Invite(format!(
            "unsupported schema {}",
            bundle.schema
        )));
    }
    validate_shared_folder_access_snapshot(&bundle.shared_folder)?;
    if let Some(snapshot) = &bundle.access_snapshot {
        validate_signed_share_access_snapshot(&bundle.shared_folder, snapshot)?;
    }
    Ok(bundle)
}

pub fn shared_folder_from_invite_for_profile(
    invite: &str,
    local_profile_id: IrisProfileId,
) -> Result<SharedFolder, SharingError> {
    let bundle = parse_share_invite(invite)?;
    let projection = bundle.shared_folder.projection();
    let members = shared_folder_members_with_projection(&bundle.shared_folder, &projection);
    if bundle.recipient_profile_id != local_profile_id
        || !members.contains_key(&local_profile_id.to_string())
    {
        return Err(SharingError::ShareInviteNotForLocalProfile { local_profile_id });
    }
    Ok(bundle.shared_folder)
}

/// Add missing share-key wraps for the current key epoch without rotating the
/// share key. Only the `AppKey` that signed the epoch may repair it, mirroring
/// the profile key-wrap rule and keeping divergent roster repairs deterministic.
pub fn repair_shared_folder_key_epoch_wraps(
    folder: &mut SharedFolder,
    signer_keys: &Keys,
    created_at: i64,
) -> Result<ShareKeyRepairOutcome, SharingError> {
    let projection = folder.projection();
    let Some((epoch, key_epoch)) = projection.key_epochs.iter().next_back() else {
        return Err(SharingError::NoKeyEpoch);
    };
    let epoch = *epoch;
    let epoch_signer_pubkey = key_epoch.signed_by_pubkey.clone();
    let signer_pubkey = signer_keys.public_key().to_hex();
    if epoch_signer_pubkey != signer_pubkey {
        return Err(SharingError::CurrentAppKeyCannotRepairKeyEpoch {
            signed_by_pubkey: epoch_signer_pubkey,
        });
    }
    if !shared_folder_app_key_can_admin_with_projection(folder, &projection, &signer_pubkey) {
        return Err(SharingError::CurrentAppKeyCannotAdminShare);
    }
    let Some(signer_facet) = projection.active_facets.get(&signer_pubkey) else {
        return Err(SharingError::CurrentAppKeyCannotRepairKeyEpochs);
    };
    if !signer_facet.capabilities.can_change_key_epochs() {
        return Err(SharingError::CurrentAppKeyCannotRepairKeyEpochs);
    }
    let missing_pubkeys = active_share_key_recipients_missing_wraps(folder, &projection, epoch);
    if missing_pubkeys.is_empty() {
        return Ok(ShareKeyRepairOutcome {
            share_id: folder.share_id,
            epoch,
            repaired_pubkeys: Vec::new(),
        });
    }

    let share_key = current_shared_folder_key(folder, signer_keys)?;
    let wrapped_dck = wrap_share_key(
        signer_keys,
        missing_pubkeys.iter().map(String::as_str),
        &share_key,
    )?;
    if let Some(key_epoch) = folder.access.key_epochs.get_mut(&epoch) {
        key_epoch.wrapped_dck.extend(wrapped_dck);
        folder.access.updated_at = folder.access.updated_at.max(created_at);
    }
    Ok(ShareKeyRepairOutcome {
        share_id: folder.share_id,
        epoch,
        repaired_pubkeys: missing_pubkeys,
    })
}

pub fn set_shared_folder_member_role(
    folder: &mut SharedFolder,
    signer_keys: &Keys,
    profile_id: IrisProfileId,
    role: ShareRole,
    created_at: i64,
) -> Result<ShareMemberRoleOutcome, SharingError> {
    let signer_pubkey = signer_keys.public_key().to_hex();
    if !shared_folder_app_key_can_admin(folder, &signer_pubkey) {
        return Err(SharingError::CurrentAppKeyCannotAdminShare);
    }
    let projection = folder.projection();
    let signer_profile_id =
        shared_folder_profile_id_for_app_key_with_projection(&projection, &signer_pubkey)
            .ok_or(SharingError::CurrentAppKeyCannotAdminShare)?;
    if signer_profile_id == profile_id {
        return Err(SharingError::CannotChangeCurrentShareMemberRole);
    }
    let member_key = profile_id.to_string();
    let members = shared_folder_members_with_projection(folder, &projection);
    let Some(member) = members.get(&member_key) else {
        return Err(SharingError::ShareMemberNotFound(profile_id));
    };
    if member.status == ShareMemberStatus::Revoked {
        return Err(SharingError::ShareMemberRevoked(profile_id));
    }

    if member.role != role {
        set_access_id_grant_role(&mut folder.access, profile_id, role);
        folder.access.updated_at = next_share_access_update_time(folder, created_at);
    }

    Ok(ShareMemberRoleOutcome {
        share_id: folder.share_id,
        profile_id,
        role,
    })
}

pub fn revoke_shared_folder_member(
    folder: &mut SharedFolder,
    signer_keys: &Keys,
    profile_id: IrisProfileId,
    reason: Option<&str>,
    created_at: i64,
) -> Result<ShareMemberRevokeOutcome, SharingError> {
    let signer_pubkey = signer_keys.public_key().to_hex();
    if !shared_folder_app_key_can_admin(folder, &signer_pubkey) {
        return Err(SharingError::CurrentAppKeyCannotAdminShare);
    }
    let projection = folder.projection();
    let signer_profile_id =
        shared_folder_profile_id_for_app_key_with_projection(&projection, &signer_pubkey)
            .ok_or(SharingError::CurrentAppKeyCannotAdminShare)?;
    if signer_profile_id == profile_id {
        return Err(SharingError::CannotRevokeCurrentShareMember);
    }
    let member_key = profile_id.to_string();
    let members = shared_folder_members_with_projection(folder, &projection);
    let Some(member) = members.get(&member_key) else {
        return Err(SharingError::ShareMemberNotFound(profile_id));
    };
    if member.status == ShareMemberStatus::Revoked {
        let epoch = folder
            .projection()
            .key_epochs
            .keys()
            .next_back()
            .copied()
            .unwrap_or_default();
        return Ok(ShareMemberRevokeOutcome {
            share_id: folder.share_id,
            profile_id,
            epoch,
            revoked_app_pubkeys: Vec::new(),
        });
    }
    revoke_access_id_grant(&mut folder.access, profile_id);

    let mut revoked_app_pubkeys = shared_folder_participant_profiles_with_projection(&projection)
        .into_iter()
        .filter(|(_, member_profile_id)| *member_profile_id == profile_id)
        .map(|(app_pubkey, _)| app_pubkey)
        .collect::<Vec<_>>();
    revoked_app_pubkeys.sort();

    let mut op_time = next_share_access_update_time(folder, created_at);
    for app_pubkey in &revoked_app_pubkeys {
        folder.access.tombstones.insert(
            app_pubkey.clone(),
            IrisProfileTombstone {
                pubkey: app_pubkey.clone(),
                profile_id: Some(profile_id),
                removed_by_pubkey: signer_pubkey.clone(),
                removed_at: op_time,
                reason: reason.map(str::to_string),
            },
        );
        op_time += 1;
    }
    folder.access.updated_at = folder.access.updated_at.max(op_time);

    let current_projection = folder.projection();
    let next_epoch = current_projection
        .key_epochs
        .keys()
        .next_back()
        .map_or(1, |epoch| epoch.saturating_add(1));
    let share_key = generate_share_key();
    let wrapped_dck = wrap_share_key(
        signer_keys,
        active_key_recipient_refs(&current_projection),
        &share_key,
    )?;
    upsert_share_key_epoch(
        &mut folder.access,
        signer_keys,
        next_epoch,
        op_time,
        wrapped_dck,
    );

    Ok(ShareMemberRevokeOutcome {
        share_id: folder.share_id,
        profile_id,
        epoch: next_epoch,
        revoked_app_pubkeys,
    })
}

pub fn refresh_shared_folder_member_statuses_from_access(_folder: &mut SharedFolder) {}

fn generate_share_key() -> [u8; 32] {
    let keys = Keys::generate();
    let mut out = [0_u8; 32];
    out.copy_from_slice(keys.secret_key().as_secret_bytes());
    out
}

fn wrap_share_key<'a, I>(
    signer_keys: &Keys,
    recipients: I,
    share_key: &[u8; 32],
) -> Result<BTreeMap<String, String>, SharingError>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut wraps = BTreeMap::new();
    for recipient in recipients {
        let recipient_pk = PublicKey::from_hex(recipient)
            .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
        let ciphertext = nip44::encrypt(
            signer_keys.secret_key(),
            &recipient_pk,
            share_key.as_slice(),
            Nip44Version::V2,
        )
        .map_err(|error| SharingError::Wrap(error.to_string()))?;
        wraps.insert(recipient.to_string(), ciphertext);
    }
    Ok(wraps)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn share_recipient(keys: &Keys, profile_id: IrisProfileId, role: ShareRole) -> ShareRecipient {
        ShareRecipient {
            profile_id,
            app_pubkey: keys.public_key().to_hex(),
            role,
            label: Some(role.label().to_string()),
            representative_npub_hint: None,
            display_name: Some(role.label().to_string()),
        }
    }

    #[test]
    fn access_snapshot_authorizes_admin_writer_and_reader() {
        let owner_keys = Keys::generate();
        let owner_id = IrisProfileId::new_v4();
        let writer_keys = Keys::generate();
        let writer_id = IrisProfileId::new_v4();
        let reader_keys = Keys::generate();
        let reader_id = IrisProfileId::new_v4();
        let mut folder = create_shared_folder(
            &owner_keys,
            owner_id,
            "Projects/Alpha",
            "Alpha",
            Some("Owner".into()),
            vec![
                share_recipient(&writer_keys, writer_id, ShareRole::Editor),
                share_recipient(&reader_keys, reader_id, ShareRole::Reader),
            ],
            10,
        )
        .unwrap();

        assert!(shared_folder_app_key_can_admin(
            &folder,
            &owner_keys.public_key().to_hex()
        ));
        assert!(shared_folder_app_key_can_write_roots(
            &folder,
            &writer_keys.public_key().to_hex()
        ));
        assert!(!shared_folder_app_key_can_write_roots(
            &folder,
            &reader_keys.public_key().to_hex()
        ));
        assert_eq!(
            current_shared_folder_key(&folder, &writer_keys)
                .unwrap()
                .len(),
            32
        );
        assert_eq!(
            current_shared_folder_key(&folder, &reader_keys)
                .unwrap()
                .len(),
            32
        );

        set_shared_folder_member_role(&mut folder, &owner_keys, reader_id, ShareRole::Editor, 20)
            .unwrap();
        assert!(shared_folder_app_key_can_write_roots(
            &folder,
            &reader_keys.public_key().to_hex()
        ));

        let revoked =
            revoke_shared_folder_member(&mut folder, &owner_keys, writer_id, None, 30).unwrap();
        assert_eq!(
            revoked.revoked_app_pubkeys,
            vec![writer_keys.public_key().to_hex()]
        );
        assert!(!shared_folder_app_key_can_write_roots(
            &folder,
            &writer_keys.public_key().to_hex()
        ));
        assert!(matches!(
            current_shared_folder_key(&folder, &writer_keys),
            Err(SharingError::ShareMemberRevoked(id)) if id == writer_id
        ));
    }

    #[test]
    fn signed_access_snapshot_roundtrips_and_validates_against_folder() {
        let owner_keys = Keys::generate();
        let owner_id = IrisProfileId::new_v4();
        let folder = create_shared_folder(
            &owner_keys,
            owner_id,
            "Projects/Beta",
            "Beta",
            Some("Owner".into()),
            Vec::new(),
            10,
        )
        .unwrap();

        let signed =
            sign_share_access_snapshot(&owner_keys, &folder, folder.access.updated_at).unwrap();
        let event = Event::from_json(&signed.event_json).unwrap();
        assert!(is_share_access_snapshot_event_coordinate(&event));
        assert_eq!(
            event.tags.identifier(),
            Some(folder.share_id.to_string().as_str())
        );
        assert!(event.tags.iter().any(|tag| {
            let parts = tag.as_slice();
            parts.len() >= 2 && parts[0] == "l" && parts[1] == SHARE_ACCESS_LABEL
        }));
        let parsed = parse_share_access_snapshot_event(&event).unwrap();
        assert_eq!(parsed.content, folder.access);
        validate_signed_share_access_snapshot(&folder, &signed).unwrap();
    }

    #[test]
    fn invite_carries_signed_access_snapshot_for_recipient() {
        let owner_keys = Keys::generate();
        let owner_id = IrisProfileId::new_v4();
        let recipient_keys = Keys::generate();
        let recipient_id = IrisProfileId::new_v4();
        let mut folder = create_shared_folder(
            &owner_keys,
            owner_id,
            "Projects/Gamma",
            "Gamma",
            Some("Owner".into()),
            Vec::new(),
            10,
        )
        .unwrap();

        let outcome = invite_shared_folder_member(
            &mut folder,
            &owner_keys,
            share_recipient(&recipient_keys, recipient_id, ShareRole::Reader),
            20,
        )
        .unwrap();
        let bundle = parse_share_invite(&outcome.invite_url).unwrap();
        assert_eq!(bundle.recipient_profile_id, recipient_id);
        assert!(bundle.access_snapshot.is_some());
        let accepted =
            shared_folder_from_invite_for_profile(&outcome.invite_url, recipient_id).unwrap();
        assert_eq!(accepted.share_id, folder.share_id);
        assert!(accepted.access.grants.iter().any(
            |grant| matches!(grant.target, ShareAccessTarget::Id { id } if id == recipient_id)
        ));
    }

    #[test]
    fn direct_pubkey_grant_authorizes_mvp_device_target() {
        let owner_keys = Keys::generate();
        let owner_id = IrisProfileId::new_v4();
        let device_keys = Keys::generate();
        let device_pubkey = device_keys.public_key().to_hex();
        let mut folder = create_shared_folder(
            &owner_keys,
            owner_id,
            "Projects/Delta",
            "Delta",
            Some("Owner".into()),
            Vec::new(),
            10,
        )
        .unwrap();
        folder.access.grants.push(ShareAccessGrant {
            target: ShareAccessTarget::pubkey(device_pubkey.clone()),
            role: ShareRole::Editor,
            status: ShareMemberStatus::Active,
            representative_npub_hint: None,
            display_name: Some("Direct device".into()),
        });
        upsert_access_device(
            &mut folder.access,
            device_pubkey.clone(),
            None,
            Some("Direct device".into()),
            20,
        );

        assert!(shared_folder_app_key_can_write_roots(
            &folder,
            &device_pubkey
        ));
        assert!(!shared_folder_app_key_can_admin(&folder, &device_pubkey));
    }
}
