use std::collections::{BTreeMap, BTreeSet};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use nostr_sdk::nips::nip19::FromBech32;
use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use crate::app_key_summary::pubkey_npub;
use crate::config::AppKeyRootRef;
use crate::iris_profile::{
    IrisProfileCapabilities, IrisProfileError, IrisProfileFacet, IrisProfileId,
    IrisProfileKeyPurpose, IrisProfileRosterOp, IrisProfileRosterProjection,
    SignedIrisProfileFacetAcceptance, SignedIrisProfileRosterOp,
    build_iris_profile_roster_op_event, iris_profile_roster_parent_ids, iris_profile_tag_kind,
    parse_iris_profile_roster_op_event, project_iris_profile_roster,
};
use crate::provider::{normalize_provider_path, sanitized_provider_file_name};

pub const SHARED_WITH_ME_DIR: &str = "Shared with me";
pub const SHARE_INVITE_SCHEMA: u32 = 1;
pub const SHARE_MEMBER_ROSTER_SCHEMA: u32 = 1;
pub const SHARE_ROSTER_CHECKPOINT_SCHEMA: u32 = 1;
pub const KIND_SHARE_MEMBER_ROSTER_OP: u16 = 30_078;
pub const KIND_SHARE_ROSTER_CHECKPOINT: u16 = 30_078;
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
    #[error("share invite: {0}")]
    Invite(String),
    #[error("share member roster: {0}")]
    ShareMemberRoster(String),
    #[error("share roster checkpoint: {0}")]
    RosterCheckpoint(String),
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
#[serde(tag = "op", rename_all = "snake_case")]
pub enum ShareMemberRosterOp {
    GrantMember {
        member: ShareMember,
    },
    SetMemberRole {
        profile_id: IrisProfileId,
        role: ShareRole,
    },
    RevokeMember {
        profile_id: IrisProfileId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareMemberRosterOpContent {
    pub schema: u32,
    pub share_id: IrisProfileId,
    pub actor_pubkey: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    pub client_nonce: String,
    pub created_at: i64,
    pub op: ShareMemberRosterOp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedShareMemberRosterOp {
    pub op_id: String,
    pub signer_pubkey: String,
    pub content: ShareMemberRosterOpContent,
    pub event_json: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareMemberRosterProjection {
    pub share_id: IrisProfileId,
    pub members: BTreeMap<String, ShareMember>,
    pub accepted_op_ids: Vec<String>,
    pub rejected_op_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedFolder {
    pub share_id: IrisProfileId,
    pub owner_profile_id: IrisProfileId,
    pub source_path: String,
    pub display_name: String,
    pub local_role: ShareRole,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub members: BTreeMap<String, ShareMember>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub member_ops: Vec<SignedShareMemberRosterOp>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub participant_profiles: BTreeMap<String, IrisProfileId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub app_key_roots: BTreeMap<String, AppKeyRootRef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roster_ops: Vec<SignedIrisProfileRosterOp>,
}

impl SharedFolder {
    #[must_use]
    pub fn projection(&self) -> IrisProfileRosterProjection {
        project_iris_profile_roster(self.share_id, self.roster_ops.clone())
    }

    #[must_use]
    pub fn member_projection(&self) -> ShareMemberRosterProjection {
        let projection = self.projection();
        project_shared_folder_member_roster_with_projection(self, &projection)
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
    folder: &SharedFolder,
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
        .or_else(|| folder.participant_profiles.get(app_pubkey).copied())
}

fn shared_folder_participant_profiles_with_projection(
    folder: &SharedFolder,
    projection: &IrisProfileRosterProjection,
) -> BTreeMap<String, IrisProfileId> {
    let mut participant_profiles = folder.participant_profiles.clone();
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

#[must_use]
pub fn project_shared_folder_member_roster(folder: &SharedFolder) -> ShareMemberRosterProjection {
    let projection = folder.projection();
    project_shared_folder_member_roster_with_projection(folder, &projection)
}

fn project_shared_folder_member_roster_from_parts(
    share_id: IrisProfileId,
    owner_profile_id: IrisProfileId,
    participant_profiles: &BTreeMap<String, IrisProfileId>,
    roster_ops: &[SignedIrisProfileRosterOp],
    member_ops: &[SignedShareMemberRosterOp],
) -> ShareMemberRosterProjection {
    let folder = SharedFolder {
        share_id,
        owner_profile_id,
        source_path: String::new(),
        display_name: String::new(),
        local_role: ShareRole::Admin,
        members: BTreeMap::new(),
        member_ops: member_ops.to_vec(),
        participant_profiles: participant_profiles.clone(),
        app_key_roots: BTreeMap::new(),
        roster_ops: roster_ops.to_vec(),
    };
    project_shared_folder_member_roster(&folder)
}

fn share_member_roster_parent_ids(folder: &SharedFolder) -> Vec<String> {
    project_shared_folder_member_roster(folder).accepted_op_ids
}

fn share_member_roster_parent_ids_for_parts(
    share_id: IrisProfileId,
    owner_profile_id: IrisProfileId,
    participant_profiles: &BTreeMap<String, IrisProfileId>,
    roster_ops: &[SignedIrisProfileRosterOp],
    member_ops: &[SignedShareMemberRosterOp],
) -> Vec<String> {
    project_shared_folder_member_roster_from_parts(
        share_id,
        owner_profile_id,
        participant_profiles,
        roster_ops,
        member_ops,
    )
    .accepted_op_ids
}

fn project_shared_folder_member_roster_with_projection(
    folder: &SharedFolder,
    key_projection: &IrisProfileRosterProjection,
) -> ShareMemberRosterProjection {
    project_shared_folder_member_roster_ops(folder, key_projection, folder.member_ops.clone())
}

fn project_shared_folder_member_roster_ops<I>(
    folder: &SharedFolder,
    key_projection: &IrisProfileRosterProjection,
    ops: I,
) -> ShareMemberRosterProjection
where
    I: IntoIterator<Item = SignedShareMemberRosterOp>,
{
    let mut projection = ShareMemberRosterProjection {
        share_id: folder.share_id,
        members: BTreeMap::new(),
        accepted_op_ids: Vec::new(),
        rejected_op_ids: Vec::new(),
    };
    let mut ops = ops
        .into_iter()
        .filter(|op| op.content.share_id == folder.share_id)
        .collect::<Vec<_>>();
    ops.sort_by(|left, right| {
        left.content
            .created_at
            .cmp(&right.content.created_at)
            .then_with(|| left.op_id.cmp(&right.op_id))
    });

    let mut accepted_ops_by_id = BTreeMap::new();
    for op in ops {
        if apply_share_member_roster_op(
            &mut projection,
            folder,
            key_projection,
            &op,
            &accepted_ops_by_id,
        ) {
            projection.accepted_op_ids.push(op.op_id.clone());
            accepted_ops_by_id.insert(op.op_id.clone(), op);
        } else {
            projection.rejected_op_ids.push(op.op_id);
        }
    }
    projection
}

fn apply_share_member_roster_op(
    projection: &mut ShareMemberRosterProjection,
    folder: &SharedFolder,
    key_projection: &IrisProfileRosterProjection,
    signed: &SignedShareMemberRosterOp,
    accepted_ops_by_id: &BTreeMap<String, SignedShareMemberRosterOp>,
) -> bool {
    if !share_member_signer_can_apply_with_parents(
        projection,
        folder,
        key_projection,
        signed,
        accepted_ops_by_id,
    ) {
        return false;
    }
    match &signed.content.op {
        ShareMemberRosterOp::GrantMember { member } => {
            if member.status == ShareMemberStatus::Revoked {
                return false;
            }
            merge_share_member_grant(&mut projection.members, member.clone())
        }
        ShareMemberRosterOp::SetMemberRole { profile_id, role } => {
            let Some(member) = projection.members.get_mut(&profile_id.to_string()) else {
                return false;
            };
            if member.status == ShareMemberStatus::Revoked {
                return false;
            }
            member.role = *role;
            true
        }
        ShareMemberRosterOp::RevokeMember { profile_id, .. } => {
            let Some(member) = projection.members.get_mut(&profile_id.to_string()) else {
                return false;
            };
            member.status = ShareMemberStatus::Revoked;
            true
        }
    }
}

fn merge_share_member_grant(
    members: &mut BTreeMap<String, ShareMember>,
    granted: ShareMember,
) -> bool {
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
    true
}

fn share_member_signer_can_apply_with_parents(
    projection: &ShareMemberRosterProjection,
    folder: &SharedFolder,
    key_projection: &IrisProfileRosterProjection,
    signed: &SignedShareMemberRosterOp,
    accepted_ops_by_id: &BTreeMap<String, SignedShareMemberRosterOp>,
) -> bool {
    if projection.members.is_empty() {
        return is_valid_share_member_bootstrap_op(folder, key_projection, signed);
    }
    if signed.content.parents.is_empty() {
        return false;
    }
    let Some(parent_projection) = project_share_member_parent_closure(
        folder,
        key_projection,
        &signed.content.parents,
        accepted_ops_by_id,
    ) else {
        return false;
    };
    share_member_signer_can_apply(folder, key_projection, &parent_projection.members, signed)
}

fn project_share_member_parent_closure(
    folder: &SharedFolder,
    key_projection: &IrisProfileRosterProjection,
    parents: &[String],
    accepted_ops_by_id: &BTreeMap<String, SignedShareMemberRosterOp>,
) -> Option<ShareMemberRosterProjection> {
    let mut pending = parents.to_vec();
    let mut seen = BTreeSet::new();
    while let Some(parent_id) = pending.pop() {
        if !seen.insert(parent_id.clone()) {
            continue;
        }
        let parent = accepted_ops_by_id.get(&parent_id)?;
        pending.extend(parent.content.parents.iter().cloned());
    }
    let parent_ops = seen
        .into_iter()
        .map(|op_id| accepted_ops_by_id.get(&op_id).cloned())
        .collect::<Option<Vec<_>>>()?;
    let parent_projection =
        project_shared_folder_member_roster_ops(folder, key_projection, parent_ops);
    if parents
        .iter()
        .all(|parent| parent_projection.accepted_op_ids.contains(parent))
    {
        Some(parent_projection)
    } else {
        None
    }
}

fn is_valid_share_member_bootstrap_op(
    folder: &SharedFolder,
    key_projection: &IrisProfileRosterProjection,
    signed: &SignedShareMemberRosterOp,
) -> bool {
    let ShareMemberRosterOp::GrantMember { member } = &signed.content.op else {
        return false;
    };
    member.profile_id == folder.owner_profile_id
        && member.role == ShareRole::Admin
        && member.status == ShareMemberStatus::Active
        && share_member_signer_profile_id(folder, key_projection, &signed.signer_pubkey)
            == Some(folder.owner_profile_id)
        && key_projection.can_admin_profile(&signed.signer_pubkey)
}

fn share_member_signer_can_apply(
    folder: &SharedFolder,
    key_projection: &IrisProfileRosterProjection,
    members: &BTreeMap<String, ShareMember>,
    signed: &SignedShareMemberRosterOp,
) -> bool {
    let Some(profile_id) =
        share_member_signer_profile_id(folder, key_projection, &signed.signer_pubkey)
    else {
        return false;
    };
    members
        .get(&profile_id.to_string())
        .is_some_and(|member| member.is_active() && member.role == ShareRole::Admin)
        && key_projection.can_admin_profile(&signed.signer_pubkey)
}

fn share_member_signer_profile_id(
    folder: &SharedFolder,
    key_projection: &IrisProfileRosterProjection,
    signer_pubkey: &str,
) -> Option<IrisProfileId> {
    shared_folder_profile_id_for_app_key_with_projection(folder, key_projection, signer_pubkey)
}

fn shared_folder_members_with_projection(
    folder: &SharedFolder,
    projection: &IrisProfileRosterProjection,
) -> BTreeMap<String, ShareMember> {
    if folder.member_ops.is_empty() {
        folder.members.clone()
    } else {
        project_shared_folder_member_roster_with_projection(folder, projection).members
    }
}

fn materialize_share_members_from_ops(folder: &mut SharedFolder) {
    if folder.member_ops.is_empty() {
        return;
    }
    folder.members = project_shared_folder_member_roster(folder).members;
}

fn next_share_member_op_time(folder: &SharedFolder, requested_at: i64) -> i64 {
    folder
        .member_ops
        .iter()
        .map(|op| op.content.created_at)
        .max()
        .map_or(requested_at, |last| {
            requested_at.max(last.saturating_add(1))
        })
}

fn member_for_app_key_with_projection(
    folder: &SharedFolder,
    projection: &IrisProfileRosterProjection,
    app_pubkey: &str,
) -> Option<ShareMember> {
    let profile_id =
        shared_folder_profile_id_for_app_key_with_projection(folder, projection, app_pubkey)?;
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
    pub missing_key_wrap_pubkeys: Vec<String>,
    pub participant_count: usize,
    pub app_key_count: usize,
    pub members: Vec<SharedFolderMemberView>,
    pub shortcut_paths: Vec<String>,
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
    pub roster_checkpoint: Option<SignedShareRosterCheckpoint>,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareRosterCheckpointContent {
    pub schema: u32,
    pub share_id: IrisProfileId,
    pub signer_pubkey: String,
    pub roster_head_op_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub member_roster_head_op_ids: Vec<String>,
    pub accepted_op_count: usize,
    pub rejected_op_count: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub accepted_member_op_count: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub rejected_member_op_count: usize,
    pub active_app_key_pubkeys: Vec<String>,
    pub tombstoned_app_key_pubkeys: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_key_epoch: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub missing_key_wrap_pubkeys: Vec<String>,
    pub members: Vec<SharedFolderMemberView>,
    pub client_nonce: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedShareRosterCheckpoint {
    pub checkpoint_id: String,
    pub signer_pubkey: String,
    pub content: ShareRosterCheckpointContent,
    pub event_json: String,
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
        missing_key_wrap_pubkeys,
        participant_count: active_share_member_count(folder),
        app_key_count: projection.active_facets.len(),
        members: shared_folder_member_views(folder, &projection),
        shortcut_paths,
    }
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
    shared_folder_participant_profiles_with_projection(folder, &projection)
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

pub fn sign_share_roster_checkpoint(
    signer_keys: &Keys,
    folder: &SharedFolder,
    created_at: i64,
) -> Result<SignedShareRosterCheckpoint, SharingError> {
    let signer_pubkey = signer_keys.public_key().to_hex();
    if !shared_folder_app_key_can_admin(folder, &signer_pubkey) {
        return Err(SharingError::CurrentAppKeyCannotAdminShare);
    }
    let client_nonce = Uuid::new_v4().to_string();
    let content =
        share_roster_checkpoint_content(folder, &signer_pubkey, client_nonce.clone(), created_at);
    let content_json =
        serde_json::to_string(&content).map_err(|error| SharingError::Invite(error.to_string()))?;
    let ts = u64::try_from(created_at).unwrap_or(0);
    let event = EventBuilder::new(
        Kind::from(KIND_SHARE_ROSTER_CHECKPOINT),
        content_json,
        vec![
            Tag::identifier(share_roster_checkpoint_d_tag(
                folder.share_id,
                &client_nonce,
            )),
            Tag::custom(iris_profile_tag_kind(), [folder.share_id.to_string()]),
            Tag::public_key(signer_keys.public_key()),
        ],
    )
    .custom_created_at(nostr_sdk::Timestamp::from(ts))
    .to_event(signer_keys)
    .map_err(|error| SharingError::RosterCheckpoint(error.to_string()))?;
    parse_share_roster_checkpoint_event(&event)
}

pub fn parse_share_roster_checkpoint_event(
    event: &Event,
) -> Result<SignedShareRosterCheckpoint, SharingError> {
    let kind = event.kind.as_u16();
    if kind != KIND_SHARE_ROSTER_CHECKPOINT {
        return Err(SharingError::RosterCheckpoint(format!(
            "invalid kind: expected {KIND_SHARE_ROSTER_CHECKPOINT}, got {kind}"
        )));
    }
    let d_tag = event
        .identifier()
        .ok_or_else(|| SharingError::RosterCheckpoint("missing d tag".to_string()))?;
    let (d_tag_share_id, d_tag_nonce) = parse_share_roster_checkpoint_d_tag(d_tag)?;
    event
        .verify()
        .map_err(|error| SharingError::RosterCheckpoint(error.to_string()))?;
    let content: ShareRosterCheckpointContent = serde_json::from_str(&event.content)
        .map_err(|error| SharingError::RosterCheckpoint(error.to_string()))?;
    if content.schema != SHARE_ROSTER_CHECKPOINT_SCHEMA {
        return Err(SharingError::RosterCheckpoint(format!(
            "unsupported schema {}",
            content.schema
        )));
    }
    if content.share_id != d_tag_share_id {
        return Err(SharingError::RosterCheckpoint(format!(
            "d-tag share {} does not match content share {}",
            d_tag_share_id, content.share_id
        )));
    }
    if content.client_nonce != d_tag_nonce {
        return Err(SharingError::RosterCheckpoint(format!(
            "d-tag nonce {} does not match content nonce {}",
            d_tag_nonce, content.client_nonce
        )));
    }
    let event_created_at = i64::try_from(event.created_at.as_u64()).unwrap_or(i64::MAX);
    if content.created_at != event_created_at {
        return Err(SharingError::RosterCheckpoint(format!(
            "event created_at {} does not match content created_at {}",
            event_created_at, content.created_at
        )));
    }
    let signer_pubkey = event.pubkey.to_hex();
    if signer_pubkey != content.signer_pubkey {
        return Err(SharingError::RosterCheckpoint(format!(
            "event signer {} does not match checkpoint signer {}",
            signer_pubkey, content.signer_pubkey
        )));
    }
    PublicKey::from_hex(&content.signer_pubkey)
        .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
    Ok(SignedShareRosterCheckpoint {
        checkpoint_id: event.id.to_hex(),
        signer_pubkey,
        content,
        event_json: event.as_json(),
    })
}

pub fn validate_share_roster_checkpoint(
    folder: &SharedFolder,
    checkpoint: &SignedShareRosterCheckpoint,
) -> Result<(), SharingError> {
    validate_shared_folder_member_roster_ops(folder)?;
    let event = Event::from_json(&checkpoint.event_json)
        .map_err(|error| SharingError::RosterCheckpoint(error.to_string()))?;
    let parsed = parse_share_roster_checkpoint_event(&event)?;
    if parsed.checkpoint_id != checkpoint.checkpoint_id
        || parsed.signer_pubkey != checkpoint.signer_pubkey
        || parsed.content != checkpoint.content
    {
        return Err(SharingError::RosterCheckpoint(
            "checkpoint event_json does not match checkpoint fields".to_string(),
        ));
    }
    if !shared_folder_app_key_can_admin(folder, &checkpoint.signer_pubkey) {
        return Err(SharingError::CurrentAppKeyCannotAdminShare);
    }
    let expected = share_roster_checkpoint_content(
        folder,
        &checkpoint.signer_pubkey,
        checkpoint.content.client_nonce.clone(),
        checkpoint.content.created_at,
    );
    if checkpoint.content != expected {
        return Err(SharingError::RosterCheckpoint(
            "checkpoint does not match share roster projection".to_string(),
        ));
    }
    Ok(())
}

fn validate_shared_folder_member_roster_ops(folder: &SharedFolder) -> Result<(), SharingError> {
    for op in &folder.member_ops {
        validate_signed_share_member_roster_op(op)?;
    }
    Ok(())
}

fn validate_signed_share_member_roster_op(
    signed: &SignedShareMemberRosterOp,
) -> Result<(), SharingError> {
    let event = Event::from_json(&signed.event_json)
        .map_err(|error| SharingError::ShareMemberRoster(error.to_string()))?;
    let parsed = parse_share_member_roster_op_event(&event)?;
    if parsed.op_id != signed.op_id
        || parsed.signer_pubkey != signed.signer_pubkey
        || parsed.content != signed.content
    {
        return Err(SharingError::ShareMemberRoster(
            "member roster op event_json does not match op fields".to_string(),
        ));
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

fn shared_folder_app_key_write_authorization_with_projection(
    folder: &SharedFolder,
    projection: &IrisProfileRosterProjection,
    app_pubkey: &str,
) -> ShareRootWriteAuthorization {
    let Some(profile_id) =
        shared_folder_profile_id_for_app_key_with_projection(folder, projection, app_pubkey)
    else {
        return ShareRootWriteAuthorization::UnknownAppKey;
    };
    let members = shared_folder_members_with_projection(folder, projection);
    let Some(member) = members.get(&profile_id.to_string()) else {
        return ShareRootWriteAuthorization::UnknownMember;
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
    active_member_for_app_key_with_projection(folder, projection, app_pubkey)
        .is_some_and(|member| member.role == ShareRole::Admin)
        && projection.can_admin_profile(app_pubkey)
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

fn share_roster_checkpoint_content(
    folder: &SharedFolder,
    signer_pubkey: &str,
    client_nonce: String,
    created_at: i64,
) -> ShareRosterCheckpointContent {
    let projection = folder.projection();
    let member_projection =
        project_shared_folder_member_roster_with_projection(folder, &projection);
    let current_key_epoch = projection.key_epochs.keys().next_back().copied();
    let missing_key_wrap_pubkeys = current_key_epoch.map_or_else(Vec::new, |epoch| {
        active_share_key_recipients_missing_wraps(folder, &projection, epoch)
    });
    ShareRosterCheckpointContent {
        schema: SHARE_ROSTER_CHECKPOINT_SCHEMA,
        share_id: folder.share_id,
        signer_pubkey: signer_pubkey.to_string(),
        roster_head_op_ids: share_roster_head_op_ids(folder.share_id, &folder.roster_ops),
        member_roster_head_op_ids: share_member_roster_head_op_ids(
            folder.share_id,
            &folder.member_ops,
            &member_projection,
        ),
        accepted_op_count: projection.accepted_op_ids.len(),
        rejected_op_count: projection.rejected_op_ids.len(),
        accepted_member_op_count: member_projection.accepted_op_ids.len(),
        rejected_member_op_count: member_projection.rejected_op_ids.len(),
        active_app_key_pubkeys: active_share_app_key_pubkeys(folder, &projection),
        tombstoned_app_key_pubkeys: tombstoned_share_app_key_pubkeys(folder, &projection),
        current_key_epoch,
        missing_key_wrap_pubkeys,
        members: shared_folder_member_views(folder, &projection),
        client_nonce,
        created_at,
    }
}

fn active_share_app_key_pubkeys(
    folder: &SharedFolder,
    projection: &IrisProfileRosterProjection,
) -> Vec<String> {
    projection
        .active_facets
        .values()
        .filter(|facet| facet.is_app_key())
        .filter(|facet| {
            active_member_for_app_key_with_projection(folder, projection, &facet.pubkey).is_some()
        })
        .map(|facet| facet.pubkey.clone())
        .collect()
}

fn tombstoned_share_app_key_pubkeys(
    folder: &SharedFolder,
    projection: &IrisProfileRosterProjection,
) -> Vec<String> {
    projection
        .tombstones
        .keys()
        .filter(|pubkey| {
            shared_folder_profile_id_for_app_key_with_projection(folder, projection, pubkey)
                .is_some()
        })
        .cloned()
        .collect()
}

fn share_roster_head_op_ids(
    share_id: IrisProfileId,
    ops: &[SignedIrisProfileRosterOp],
) -> Vec<String> {
    let projection = project_iris_profile_roster(share_id, ops.to_vec());
    let accepted = projection
        .accepted_op_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let parented = ops
        .iter()
        .filter(|op| accepted.contains(&op.op_id))
        .flat_map(|op| op.content.parents.iter().cloned())
        .filter(|op_id| accepted.contains(op_id))
        .collect::<BTreeSet<_>>();
    accepted.difference(&parented).cloned().collect()
}

fn share_member_roster_head_op_ids(
    share_id: IrisProfileId,
    ops: &[SignedShareMemberRosterOp],
    projection: &ShareMemberRosterProjection,
) -> Vec<String> {
    let accepted = projection
        .accepted_op_ids
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let parented = ops
        .iter()
        .filter(|op| op.content.share_id == share_id && accepted.contains(&op.op_id))
        .flat_map(|op| op.content.parents.iter().cloned())
        .filter(|op_id| accepted.contains(op_id))
        .collect::<BTreeSet<_>>();
    accepted.difference(&parented).cloned().collect()
}

fn share_roster_checkpoint_d_tag(share_id: IrisProfileId, client_nonce: &str) -> String {
    format!("iris-drive/share/{share_id}/roster-checkpoint/{client_nonce}")
}

fn parse_share_roster_checkpoint_d_tag(
    d_tag: &str,
) -> Result<(IrisProfileId, String), SharingError> {
    let rest = d_tag
        .strip_prefix("iris-drive/share/")
        .ok_or_else(|| SharingError::RosterCheckpoint(format!("missing prefix: {d_tag}")))?;
    let (share_id, nonce) = rest
        .split_once("/roster-checkpoint/")
        .ok_or_else(|| SharingError::RosterCheckpoint(format!("missing checkpoint: {d_tag}")))?;
    if nonce.is_empty() || nonce.contains('/') {
        return Err(SharingError::RosterCheckpoint(format!(
            "invalid nonce: {d_tag}"
        )));
    }
    let share_id = share_id
        .parse::<IrisProfileId>()
        .map_err(|error| SharingError::RosterCheckpoint(format!("invalid share UUID: {error}")))?;
    Ok((share_id, nonce.to_string()))
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
) -> Vec<SharedFolderMemberView> {
    let mut app_key_counts = BTreeMap::<IrisProfileId, usize>::new();
    for profile_id in
        shared_folder_participant_profiles_with_projection(folder, projection).values()
    {
        *app_key_counts.entry(*profile_id).or_default() += 1;
    }
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

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(value: &usize) -> bool {
    *value == 0
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
    let participants = collect_share_participants(
        &owner_pubkey,
        owner_profile_id,
        local_label.clone(),
        recipients,
    )?;
    let InitialShareRoster {
        mut roster_ops,
        member_ops,
        members,
        next_roster_op_time,
    } = build_initial_share_rosters(
        owner_keys,
        share_id,
        owner_profile_id,
        local_label,
        &participants,
        created_at,
    )?;

    let share_key = generate_share_key();
    let projection = project_iris_profile_roster(share_id, roster_ops.clone());
    let recipients = active_initial_share_key_recipients(&projection, &members);
    let wrapped_dck = wrap_share_key(owner_keys, recipients, &share_key)?;
    roster_ops.push(sign_share_roster_op_with_parents(
        owner_keys,
        share_id,
        iris_profile_roster_parent_ids(&roster_ops),
        IrisProfileRosterOp::RotateKeyEpoch {
            epoch: 1,
            wrapped_dck,
        },
        next_roster_op_time,
    )?);

    Ok(SharedFolder {
        share_id,
        owner_profile_id,
        source_path,
        display_name,
        local_role: ShareRole::Admin,
        members,
        member_ops,
        participant_profiles: participants.participant_profiles,
        app_key_roots: BTreeMap::new(),
        roster_ops,
    })
}

struct InitialShareRoster {
    roster_ops: Vec<SignedIrisProfileRosterOp>,
    member_ops: Vec<SignedShareMemberRosterOp>,
    members: BTreeMap<String, ShareMember>,
    next_roster_op_time: i64,
}

struct ShareParticipants {
    participant_profiles: BTreeMap<String, IrisProfileId>,
    members: BTreeMap<String, ShareMember>,
    recipients_by_app_key: BTreeMap<String, ShareRecipient>,
}

fn build_initial_share_rosters(
    owner_keys: &nostr_sdk::Keys,
    share_id: IrisProfileId,
    owner_profile_id: IrisProfileId,
    local_label: Option<String>,
    participants: &ShareParticipants,
    created_at: i64,
) -> Result<InitialShareRoster, SharingError> {
    let context = InitialShareRosterContext {
        owner_keys,
        share_id,
        owner_profile_id,
        participants,
    };
    let mut roster_ops = vec![sign_share_roster_op(
        owner_keys,
        share_id,
        IrisProfileRosterOp::AddFacet {
            facet: IrisProfileFacet::app_key(
                owner_keys.public_key().to_hex(),
                created_at,
                local_label,
                ShareRole::Admin.capabilities(),
            )
            .with_profile_id(owner_profile_id),
        },
        created_at,
    )?];
    let mut member_ops = vec![sign_share_member_roster_op(
        owner_keys,
        share_id,
        ShareMemberRosterOp::GrantMember {
            member: participants
                .members
                .get(&owner_profile_id.to_string())
                .cloned()
                .unwrap_or_else(|| {
                    ShareMember::active(owner_profile_id, ShareRole::Admin, None, None)
                }),
        },
        created_at,
    )?];
    let mut next_roster_op_time = created_at;
    let mut next_member_op_time = created_at;
    for recipient in participants.recipients_by_app_key.values() {
        next_roster_op_time += 1;
        push_initial_share_recipient_facet(
            owner_keys,
            share_id,
            &participants.members,
            &mut roster_ops,
            recipient,
            next_roster_op_time,
        )?;
        if !member_ops_grant_profile(&member_ops, recipient.profile_id) {
            next_member_op_time += 1;
            push_initial_share_member_grant(
                &context,
                &roster_ops,
                &mut member_ops,
                recipient,
                next_member_op_time,
            )?;
        }
    }
    let members = project_shared_folder_member_roster_from_parts(
        share_id,
        owner_profile_id,
        &participants.participant_profiles,
        &roster_ops,
        &member_ops,
    )
    .members;
    Ok(InitialShareRoster {
        roster_ops,
        member_ops,
        members,
        next_roster_op_time: next_roster_op_time.saturating_add(1),
    })
}

struct InitialShareRosterContext<'a> {
    owner_keys: &'a nostr_sdk::Keys,
    share_id: IrisProfileId,
    owner_profile_id: IrisProfileId,
    participants: &'a ShareParticipants,
}

fn push_initial_share_recipient_facet(
    owner_keys: &nostr_sdk::Keys,
    share_id: IrisProfileId,
    members: &BTreeMap<String, ShareMember>,
    roster_ops: &mut Vec<SignedIrisProfileRosterOp>,
    recipient: &ShareRecipient,
    created_at: i64,
) -> Result<(), SharingError> {
    let member_role = members
        .get(&recipient.profile_id.to_string())
        .map_or(recipient.role, |member| member.role);
    roster_ops.push(sign_share_roster_op_with_parents(
        owner_keys,
        share_id,
        iris_profile_roster_parent_ids(roster_ops),
        IrisProfileRosterOp::AddFacet {
            facet: IrisProfileFacet::app_key(
                recipient.app_pubkey.clone(),
                created_at,
                recipient.label.clone(),
                member_role.capabilities(),
            )
            .with_profile_id(recipient.profile_id),
        },
        created_at,
    )?);
    Ok(())
}

fn push_initial_share_member_grant(
    context: &InitialShareRosterContext<'_>,
    roster_ops: &[SignedIrisProfileRosterOp],
    member_ops: &mut Vec<SignedShareMemberRosterOp>,
    recipient: &ShareRecipient,
    created_at: i64,
) -> Result<(), SharingError> {
    let member_key = recipient.profile_id.to_string();
    let member_role = context
        .participants
        .members
        .get(&member_key)
        .map_or(recipient.role, |member| member.role);
    member_ops.push(sign_share_member_roster_op_with_parents(
        context.owner_keys,
        context.share_id,
        share_member_roster_parent_ids_for_parts(
            context.share_id,
            context.owner_profile_id,
            &context.participants.participant_profiles,
            roster_ops,
            member_ops,
        ),
        ShareMemberRosterOp::GrantMember {
            member: context
                .participants
                .members
                .get(&member_key)
                .cloned()
                .unwrap_or_else(|| {
                    ShareMember::active(
                        recipient.profile_id,
                        member_role,
                        recipient.representative_npub_hint.clone(),
                        recipient.display_name.clone(),
                    )
                }),
        },
        created_at,
    )?);
    Ok(())
}

fn member_ops_grant_profile(
    member_ops: &[SignedShareMemberRosterOp],
    profile_id: IrisProfileId,
) -> bool {
    member_ops.iter().any(|op| {
        matches!(
            &op.content.op,
            ShareMemberRosterOp::GrantMember { member } if member.profile_id == profile_id
        )
    })
}

fn active_initial_share_key_recipients<'a>(
    projection: &'a IrisProfileRosterProjection,
    members: &BTreeMap<String, ShareMember>,
) -> BTreeSet<&'a str> {
    projection
        .active_facets
        .values()
        .filter(|facet| facet.capabilities.can_receive_key_wraps)
        .filter(|facet| {
            facet.profile_id.is_some_and(|profile_id| {
                members
                    .get(&profile_id.to_string())
                    .is_some_and(ShareMember::is_active)
            })
        })
        .map(|facet| facet.pubkey.as_str())
        .collect()
}

fn collect_share_participants(
    owner_pubkey: &str,
    owner_profile_id: IrisProfileId,
    local_label: Option<String>,
    recipients: Vec<ShareRecipient>,
) -> Result<ShareParticipants, SharingError> {
    let mut participant_profiles = BTreeMap::from([(owner_pubkey.to_string(), owner_profile_id)]);
    let mut members = BTreeMap::from([(
        owner_profile_id.to_string(),
        ShareMember::active(owner_profile_id, ShareRole::Admin, None, local_label),
    )]);
    let mut recipients_by_app_key = BTreeMap::<String, ShareRecipient>::new();

    for recipient in recipients {
        PublicKey::from_hex(&recipient.app_pubkey)
            .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
        participant_profiles.insert(recipient.app_pubkey.clone(), recipient.profile_id);
        upsert_share_member(&mut members, &recipient);
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
        participant_profiles,
        members,
        recipients_by_app_key,
    })
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
        let op_time = created_at.saturating_add(i64::try_from(index).unwrap_or(i64::MAX));
        invited = Some(add_invited_share_member(
            folder,
            signer_keys,
            recipient,
            op_time,
        )?);
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
    let share_key = generate_share_key();
    let wrapped_dck = wrap_share_key(
        signer_keys,
        active_share_key_recipients(folder, &projection)
            .iter()
            .map(String::as_str),
        &share_key,
    )?;
    folder.roster_ops.push(sign_share_roster_op_with_parents(
        signer_keys,
        folder.share_id,
        iris_profile_roster_parent_ids(&folder.roster_ops),
        IrisProfileRosterOp::RotateKeyEpoch {
            epoch: next_epoch,
            wrapped_dck,
        },
        created_at.saturating_add(op_offset),
    )?);
    let bundle = ShareInviteBundle {
        schema: SHARE_INVITE_SCHEMA,
        shared_folder: folder.clone(),
        recipient_profile_id: invited.profile_id,
        role: invited.role,
        representative_npub_hint: invited.representative_npub_hint,
        roster_checkpoint: Some(sign_share_roster_checkpoint(
            signer_keys,
            folder,
            created_at.saturating_add(op_offset).saturating_add(1),
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
    signer_keys: &Keys,
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
    let member_op_time = next_share_member_op_time(folder, created_at);
    folder
        .member_ops
        .push(sign_share_member_roster_op_with_parents(
            signer_keys,
            folder.share_id,
            share_member_roster_parent_ids(folder),
            ShareMemberRosterOp::GrantMember {
                member: member.clone(),
            },
            member_op_time,
        )?);
    folder
        .participant_profiles
        .insert(recipient.app_pubkey.clone(), recipient.profile_id);
    let member_role = folder
        .member_projection()
        .members
        .get(&recipient.profile_id.to_string())
        .map_or(recipient.role, |member| member.role);
    folder.roster_ops.push(sign_share_roster_op_with_parents(
        signer_keys,
        folder.share_id,
        iris_profile_roster_parent_ids(&folder.roster_ops),
        IrisProfileRosterOp::AddFacet {
            facet: IrisProfileFacet::app_key(
                recipient.app_pubkey,
                created_at,
                recipient.label,
                member_role.capabilities(),
            )
            .with_profile_id(recipient.profile_id),
        },
        created_at,
    )?);
    materialize_share_members_from_ops(folder);
    Ok(InvitedShareMember {
        profile_id: recipient.profile_id,
        role: member_role,
        representative_npub_hint: recipient.representative_npub_hint,
    })
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
    validate_shared_folder_member_roster_ops(&bundle.shared_folder)?;
    if let Some(checkpoint) = &bundle.roster_checkpoint {
        validate_share_roster_checkpoint(&bundle.shared_folder, checkpoint)?;
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
    let repair_op = sign_share_roster_op_with_parents(
        signer_keys,
        folder.share_id,
        iris_profile_roster_parent_ids(&folder.roster_ops),
        IrisProfileRosterOp::RepairKeyWraps { epoch, wrapped_dck },
        created_at,
    )?;
    folder.roster_ops.push(repair_op);
    Ok(ShareKeyRepairOutcome {
        share_id: folder.share_id,
        epoch,
        repaired_pubkeys: missing_pubkeys,
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
        shared_folder_profile_id_for_app_key_with_projection(folder, &projection, &signer_pubkey)
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
    let member_op_time = next_share_member_op_time(folder, created_at);
    folder
        .member_ops
        .push(sign_share_member_roster_op_with_parents(
            signer_keys,
            folder.share_id,
            share_member_roster_parent_ids(folder),
            ShareMemberRosterOp::RevokeMember {
                profile_id,
                reason: reason.map(str::to_string),
            },
            member_op_time,
        )?);
    materialize_share_members_from_ops(folder);

    let mut revoked_app_pubkeys =
        shared_folder_participant_profiles_with_projection(folder, &projection)
            .into_iter()
            .filter(|(_, member_profile_id)| *member_profile_id == profile_id)
            .map(|(app_pubkey, _)| app_pubkey)
            .collect::<Vec<_>>();
    revoked_app_pubkeys.sort();

    let mut op_time = created_at;
    for app_pubkey in &revoked_app_pubkeys {
        folder.roster_ops.push(sign_share_roster_op_with_parents(
            signer_keys,
            folder.share_id,
            iris_profile_roster_parent_ids(&folder.roster_ops),
            IrisProfileRosterOp::TombstoneFacet {
                pubkey: app_pubkey.clone(),
                reason: reason.map(str::to_string),
            },
            op_time,
        )?);
        op_time += 1;
    }

    let current_projection = folder.projection();
    let next_epoch = current_projection
        .key_epochs
        .keys()
        .next_back()
        .map_or(1, |epoch| epoch.saturating_add(1));
    let share_key = generate_share_key();
    let wrapped_dck = wrap_share_key(
        signer_keys,
        active_share_key_recipients(folder, &current_projection)
            .iter()
            .map(String::as_str),
        &share_key,
    )?;
    folder.roster_ops.push(sign_share_roster_op_with_parents(
        signer_keys,
        folder.share_id,
        iris_profile_roster_parent_ids(&folder.roster_ops),
        IrisProfileRosterOp::RotateKeyEpoch {
            epoch: next_epoch,
            wrapped_dck,
        },
        op_time,
    )?);

    Ok(ShareMemberRevokeOutcome {
        share_id: folder.share_id,
        profile_id,
        epoch: next_epoch,
        revoked_app_pubkeys,
    })
}

pub fn refresh_shared_folder_member_statuses_from_roster(folder: &mut SharedFolder) {
    let projection = folder.projection();
    let mut app_keys_by_profile = BTreeMap::<IrisProfileId, Vec<String>>::new();
    for (app_pubkey, profile_id) in
        shared_folder_participant_profiles_with_projection(folder, &projection)
    {
        app_keys_by_profile
            .entry(profile_id)
            .or_default()
            .push(app_pubkey);
    }
    for (profile_id, app_pubkeys) in app_keys_by_profile {
        if app_pubkeys.is_empty() {
            continue;
        }
        if app_pubkeys
            .iter()
            .all(|app_pubkey| projection.tombstones.contains_key(app_pubkey))
            && let Some(member) = folder.members.get_mut(&profile_id.to_string())
        {
            member.status = ShareMemberStatus::Revoked;
        }
    }
}

fn sign_share_roster_op(
    signer_keys: &Keys,
    share_id: IrisProfileId,
    op: IrisProfileRosterOp,
    created_at: i64,
) -> Result<SignedIrisProfileRosterOp, SharingError> {
    sign_share_roster_op_with_parents(signer_keys, share_id, Vec::new(), op, created_at)
}

fn sign_share_roster_op_with_parents(
    signer_keys: &Keys,
    share_id: IrisProfileId,
    parents: Vec<String>,
    op: IrisProfileRosterOp,
    created_at: i64,
) -> Result<SignedIrisProfileRosterOp, SharingError> {
    let event =
        build_iris_profile_roster_op_event(signer_keys, share_id, parents, None, op, created_at)?;
    parse_iris_profile_roster_op_event(&event).map_err(SharingError::from)
}

fn sign_share_member_roster_op(
    signer_keys: &Keys,
    share_id: IrisProfileId,
    op: ShareMemberRosterOp,
    created_at: i64,
) -> Result<SignedShareMemberRosterOp, SharingError> {
    sign_share_member_roster_op_with_parents(signer_keys, share_id, Vec::new(), op, created_at)
}

fn sign_share_member_roster_op_with_parents(
    signer_keys: &Keys,
    share_id: IrisProfileId,
    parents: Vec<String>,
    op: ShareMemberRosterOp,
    created_at: i64,
) -> Result<SignedShareMemberRosterOp, SharingError> {
    let event = build_share_member_roster_op_event(signer_keys, share_id, parents, op, created_at)?;
    parse_share_member_roster_op_event(&event)
}

pub fn build_share_member_roster_op_event(
    signer_keys: &Keys,
    share_id: IrisProfileId,
    parents: Vec<String>,
    op: ShareMemberRosterOp,
    created_at: i64,
) -> Result<Event, SharingError> {
    validate_share_member_roster_op(&op)?;
    let client_nonce = Uuid::new_v4().to_string();
    let content = ShareMemberRosterOpContent {
        schema: SHARE_MEMBER_ROSTER_SCHEMA,
        share_id,
        actor_pubkey: signer_keys.public_key().to_hex(),
        parents,
        client_nonce: client_nonce.clone(),
        created_at,
        op,
    };
    let content_json = serde_json::to_string(&content)
        .map_err(|e| SharingError::ShareMemberRoster(e.to_string()))?;
    let ts = u64::try_from(created_at).unwrap_or(0);
    EventBuilder::new(
        Kind::from(KIND_SHARE_MEMBER_ROSTER_OP),
        content_json,
        vec![
            Tag::identifier(share_member_roster_op_d_tag(share_id, &client_nonce)),
            Tag::custom(iris_profile_tag_kind(), [share_id.to_string()]),
            Tag::public_key(signer_keys.public_key()),
        ],
    )
    .custom_created_at(nostr_sdk::Timestamp::from(ts))
    .to_event(signer_keys)
    .map_err(|e| SharingError::ShareMemberRoster(e.to_string()))
}

pub fn parse_share_member_roster_op_event(
    event: &Event,
) -> Result<SignedShareMemberRosterOp, SharingError> {
    let kind = event.kind.as_u16();
    if kind != KIND_SHARE_MEMBER_ROSTER_OP {
        return Err(SharingError::ShareMemberRoster(format!(
            "invalid kind: expected {KIND_SHARE_MEMBER_ROSTER_OP}, got {kind}"
        )));
    }
    let d_tag = event
        .identifier()
        .ok_or_else(|| SharingError::ShareMemberRoster("missing d tag".to_string()))?;
    let (d_tag_share_id, d_tag_nonce) = parse_share_member_roster_op_d_tag(d_tag)?;
    event
        .verify()
        .map_err(|error| SharingError::ShareMemberRoster(error.to_string()))?;
    let content: ShareMemberRosterOpContent = serde_json::from_str(&event.content)
        .map_err(|error| SharingError::ShareMemberRoster(error.to_string()))?;
    if content.schema != SHARE_MEMBER_ROSTER_SCHEMA {
        return Err(SharingError::ShareMemberRoster(format!(
            "unsupported share member roster schema {}",
            content.schema
        )));
    }
    if content.share_id != d_tag_share_id {
        return Err(SharingError::ShareMemberRoster(format!(
            "d-tag share {} does not match content share {}",
            d_tag_share_id, content.share_id
        )));
    }
    if content.client_nonce != d_tag_nonce {
        return Err(SharingError::ShareMemberRoster(format!(
            "d-tag nonce {} does not match content nonce {}",
            d_tag_nonce, content.client_nonce
        )));
    }
    let event_created_at = i64::try_from(event.created_at.as_u64()).unwrap_or(i64::MAX);
    if content.created_at != event_created_at {
        return Err(SharingError::ShareMemberRoster(format!(
            "event created_at {} does not match content created_at {}",
            event_created_at, content.created_at
        )));
    }
    let signer_pubkey = event.pubkey.to_hex();
    if signer_pubkey != content.actor_pubkey {
        return Err(SharingError::ShareMemberRoster(format!(
            "event signer {} does not match member roster actor {}",
            signer_pubkey, content.actor_pubkey
        )));
    }
    PublicKey::from_hex(&content.actor_pubkey)
        .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
    validate_share_member_roster_op(&content.op)?;
    Ok(SignedShareMemberRosterOp {
        op_id: event.id.to_hex(),
        signer_pubkey,
        content,
        event_json: event.as_json(),
    })
}

fn validate_share_member_roster_op(op: &ShareMemberRosterOp) -> Result<(), SharingError> {
    if let ShareMemberRosterOp::GrantMember { member } = op
        && member.status == ShareMemberStatus::Revoked
    {
        return Err(SharingError::ShareMemberRoster(
            "grant_member cannot grant revoked status".to_string(),
        ));
    }
    Ok(())
}

fn share_member_roster_op_d_tag(share_id: IrisProfileId, client_nonce: &str) -> String {
    format!("iris-drive/share/{share_id}/member-roster-op/{client_nonce}")
}

fn parse_share_member_roster_op_d_tag(
    d_tag: &str,
) -> Result<(IrisProfileId, String), SharingError> {
    let rest = d_tag
        .strip_prefix("iris-drive/share/")
        .ok_or_else(|| SharingError::ShareMemberRoster(format!("missing prefix: {d_tag}")))?;
    let (share_id, nonce) = rest
        .split_once("/member-roster-op/")
        .ok_or_else(|| SharingError::ShareMemberRoster(format!("missing member op: {d_tag}")))?;
    if nonce.is_empty() || nonce.contains('/') {
        return Err(SharingError::ShareMemberRoster(format!(
            "invalid nonce: {d_tag}"
        )));
    }
    let share_id = share_id
        .parse::<IrisProfileId>()
        .map_err(|error| SharingError::ShareMemberRoster(format!("invalid share UUID: {error}")))?;
    Ok((share_id, nonce.to_string()))
}

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
    use nostr_sdk::Keys;

    use super::*;

    fn recipient(role: ShareRole) -> (Keys, ShareRecipient) {
        let keys = Keys::generate();
        (
            keys.clone(),
            ShareRecipient {
                profile_id: IrisProfileId::new_v4(),
                app_pubkey: keys.public_key().to_hex(),
                role,
                label: Some("Phone".to_string()),
                representative_npub_hint: None,
                display_name: Some("Recipient".to_string()),
            },
        )
    }

    #[test]
    fn share_projection_vocabulary_is_core_owned() {
        assert_eq!(ShareRole::Admin.as_str(), "admin");
        assert_eq!(ShareRole::Admin.label(), "Admin");
        assert_eq!(ShareRole::Editor.as_str(), "editor");
        assert_eq!(ShareRole::Editor.label(), "Editor");
        assert_eq!(ShareRole::Reader.as_str(), "reader");
        assert_eq!(ShareRole::Reader.label(), "Reader");

        assert_eq!(
            ShareRole::parse_user_input("writer"),
            Some(ShareRole::Editor)
        );
        assert_eq!(
            ShareRole::parse_user_input(" read "),
            Some(ShareRole::Reader)
        );
        assert_eq!(ShareRole::parse_user_input("owner"), None);

        assert_eq!(ShareMemberStatus::Pending.as_str(), "pending");
        assert_eq!(ShareMemberStatus::Pending.label(), "Pending");
        assert_eq!(ShareMemberStatus::Active.as_str(), "active");
        assert_eq!(ShareMemberStatus::Active.label(), "Active");
        assert_eq!(ShareMemberStatus::Revoked.as_str(), "revoked");
        assert_eq!(ShareMemberStatus::Revoked.label(), "Revoked");

        assert_eq!(
            ShareRootWriteAuthorization::Authorized.as_str(),
            "authorized"
        );
        assert_eq!(
            ShareRootWriteAuthorization::Authorized.label(),
            "Authorized"
        );
        assert!(ShareRootWriteAuthorization::Authorized.is_authorized());
        assert_eq!(
            ShareRootWriteAuthorization::InsufficientShareRole.as_str(),
            "insufficient_share_role"
        );
        assert_eq!(
            ShareRootWriteAuthorization::InsufficientShareRole.label(),
            "Insufficient share role"
        );
        assert!(!ShareRootWriteAuthorization::RevokedMember.is_authorized());
    }

    #[test]
    fn share_root_write_authorization_explains_profile_member_and_facet_state() {
        let owner_keys = Keys::generate();
        let owner_pubkey = owner_keys.public_key().to_hex();
        let owner_profile_id = IrisProfileId::new_v4();
        let reader_keys = Keys::generate();
        let reader_pubkey = reader_keys.public_key().to_hex();
        let reader_profile_id = IrisProfileId::new_v4();
        let folder = create_shared_folder(
            &owner_keys,
            owner_profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Owner".into()),
            vec![ShareRecipient {
                profile_id: reader_profile_id,
                app_pubkey: reader_pubkey.clone(),
                role: ShareRole::Reader,
                label: Some("Reader".into()),
                representative_npub_hint: None,
                display_name: Some("Reader".into()),
            }],
            10,
        )
        .unwrap();

        assert_eq!(
            shared_folder_app_key_write_authorization(&folder, &owner_pubkey),
            ShareRootWriteAuthorization::Authorized
        );
        assert_eq!(
            shared_folder_app_key_write_authorization(&folder, &reader_pubkey),
            ShareRootWriteAuthorization::InsufficientShareRole
        );
        assert_eq!(
            shared_folder_authorized_writer_pubkeys(&folder),
            vec![owner_pubkey.clone()]
        );

        let stranger_pubkey = Keys::generate().public_key().to_hex();
        assert_eq!(
            shared_folder_app_key_write_authorization(&folder, &stranger_pubkey),
            ShareRootWriteAuthorization::UnknownAppKey
        );

        let mut inconsistent = folder.clone();
        inconsistent
            .participant_profiles
            .insert(stranger_pubkey.clone(), IrisProfileId::new_v4());
        assert_eq!(
            shared_folder_app_key_write_authorization(&inconsistent, &stranger_pubkey),
            ShareRootWriteAuthorization::UnknownMember
        );

        let mut pending = folder.clone();
        pending.member_ops.clear();
        pending
            .members
            .get_mut(&reader_profile_id.to_string())
            .unwrap()
            .status = ShareMemberStatus::Pending;
        assert_eq!(
            shared_folder_app_key_write_authorization(&pending, &reader_pubkey),
            ShareRootWriteAuthorization::PendingMember
        );

        let mut revoked = folder.clone();
        revoked.member_ops.clear();
        revoked
            .members
            .get_mut(&reader_profile_id.to_string())
            .unwrap()
            .status = ShareMemberStatus::Revoked;
        assert_eq!(
            shared_folder_app_key_write_authorization(&revoked, &reader_pubkey),
            ShareRootWriteAuthorization::RevokedMember
        );

        let mut promoted_reader = folder.clone();
        promoted_reader.member_ops.clear();
        promoted_reader
            .members
            .get_mut(&reader_profile_id.to_string())
            .unwrap()
            .role = ShareRole::Editor;
        assert_eq!(
            shared_folder_app_key_write_authorization(&promoted_reader, &reader_pubkey),
            ShareRootWriteAuthorization::AppKeyCannotWriteRoots
        );

        let mut missing_facet = promoted_reader.clone();
        missing_facet.roster_ops.clear();
        assert_eq!(
            shared_folder_app_key_write_authorization(&missing_facet, &reader_pubkey),
            ShareRootWriteAuthorization::AppKeyNotActive
        );

        let social_keys = Keys::generate();
        let social_pubkey = social_keys.public_key().to_hex();
        let social_profile_id = IrisProfileId::new_v4();
        let mut social_folder = create_shared_folder(
            &owner_keys,
            owner_profile_id,
            "Projects/Social",
            "Social",
            Some("Owner".into()),
            Vec::new(),
            20,
        )
        .unwrap();
        append_profile_facet(
            &mut social_folder.roster_ops,
            &owner_keys,
            social_folder.share_id,
            IrisProfileFacet::social_profile(social_pubkey.clone(), 21, None),
            21,
        );
        social_folder.members.insert(
            social_profile_id.to_string(),
            ShareMember::active(social_profile_id, ShareRole::Editor, None, None),
        );
        social_folder
            .participant_profiles
            .insert(social_pubkey.clone(), social_profile_id);
        assert!(
            !shared_folder_app_key_write_authorization(&social_folder, &social_pubkey)
                .is_authorized()
        );
    }

    fn folder_with_recipient_missing_wrap() -> (SharedFolder, String, String) {
        let owner_keys = Keys::generate();
        let recipient_keys = Keys::generate();
        let owner_pubkey = owner_keys.public_key().to_hex();
        let recipient_pubkey = recipient_keys.public_key().to_hex();
        let owner_profile_id = IrisProfileId::new_v4();
        let recipient_profile_id = IrisProfileId::new_v4();
        let share_id = IrisProfileId::new_v4();
        let mut wrapped_dck = BTreeMap::new();
        wrapped_dck.insert(owner_pubkey.clone(), "owner-wrap".to_string());
        let mut roster_ops = vec![
            sign_share_roster_op(
                &owner_keys,
                share_id,
                IrisProfileRosterOp::AddFacet {
                    facet: IrisProfileFacet::app_key(
                        owner_pubkey.clone(),
                        10,
                        Some("Desktop".to_string()),
                        ShareRole::Admin.capabilities(),
                    ),
                },
                10,
            )
            .unwrap(),
        ];
        roster_ops.push(
            sign_share_roster_op_with_parents(
                &owner_keys,
                share_id,
                iris_profile_roster_parent_ids(&roster_ops),
                IrisProfileRosterOp::AddFacet {
                    facet: IrisProfileFacet::app_key(
                        recipient_pubkey.clone(),
                        11,
                        Some("Phone".to_string()),
                        ShareRole::Editor.capabilities(),
                    ),
                },
                11,
            )
            .unwrap(),
        );
        roster_ops.push(
            sign_share_roster_op_with_parents(
                &owner_keys,
                share_id,
                iris_profile_roster_parent_ids(&roster_ops),
                IrisProfileRosterOp::RotateKeyEpoch {
                    epoch: 1,
                    wrapped_dck,
                },
                12,
            )
            .unwrap(),
        );
        let folder = SharedFolder {
            share_id,
            owner_profile_id: IrisProfileId::new_v4(),
            source_path: "Projects/Alpha".to_string(),
            display_name: "Alpha".to_string(),
            local_role: ShareRole::Admin,
            members: BTreeMap::from([
                (
                    owner_profile_id.to_string(),
                    ShareMember::active(
                        owner_profile_id,
                        ShareRole::Admin,
                        None,
                        Some("Desktop".to_string()),
                    ),
                ),
                (
                    recipient_profile_id.to_string(),
                    ShareMember::active(
                        recipient_profile_id,
                        ShareRole::Editor,
                        None,
                        Some("Phone".to_string()),
                    ),
                ),
            ]),
            member_ops: Vec::new(),
            participant_profiles: BTreeMap::from([
                (owner_pubkey.clone(), owner_profile_id),
                (recipient_pubkey.clone(), recipient_profile_id),
            ]),
            app_key_roots: BTreeMap::new(),
            roster_ops,
        };

        (folder, owner_pubkey, recipient_pubkey)
    }

    fn facet_acceptance(
        keys: &Keys,
        profile_id: IrisProfileId,
        purpose: IrisProfileKeyPurpose,
        roster_op_id: Option<String>,
        accepted_at: i64,
    ) -> SignedIrisProfileFacetAcceptance {
        let event = crate::build_iris_profile_facet_acceptance_event(
            keys,
            profile_id,
            [purpose],
            roster_op_id,
            accepted_at,
        )
        .unwrap();
        crate::parse_iris_profile_facet_acceptance_event(&event).unwrap()
    }

    fn append_profile_roster_op(
        ops: &mut Vec<SignedIrisProfileRosterOp>,
        signer: &Keys,
        profile_id: IrisProfileId,
        op: IrisProfileRosterOp,
        created_at: i64,
    ) -> SignedIrisProfileRosterOp {
        let entry = if ops.is_empty() {
            sign_share_roster_op(signer, profile_id, op, created_at).unwrap()
        } else {
            sign_share_roster_op_with_parents(
                signer,
                profile_id,
                iris_profile_roster_parent_ids(ops),
                op,
                created_at,
            )
            .unwrap()
        };
        ops.push(entry.clone());
        entry
    }

    fn append_profile_facet(
        ops: &mut Vec<SignedIrisProfileRosterOp>,
        signer: &Keys,
        profile_id: IrisProfileId,
        facet: IrisProfileFacet,
        created_at: i64,
    ) -> SignedIrisProfileRosterOp {
        append_profile_roster_op(
            ops,
            signer,
            profile_id,
            IrisProfileRosterOp::AddFacet { facet },
            created_at,
        )
    }

    fn append_profile_tombstone(
        ops: &mut Vec<SignedIrisProfileRosterOp>,
        signer: &Keys,
        profile_id: IrisProfileId,
        pubkey: String,
        created_at: i64,
    ) {
        append_profile_roster_op(
            ops,
            signer,
            profile_id,
            IrisProfileRosterOp::TombstoneFacet {
                pubkey,
                reason: Some("old install".to_string()),
            },
            created_at,
        );
    }

    fn append_share_member_grant(
        folder: &mut SharedFolder,
        signer: &Keys,
        member: ShareMember,
        created_at: i64,
    ) {
        folder.member_ops.push(
            sign_share_member_roster_op_with_parents(
                signer,
                folder.share_id,
                share_member_roster_parent_ids(folder),
                ShareMemberRosterOp::GrantMember { member },
                created_at,
            )
            .unwrap(),
        );
        materialize_share_members_from_ops(folder);
    }

    struct ShareRecipientResolutionEvidence {
        profile_id: IrisProfileId,
        social_pubkey: String,
        phone_pubkey: String,
        ops: Vec<SignedIrisProfileRosterOp>,
        acceptances: Vec<SignedIrisProfileFacetAcceptance>,
    }

    fn accepted_share_recipient_resolution_evidence() -> ShareRecipientResolutionEvidence {
        let profile_id = IrisProfileId::new_v4();
        let admin = Keys::generate();
        let social = Keys::generate();
        let phone = Keys::generate();
        let laptop = Keys::generate();
        let social_pubkey = social.public_key().to_hex();
        let phone_pubkey = phone.public_key().to_hex();
        let laptop_pubkey = laptop.public_key().to_hex();
        let mut ops = Vec::new();
        append_profile_facet(
            &mut ops,
            &admin,
            profile_id,
            IrisProfileFacet::app_key(
                admin.public_key().to_hex(),
                10,
                Some("Admin".to_string()),
                IrisProfileCapabilities::app_admin(),
            ),
            10,
        );
        let social_op = append_profile_facet(
            &mut ops,
            &admin,
            profile_id,
            IrisProfileFacet::social_profile(social_pubkey.clone(), 11, Some("Alice".to_string())),
            11,
        );
        let phone_op = append_profile_facet(
            &mut ops,
            &admin,
            profile_id,
            IrisProfileFacet::app_key(
                phone_pubkey.clone(),
                12,
                Some("Phone".to_string()),
                IrisProfileCapabilities::app_reader(),
            ),
            12,
        );
        let laptop_op = append_profile_facet(
            &mut ops,
            &admin,
            profile_id,
            IrisProfileFacet::app_key(
                laptop_pubkey.clone(),
                13,
                Some("Laptop".to_string()),
                IrisProfileCapabilities::app_reader(),
            ),
            13,
        );
        append_profile_tombstone(&mut ops, &admin, profile_id, laptop_pubkey, 15);
        let acceptances = vec![
            facet_acceptance(
                &social,
                profile_id,
                IrisProfileKeyPurpose::SocialProfile,
                Some(social_op.op_id),
                20,
            ),
            facet_acceptance(
                &phone,
                profile_id,
                IrisProfileKeyPurpose::AppKey,
                Some(phone_op.op_id),
                21,
            ),
            facet_acceptance(
                &laptop,
                profile_id,
                IrisProfileKeyPurpose::AppKey,
                Some(laptop_op.op_id),
                22,
            ),
        ];
        ShareRecipientResolutionEvidence {
            profile_id,
            social_pubkey,
            phone_pubkey,
            ops,
            acceptances,
        }
    }

    #[test]
    fn shared_folder_has_own_roster_epoch_and_shortcut() {
        let owner_keys = Keys::generate();
        let owner_profile_id = IrisProfileId::new_v4();
        let (recipient_keys, recipient) = recipient(ShareRole::Editor);

        let folder = create_shared_folder(
            &owner_keys,
            owner_profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Desktop".to_string()),
            vec![recipient],
            10,
        )
        .unwrap();

        assert_eq!(folder.owner_profile_id, owner_profile_id);
        assert_eq!(folder.source_path, "Projects/Alpha");
        assert_eq!(folder.display_name, "Alpha");

        let projection = folder.projection();
        let owner_pubkey = owner_keys.public_key().to_hex();
        let recipient_pubkey = recipient_keys.public_key().to_hex();
        assert!(projection.can_admin_profile(&owner_pubkey));
        assert!(projection.can_write_roots(&owner_pubkey));
        assert!(projection.can_write_roots(&recipient_pubkey));
        assert!(!projection.can_admin_profile(&recipient_pubkey));
        let epoch = projection.key_epochs.values().next_back().unwrap();
        assert_eq!(epoch.epoch, 1);
        assert!(epoch.wrapped_dck.contains_key(&owner_pubkey));
        assert!(epoch.wrapped_dck.contains_key(&recipient_pubkey));

        let shortcut = ShareShortcut::new(folder.share_id, "Shared/Alpha", "").unwrap();
        assert_eq!(shortcut.path, "Shared/Alpha");
        assert_eq!(shortcut.target_path, "");
    }

    #[test]
    fn shared_folder_view_surfaces_shared_with_me_and_shortcuts() {
        let owner_keys = Keys::generate();
        let owner_profile_id = IrisProfileId::new_v4();
        let (recipient_keys, recipient) = recipient(ShareRole::Editor);
        let folder = create_shared_folder(
            &owner_keys,
            owner_profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Desktop".to_string()),
            vec![recipient],
            10,
        )
        .unwrap();
        let shortcut = ShareShortcut::new(folder.share_id, "Projects/Alpha shared", "").unwrap();

        let views = shared_folder_views(
            std::slice::from_ref(&folder),
            std::slice::from_ref(&shortcut),
            &recipient_keys.public_key().to_hex(),
        );

        assert_eq!(views.len(), 1);
        let view = &views[0];
        assert_eq!(view.share_id, folder.share_id);
        assert_eq!(view.shared_with_me_path, "Shared with me/Alpha");
        assert_eq!(view.shortcut_paths, vec!["Projects/Alpha shared"]);
        assert_eq!(view.local_role, ShareRole::Editor);
        assert_eq!(
            view.write_authorization,
            ShareRootWriteAuthorization::Authorized
        );
        assert!(view.can_write);
        assert!(!view.can_admin);
        assert_eq!(view.key_status, SharedFolderKeyStatus::Available);
        assert_eq!(view.current_key_epoch, Some(1));
        assert!(view.has_current_key_wrap);
        assert!(!view.key_unavailable);
        assert!(!view.repair_needed);
        assert!(view.missing_key_wrap_pubkeys.is_empty());
        assert_eq!(view.participant_count, 2);
        assert_eq!(view.app_key_count, 2);
        assert_eq!(view.members.len(), 2);
        assert!(view.members.iter().any(|member| {
            member.profile_id == owner_profile_id
                && member.role == ShareRole::Admin
                && member.status == ShareMemberStatus::Active
                && member.display_name == "Desktop"
                && member.app_key_count == 1
        }));
        assert!(view.members.iter().any(|member| {
            member.profile_id
                == folder
                    .participant_profiles
                    .get(&recipient_keys.public_key().to_hex())
                    .copied()
                    .unwrap()
                && member.role == ShareRole::Editor
                && member.status == ShareMemberStatus::Active
                && member.display_name == "Recipient"
                && member.app_key_count == 1
        }));
    }

    #[test]
    fn share_authority_uses_roster_facet_profile_binding() {
        let owner_keys = Keys::generate();
        let owner_profile_id = IrisProfileId::new_v4();
        let (recipient_keys, recipient) = recipient(ShareRole::Editor);
        let mut folder = create_shared_folder(
            &owner_keys,
            owner_profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Desktop".to_string()),
            vec![recipient],
            10,
        )
        .unwrap();
        folder.participant_profiles.clear();

        let recipient_pubkey = recipient_keys.public_key().to_hex();
        assert_eq!(
            shared_folder_app_key_write_authorization(&folder, &recipient_pubkey),
            ShareRootWriteAuthorization::Authorized
        );
        let mut expected_recipients =
            vec![owner_keys.public_key().to_hex(), recipient_pubkey.clone()];
        expected_recipients.sort();
        assert_eq!(
            shared_folder_key_recipient_pubkeys(&folder),
            expected_recipients
        );
        let view = shared_folder_view(&folder, &[], &recipient_pubkey);
        assert_eq!(view.local_role, ShareRole::Editor);
        assert_eq!(view.key_status, SharedFolderKeyStatus::Available);
        assert!(view.members.iter().any(|member| {
            member.profile_id != owner_profile_id
                && member.role == ShareRole::Editor
                && member.app_key_count == 1
        }));
    }

    #[test]
    fn share_member_authority_projects_from_signed_member_ops() {
        let owner_keys = Keys::generate();
        let owner_profile_id = IrisProfileId::new_v4();
        let (recipient_keys, recipient) = recipient(ShareRole::Editor);
        let mut folder = create_shared_folder(
            &owner_keys,
            owner_profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Desktop".to_string()),
            vec![recipient],
            10,
        )
        .unwrap();
        assert!(
            !folder.member_ops.is_empty(),
            "share membership must have a signed roster log"
        );

        folder.members.clear();
        let recipient_pubkey = recipient_keys.public_key().to_hex();

        assert_eq!(
            shared_folder_app_key_write_authorization(&folder, &recipient_pubkey),
            ShareRootWriteAuthorization::Authorized
        );
        assert_eq!(shared_folder_key_recipient_pubkeys(&folder), {
            let mut pubkeys = vec![owner_keys.public_key().to_hex(), recipient_pubkey.clone()];
            pubkeys.sort();
            pubkeys
        });

        let view = shared_folder_view(&folder, &[], &recipient_pubkey);
        assert_eq!(view.local_role, ShareRole::Editor);
        assert_eq!(view.participant_count, 2);
        assert!(view.members.iter().any(|member| {
            member.profile_id != owner_profile_id
                && member.role == ShareRole::Editor
                && member.status == ShareMemberStatus::Active
                && member.display_name == "Recipient"
        }));

        let checkpoint = sign_share_roster_checkpoint(&owner_keys, &folder, 20).unwrap();
        assert_eq!(checkpoint.content.accepted_member_op_count, 2);
        assert_eq!(checkpoint.content.rejected_member_op_count, 0);
        assert_eq!(checkpoint.content.members.len(), 2);
        validate_share_roster_checkpoint(&folder, &checkpoint).unwrap();
    }

    #[test]
    fn shared_folder_members_group_multiple_app_keys_by_profile() {
        let owner_keys = Keys::generate();
        let owner_profile_id = IrisProfileId::new_v4();
        let recipient_profile_id = IrisProfileId::new_v4();
        let laptop_keys = Keys::generate();
        let phone_keys = Keys::generate();
        let folder = create_shared_folder(
            &owner_keys,
            owner_profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Owner laptop".to_string()),
            vec![
                ShareRecipient {
                    profile_id: recipient_profile_id,
                    app_pubkey: laptop_keys.public_key().to_hex(),
                    role: ShareRole::Reader,
                    label: Some("Laptop".to_string()),
                    representative_npub_hint: Some("npub1alice".to_string()),
                    display_name: Some("Alice".to_string()),
                },
                ShareRecipient {
                    profile_id: recipient_profile_id,
                    app_pubkey: phone_keys.public_key().to_hex(),
                    role: ShareRole::Editor,
                    label: Some("Phone".to_string()),
                    representative_npub_hint: Some("npub1alice".to_string()),
                    display_name: Some("Alice".to_string()),
                },
            ],
            10,
        )
        .unwrap();

        let view = shared_folder_view(&folder, &[], &laptop_keys.public_key().to_hex());
        let alice = view
            .members
            .iter()
            .find(|member| member.profile_id == recipient_profile_id)
            .expect("recipient member is projected");

        assert_eq!(view.participant_count, 2);
        assert_eq!(view.app_key_count, 3);
        assert_eq!(alice.role, ShareRole::Editor);
        assert_eq!(alice.display_name, "Alice");
        assert_eq!(
            alice.representative_npub_hint.as_deref(),
            Some("npub1alice")
        );
        assert_eq!(alice.app_key_count, 2);
        assert_eq!(view.local_role, ShareRole::Editor);
        assert!(view.can_write);
    }

    #[test]
    fn share_recipient_resolution_uses_representative_social_self_link() {
        let ShareRecipientResolutionEvidence {
            profile_id,
            social_pubkey,
            phone_pubkey,
            ops,
            acceptances,
        } = accepted_share_recipient_resolution_evidence();
        let resolved = resolve_share_recipient_from_profile_evidence(
            profile_id,
            &social_pubkey,
            &ops,
            &acceptances,
            None,
        )
        .unwrap();
        assert_eq!(resolved.profile_id, profile_id);
        assert_eq!(resolved.representative_pubkey, social_pubkey);
        assert_eq!(resolved.representative_npub, pubkey_npub(&social_pubkey));
        assert_eq!(resolved.display_name.as_deref(), Some("Alice"));
        assert_eq!(resolved.app_pubkeys, vec![phone_pubkey.clone()]);
        assert_eq!(resolved.linked_social_pubkeys, vec![social_pubkey.clone()]);
        assert_eq!(
            resolved.share_recipients(ShareRole::Reader),
            vec![ShareRecipient {
                profile_id,
                app_pubkey: phone_pubkey,
                role: ShareRole::Reader,
                label: None,
                representative_npub_hint: Some(pubkey_npub(&social_pubkey)),
                display_name: Some("Alice".to_string()),
            }]
        );
    }

    #[test]
    fn share_recipient_resolution_requires_representative_self_link() {
        let profile_id = IrisProfileId::new_v4();
        let admin = Keys::generate();
        let social = Keys::generate();
        let phone = Keys::generate();
        let social_pubkey = social.public_key().to_hex();
        let phone_pubkey = phone.public_key().to_hex();
        let mut ops = Vec::new();
        append_profile_facet(
            &mut ops,
            &admin,
            profile_id,
            IrisProfileFacet::app_key(
                admin.public_key().to_hex(),
                10,
                Some("Admin".to_string()),
                IrisProfileCapabilities::app_admin(),
            ),
            10,
        );
        append_profile_facet(
            &mut ops,
            &admin,
            profile_id,
            IrisProfileFacet::social_profile(social_pubkey.clone(), 11, None),
            11,
        );
        let phone_op = append_profile_facet(
            &mut ops,
            &admin,
            profile_id,
            IrisProfileFacet::app_key(
                phone_pubkey,
                12,
                Some("Phone".to_string()),
                IrisProfileCapabilities::app_reader(),
            ),
            12,
        );
        let acceptances = vec![facet_acceptance(
            &phone,
            profile_id,
            IrisProfileKeyPurpose::AppKey,
            Some(phone_op.op_id),
            20,
        )];

        assert!(matches!(
            resolve_share_recipient_from_profile_evidence(
                profile_id,
                &social_pubkey,
                &ops,
                &acceptances,
                None,
            ),
            Err(SharingError::RecipientResolution(_))
        ));
    }

    #[test]
    fn resolved_share_invite_wraps_all_appkeys_with_one_epoch() {
        let owner = Keys::generate();
        let owner_profile_id = IrisProfileId::new_v4();
        let recipient_profile_id = IrisProfileId::new_v4();
        let phone_pubkey = Keys::generate().public_key().to_hex();
        let laptop_pubkey = Keys::generate().public_key().to_hex();
        let mut app_pubkeys = vec![phone_pubkey.clone(), laptop_pubkey.clone()];
        app_pubkeys.sort();
        let mut folder = create_shared_folder(
            &owner,
            owner_profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Owner".to_string()),
            Vec::new(),
            10,
        )
        .unwrap();
        let resolved = ResolvedShareRecipient {
            profile_id: recipient_profile_id,
            representative_pubkey: phone_pubkey.clone(),
            representative_npub: pubkey_npub(&phone_pubkey),
            display_name: Some("Alice".to_string()),
            app_pubkeys: app_pubkeys.clone(),
            linked_social_pubkeys: vec![phone_pubkey.clone()],
        };

        let outcome = invite_shared_folder_resolved_recipient(
            &mut folder,
            &owner,
            &resolved,
            ShareRole::Editor,
            20,
        )
        .unwrap();

        assert_eq!(outcome.profile_id, recipient_profile_id);
        assert_eq!(outcome.epoch, 2);
        let projection = folder.projection();
        let epoch = projection.key_epochs.get(&2).unwrap();
        assert!(epoch.wrapped_dck.contains_key(&owner.public_key().to_hex()));
        assert!(epoch.wrapped_dck.contains_key(&phone_pubkey));
        assert!(epoch.wrapped_dck.contains_key(&laptop_pubkey));
        assert_eq!(
            folder
                .members
                .get(&recipient_profile_id.to_string())
                .unwrap()
                .role,
            ShareRole::Editor
        );
        let view = shared_folder_view(&folder, &[], &phone_pubkey);
        let alice = view
            .members
            .iter()
            .find(|member| member.profile_id == recipient_profile_id)
            .unwrap();
        assert_eq!(alice.display_name, "Alice");
        assert_eq!(alice.app_key_count, 2);
        let bundle = parse_share_invite(&outcome.invite_url).unwrap();
        assert_eq!(bundle.recipient_profile_id, recipient_profile_id);
        assert!(bundle.roster_checkpoint.is_some());
    }

    #[test]
    fn shared_folder_view_distinguishes_missing_epoch_from_unavailable_key() {
        let owner_keys = Keys::generate();
        let owner_pubkey = owner_keys.public_key().to_hex();
        let owner_profile_id = IrisProfileId::new_v4();
        let share_id = IrisProfileId::new_v4();
        let folder = SharedFolder {
            share_id,
            owner_profile_id,
            source_path: "Projects/Alpha".to_string(),
            display_name: "Alpha".to_string(),
            local_role: ShareRole::Admin,
            members: BTreeMap::from([(
                owner_profile_id.to_string(),
                ShareMember::active(
                    owner_profile_id,
                    ShareRole::Admin,
                    None,
                    Some("Desktop".to_string()),
                ),
            )]),
            member_ops: Vec::new(),
            participant_profiles: BTreeMap::from([(owner_pubkey.clone(), owner_profile_id)]),
            app_key_roots: BTreeMap::new(),
            roster_ops: vec![
                sign_share_roster_op(
                    &owner_keys,
                    share_id,
                    IrisProfileRosterOp::AddFacet {
                        facet: IrisProfileFacet::app_key(
                            owner_pubkey.clone(),
                            10,
                            Some("Desktop".to_string()),
                            ShareRole::Admin.capabilities(),
                        ),
                    },
                    10,
                )
                .unwrap(),
            ],
        };

        let view = shared_folder_view(&folder, &[], &owner_pubkey);

        assert_eq!(view.current_key_epoch, None);
        assert_eq!(view.key_status, SharedFolderKeyStatus::NoKeyEpoch);
        assert!(!view.has_current_key_wrap);
        assert!(!view.key_unavailable);
        assert!(!view.repair_needed);
        assert!(view.missing_key_wrap_pubkeys.is_empty());
    }

    #[test]
    fn shared_folder_view_surfaces_repair_needed_and_key_unavailable() {
        let (folder, owner_pubkey, recipient_pubkey) = folder_with_recipient_missing_wrap();

        let owner_view = shared_folder_view(&folder, &[], &owner_pubkey);
        let recipient_view = shared_folder_view(&folder, &[], &recipient_pubkey);

        assert_eq!(owner_view.key_status, SharedFolderKeyStatus::RepairNeeded);
        assert!(owner_view.has_current_key_wrap);
        assert!(!owner_view.key_unavailable);
        assert!(owner_view.repair_needed);
        assert_eq!(
            owner_view.missing_key_wrap_pubkeys,
            vec![recipient_pubkey.clone()]
        );
        assert_eq!(
            recipient_view.key_status,
            SharedFolderKeyStatus::KeyUnavailable
        );
        assert!(!recipient_view.has_current_key_wrap);
        assert!(recipient_view.key_unavailable);
        assert!(recipient_view.repair_needed);
        assert_eq!(
            recipient_view.missing_key_wrap_pubkeys,
            vec![recipient_pubkey]
        );
    }

    #[test]
    fn shared_folder_missing_key_wraps_can_be_repaired_by_epoch_signing_admin() {
        let owner_keys = Keys::generate();
        let recipient_keys = Keys::generate();
        let owner_pubkey = owner_keys.public_key().to_hex();
        let recipient_pubkey = recipient_keys.public_key().to_hex();
        let recipient_profile_id = IrisProfileId::new_v4();
        let mut folder = create_shared_folder(
            &owner_keys,
            IrisProfileId::new_v4(),
            "Projects/Alpha",
            "Alpha",
            Some("Desktop".to_string()),
            Vec::new(),
            10,
        )
        .unwrap();
        let owner_share_key = current_shared_folder_key(&folder, &owner_keys).unwrap();
        folder.roster_ops.push(
            sign_share_roster_op_with_parents(
                &owner_keys,
                folder.share_id,
                iris_profile_roster_parent_ids(&folder.roster_ops),
                IrisProfileRosterOp::AddFacet {
                    facet: IrisProfileFacet::app_key(
                        recipient_pubkey.clone(),
                        12,
                        Some("Phone".to_string()),
                        ShareRole::Editor.capabilities(),
                    ),
                },
                12,
            )
            .unwrap(),
        );
        folder
            .participant_profiles
            .insert(recipient_pubkey.clone(), recipient_profile_id);
        append_share_member_grant(
            &mut folder,
            &owner_keys,
            ShareMember::active(
                recipient_profile_id,
                ShareRole::Editor,
                None,
                Some("Phone".to_string()),
            ),
            12,
        );

        let unavailable = shared_folder_view(&folder, &[], &recipient_pubkey);
        assert_eq!(
            unavailable.key_status,
            SharedFolderKeyStatus::KeyUnavailable
        );
        assert!(matches!(
            current_shared_folder_key(&folder, &recipient_keys),
            Err(SharingError::NoWrapForCurrentAppKey)
        ));

        let repair = repair_shared_folder_key_epoch_wraps(&mut folder, &owner_keys, 13).unwrap();

        assert_eq!(repair.epoch, 1);
        assert_eq!(repair.repaired_pubkeys, vec![recipient_pubkey.clone()]);
        assert_eq!(repair.share_id, folder.share_id);
        assert_eq!(
            folder
                .projection()
                .active_key_recipients_missing_wraps(repair.epoch),
            Vec::<String>::new()
        );
        assert_eq!(
            current_shared_folder_key(&folder, &recipient_keys).unwrap(),
            owner_share_key
        );
        let repaired_owner_view = shared_folder_view(&folder, &[], &owner_pubkey);
        let repaired_recipient_view = shared_folder_view(&folder, &[], &recipient_pubkey);
        assert_eq!(
            repaired_owner_view.key_status,
            SharedFolderKeyStatus::Available
        );
        assert_eq!(
            repaired_recipient_view.key_status,
            SharedFolderKeyStatus::Available
        );
    }

    #[test]
    fn shared_folder_key_wrap_repair_must_be_signed_by_epoch_signer() {
        let owner_keys = Keys::generate();
        let other_admin_keys = Keys::generate();
        let other_admin_pubkey = other_admin_keys.public_key().to_hex();
        let other_admin_profile_id = IrisProfileId::new_v4();
        let mut folder = create_shared_folder(
            &owner_keys,
            IrisProfileId::new_v4(),
            "Projects/Alpha",
            "Alpha",
            Some("Desktop".to_string()),
            Vec::new(),
            10,
        )
        .unwrap();
        folder.roster_ops.push(
            sign_share_roster_op_with_parents(
                &owner_keys,
                folder.share_id,
                iris_profile_roster_parent_ids(&folder.roster_ops),
                IrisProfileRosterOp::AddFacet {
                    facet: IrisProfileFacet::app_key(
                        other_admin_pubkey.clone(),
                        12,
                        Some("Other admin".to_string()),
                        ShareRole::Admin.capabilities(),
                    ),
                },
                12,
            )
            .unwrap(),
        );
        folder
            .participant_profiles
            .insert(other_admin_pubkey.clone(), other_admin_profile_id);
        append_share_member_grant(
            &mut folder,
            &owner_keys,
            ShareMember::active(
                other_admin_profile_id,
                ShareRole::Admin,
                None,
                Some("Other admin".to_string()),
            ),
            12,
        );

        match repair_shared_folder_key_epoch_wraps(&mut folder, &other_admin_keys, 13) {
            Err(SharingError::CurrentAppKeyCannotRepairKeyEpoch { signed_by_pubkey }) => {
                assert_eq!(signed_by_pubkey, owner_keys.public_key().to_hex());
            }
            other => panic!("expected epoch signer error, got {other:?}"),
        }
        assert_eq!(
            folder.projection().active_key_recipients_missing_wraps(1),
            vec![other_admin_pubkey]
        );
    }

    #[test]
    fn shared_folder_key_wrap_repair_requires_active_share_admin_member() {
        let owner_keys = Keys::generate();
        let owner_pubkey = owner_keys.public_key().to_hex();
        let signer_keys = Keys::generate();
        let signer_pubkey = signer_keys.public_key().to_hex();
        let signer_profile_id = IrisProfileId::new_v4();
        let recipient_keys = Keys::generate();
        let recipient_pubkey = recipient_keys.public_key().to_hex();
        let recipient_profile_id = IrisProfileId::new_v4();
        let mut folder = create_shared_folder(
            &owner_keys,
            IrisProfileId::new_v4(),
            "Projects/Alpha",
            "Alpha",
            Some("Owner".to_string()),
            Vec::new(),
            10,
        )
        .unwrap();
        folder.roster_ops.push(
            sign_share_roster_op_with_parents(
                &owner_keys,
                folder.share_id,
                iris_profile_roster_parent_ids(&folder.roster_ops),
                IrisProfileRosterOp::AddFacet {
                    facet: IrisProfileFacet::app_key(
                        signer_pubkey.clone(),
                        12,
                        Some("Signer".to_string()),
                        ShareRole::Admin.capabilities(),
                    )
                    .with_profile_id(signer_profile_id),
                },
                12,
            )
            .unwrap(),
        );
        folder
            .participant_profiles
            .insert(signer_pubkey.clone(), signer_profile_id);
        append_share_member_grant(
            &mut folder,
            &owner_keys,
            ShareMember::active(
                signer_profile_id,
                ShareRole::Editor,
                None,
                Some("Signer".to_string()),
            ),
            12,
        );

        let share_key = current_shared_folder_key(&folder, &owner_keys).unwrap();
        let wrapped_dck = wrap_share_key(
            &signer_keys,
            [owner_pubkey.as_str(), signer_pubkey.as_str()],
            &share_key,
        )
        .unwrap();
        folder.roster_ops.push(
            sign_share_roster_op_with_parents(
                &signer_keys,
                folder.share_id,
                iris_profile_roster_parent_ids(&folder.roster_ops),
                IrisProfileRosterOp::RotateKeyEpoch {
                    epoch: 2,
                    wrapped_dck,
                },
                13,
            )
            .unwrap(),
        );
        folder.roster_ops.push(
            sign_share_roster_op_with_parents(
                &owner_keys,
                folder.share_id,
                iris_profile_roster_parent_ids(&folder.roster_ops),
                IrisProfileRosterOp::AddFacet {
                    facet: IrisProfileFacet::app_key(
                        recipient_pubkey.clone(),
                        14,
                        Some("Recipient".to_string()),
                        ShareRole::Reader.capabilities(),
                    )
                    .with_profile_id(recipient_profile_id),
                },
                14,
            )
            .unwrap(),
        );
        folder
            .participant_profiles
            .insert(recipient_pubkey.clone(), recipient_profile_id);
        append_share_member_grant(
            &mut folder,
            &owner_keys,
            ShareMember::active(
                recipient_profile_id,
                ShareRole::Reader,
                None,
                Some("Recipient".to_string()),
            ),
            14,
        );

        assert_eq!(
            active_share_key_recipients_missing_wraps(&folder, &folder.projection(), 2),
            vec![recipient_pubkey]
        );
        assert!(matches!(
            repair_shared_folder_key_epoch_wraps(&mut folder, &signer_keys, 15),
            Err(SharingError::CurrentAppKeyCannotAdminShare)
        ));
    }

    #[test]
    fn revoking_share_member_tombstones_profile_app_keys_and_rotates_epoch() {
        let owner_keys = Keys::generate();
        let owner_profile_id = IrisProfileId::new_v4();
        let recipient_profile_id = IrisProfileId::new_v4();
        let laptop_keys = Keys::generate();
        let phone_keys = Keys::generate();
        let mut folder = create_shared_folder(
            &owner_keys,
            owner_profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Owner".to_string()),
            vec![
                ShareRecipient {
                    profile_id: recipient_profile_id,
                    app_pubkey: laptop_keys.public_key().to_hex(),
                    role: ShareRole::Editor,
                    label: Some("Laptop".to_string()),
                    representative_npub_hint: Some("npub1alice".to_string()),
                    display_name: Some("Alice".to_string()),
                },
                ShareRecipient {
                    profile_id: recipient_profile_id,
                    app_pubkey: phone_keys.public_key().to_hex(),
                    role: ShareRole::Editor,
                    label: Some("Phone".to_string()),
                    representative_npub_hint: Some("npub1alice".to_string()),
                    display_name: Some("Alice".to_string()),
                },
            ],
            10,
        )
        .unwrap();

        let outcome = revoke_shared_folder_member(
            &mut folder,
            &owner_keys,
            recipient_profile_id,
            Some("removed"),
            20,
        )
        .unwrap();
        let projection = folder.projection();

        assert_eq!(outcome.profile_id, recipient_profile_id);
        assert_eq!(outcome.epoch, 2);
        let mut expected_revoked = vec![
            laptop_keys.public_key().to_hex(),
            phone_keys.public_key().to_hex(),
        ];
        expected_revoked.sort();
        assert_eq!(outcome.revoked_app_pubkeys, expected_revoked);
        assert_eq!(
            folder
                .members
                .get(&recipient_profile_id.to_string())
                .unwrap()
                .status,
            ShareMemberStatus::Revoked
        );
        assert!(
            projection
                .tombstones
                .contains_key(&laptop_keys.public_key().to_hex())
        );
        assert!(
            projection
                .tombstones
                .contains_key(&phone_keys.public_key().to_hex())
        );
        let epoch = projection.key_epochs.get(&2).unwrap();
        assert!(
            epoch
                .wrapped_dck
                .contains_key(&owner_keys.public_key().to_hex())
        );
        assert!(
            !epoch
                .wrapped_dck
                .contains_key(&laptop_keys.public_key().to_hex())
        );
        assert!(
            !epoch
                .wrapped_dck
                .contains_key(&phone_keys.public_key().to_hex())
        );

        let laptop_pubkey = laptop_keys.public_key().to_hex();
        let current_key = current_shared_folder_key(&folder, &owner_keys).unwrap();
        let leaked_wrap = wrap_share_key(
            &owner_keys,
            std::iter::once(laptop_pubkey.as_str()),
            &current_key,
        )
        .unwrap();
        folder.roster_ops.push(
            sign_share_roster_op_with_parents(
                &owner_keys,
                folder.share_id,
                iris_profile_roster_parent_ids(&folder.roster_ops),
                IrisProfileRosterOp::RepairKeyWraps {
                    epoch: 2,
                    wrapped_dck: leaked_wrap,
                },
                21,
            )
            .unwrap(),
        );
        assert!(matches!(
            current_shared_folder_key(&folder, &laptop_keys),
            Err(SharingError::ShareMemberRevoked(profile_id))
                if profile_id == recipient_profile_id
        ));

        let revoked_view = shared_folder_view(&folder, &[], &laptop_keys.public_key().to_hex());
        assert_eq!(revoked_view.key_status, SharedFolderKeyStatus::Revoked);
        assert_eq!(
            revoked_view.write_authorization,
            ShareRootWriteAuthorization::RevokedMember
        );
        assert!(!revoked_view.can_write);
    }

    #[test]
    fn share_invite_adds_profile_member_rotates_epoch_and_accepts_for_recipient() {
        let owner_keys = Keys::generate();
        let owner_profile_id = IrisProfileId::new_v4();
        let recipient_keys = Keys::generate();
        let recipient_profile_id = IrisProfileId::new_v4();
        let mut folder = create_shared_folder(
            &owner_keys,
            owner_profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Owner".to_string()),
            Vec::new(),
            10,
        )
        .unwrap();

        let invite = invite_shared_folder_member(
            &mut folder,
            &owner_keys,
            ShareRecipient {
                profile_id: recipient_profile_id,
                app_pubkey: recipient_keys.public_key().to_hex(),
                role: ShareRole::Reader,
                label: Some("Phone".to_string()),
                representative_npub_hint: Some("npub1alice".to_string()),
                display_name: Some("Alice".to_string()),
            },
            20,
        )
        .unwrap();
        let accepted =
            shared_folder_from_invite_for_profile(&invite.invite_url, recipient_profile_id)
                .unwrap();
        let bundle = parse_share_invite(&invite.invite_url).unwrap();
        let checkpoint = bundle.roster_checkpoint.as_ref().unwrap();
        let projection = accepted.projection();

        assert!(invite.invite_url.starts_with(SHARE_INVITE_PREFIX));
        assert_eq!(invite.epoch, 2);
        assert_eq!(checkpoint.content.share_id, folder.share_id);
        assert_eq!(checkpoint.signer_pubkey, owner_keys.public_key().to_hex());
        assert_eq!(checkpoint.content.accepted_op_count, 4);
        assert_eq!(checkpoint.content.roster_head_op_ids.len(), 1);
        assert_eq!(checkpoint.content.current_key_epoch, Some(2));
        assert_eq!(checkpoint.content.members.len(), 2);
        assert!(checkpoint.content.missing_key_wrap_pubkeys.is_empty());
        assert_eq!(
            accepted
                .members
                .get(&recipient_profile_id.to_string())
                .unwrap()
                .display_name
                .as_deref(),
            Some("Alice")
        );
        assert_eq!(
            projection.key_wrap_status(&recipient_keys.public_key().to_hex(), 2),
            crate::iris_profile::KeyWrapStatus::Available
        );
        assert_eq!(
            current_shared_folder_key(&accepted, &recipient_keys).unwrap(),
            current_shared_folder_key(&accepted, &owner_keys).unwrap()
        );
        assert!(matches!(
            shared_folder_from_invite_for_profile(&invite.invite_url, owner_profile_id),
            Err(SharingError::ShareInviteNotForLocalProfile { .. })
        ));
        assert!(matches!(
            shared_folder_from_invite_for_profile(&invite.invite_url, IrisProfileId::new_v4()),
            Err(SharingError::ShareInviteNotForLocalProfile { .. })
        ));
    }

    #[test]
    fn share_invite_rejects_tampered_roster_checkpoint() {
        let owner_keys = Keys::generate();
        let recipient_keys = Keys::generate();
        let recipient_profile_id = IrisProfileId::new_v4();
        let mut folder = create_shared_folder(
            &owner_keys,
            IrisProfileId::new_v4(),
            "Projects/Alpha",
            "Alpha",
            Some("Owner".to_string()),
            Vec::new(),
            10,
        )
        .unwrap();
        let invite = invite_shared_folder_member(
            &mut folder,
            &owner_keys,
            ShareRecipient {
                profile_id: recipient_profile_id,
                app_pubkey: recipient_keys.public_key().to_hex(),
                role: ShareRole::Reader,
                label: Some("Phone".to_string()),
                representative_npub_hint: None,
                display_name: Some("Alice".to_string()),
            },
            20,
        )
        .unwrap();

        let mut bundle = parse_share_invite(&invite.invite_url).unwrap();
        bundle
            .roster_checkpoint
            .as_mut()
            .unwrap()
            .content
            .accepted_op_count += 1;
        let tampered = encode_share_invite(&bundle).unwrap();

        assert!(matches!(
            parse_share_invite(&tampered),
            Err(SharingError::RosterCheckpoint(_))
        ));
    }

    #[test]
    fn share_invite_acceptance_projects_members_from_signed_member_ops() {
        let owner_keys = Keys::generate();
        let recipient_keys = Keys::generate();
        let recipient_profile_id = IrisProfileId::new_v4();
        let mut folder = create_shared_folder(
            &owner_keys,
            IrisProfileId::new_v4(),
            "Projects/Alpha",
            "Alpha",
            Some("Owner".to_string()),
            Vec::new(),
            10,
        )
        .unwrap();
        let invite = invite_shared_folder_member(
            &mut folder,
            &owner_keys,
            ShareRecipient {
                profile_id: recipient_profile_id,
                app_pubkey: recipient_keys.public_key().to_hex(),
                role: ShareRole::Reader,
                label: Some("Phone".to_string()),
                representative_npub_hint: None,
                display_name: Some("Alice".to_string()),
            },
            20,
        )
        .unwrap();

        let mut bundle = parse_share_invite(&invite.invite_url).unwrap();
        bundle.shared_folder.members.clear();
        let cacheless_invite = encode_share_invite(&bundle).unwrap();

        let accepted =
            shared_folder_from_invite_for_profile(&cacheless_invite, recipient_profile_id).unwrap();
        assert_eq!(accepted.share_id, folder.share_id);
        assert_eq!(
            accepted
                .member_projection()
                .members
                .get(&recipient_profile_id.to_string())
                .unwrap()
                .display_name
                .as_deref(),
            Some("Alice")
        );
    }

    #[test]
    fn share_invite_rejects_tampered_member_roster_event_json() {
        let owner_keys = Keys::generate();
        let recipient_keys = Keys::generate();
        let recipient_profile_id = IrisProfileId::new_v4();
        let mut folder = create_shared_folder(
            &owner_keys,
            IrisProfileId::new_v4(),
            "Projects/Alpha",
            "Alpha",
            Some("Owner".to_string()),
            Vec::new(),
            10,
        )
        .unwrap();
        let invite = invite_shared_folder_member(
            &mut folder,
            &owner_keys,
            ShareRecipient {
                profile_id: recipient_profile_id,
                app_pubkey: recipient_keys.public_key().to_hex(),
                role: ShareRole::Reader,
                label: Some("Phone".to_string()),
                representative_npub_hint: None,
                display_name: Some("Alice".to_string()),
            },
            20,
        )
        .unwrap();

        let mut bundle = parse_share_invite(&invite.invite_url).unwrap();
        bundle.shared_folder.member_ops[0].event_json = "{}".to_string();
        let tampered = encode_share_invite(&bundle).unwrap();

        assert!(matches!(
            parse_share_invite(&tampered),
            Err(SharingError::ShareMemberRoster(_))
        ));
    }

    #[test]
    fn default_share_shortcut_path_is_unique_under_my_drive_parent() {
        let share_id = IrisProfileId::new_v4();
        let existing = vec![
            ShareShortcut::new(share_id, "Projects/Alpha", "").unwrap(),
            ShareShortcut::new(share_id, "Projects/Alpha (2)", "").unwrap(),
        ];

        let path = default_share_shortcut_path(&existing, "Alpha", "Projects").unwrap();

        assert_eq!(path, "Projects/Alpha (3)");
        assert!(default_share_shortcut_path(&existing, "Alpha", "../Projects").is_err());
    }

    #[test]
    fn reader_recipient_gets_key_wrap_without_write_authority() {
        let owner_keys = Keys::generate();
        let (reader_keys, reader) = recipient(ShareRole::Reader);

        let folder = create_shared_folder(
            &owner_keys,
            IrisProfileId::new_v4(),
            "Photos",
            "Photos",
            None,
            vec![reader],
            20,
        )
        .unwrap();

        let projection = folder.projection();
        let reader_pubkey = reader_keys.public_key().to_hex();
        assert!(!projection.can_write_roots(&reader_pubkey));
        assert!(!projection.can_admin_profile(&reader_pubkey));
        assert_eq!(
            projection.key_wrap_status(&reader_pubkey, 1),
            crate::iris_profile::KeyWrapStatus::Available
        );
    }

    #[test]
    fn share_paths_reject_traversal_and_native_separators() {
        let owner_keys = Keys::generate();
        assert!(
            create_shared_folder(
                &owner_keys,
                IrisProfileId::new_v4(),
                "../Secrets",
                "Secrets",
                None,
                Vec::new(),
                30,
            )
            .is_err()
        );
        assert!(ShareShortcut::new(IrisProfileId::new_v4(), "Shared\\Alpha", "").is_err());
    }

    #[test]
    fn shared_folders_and_shortcuts_are_config_helpers() {
        let owner_keys = Keys::generate();
        let folder = create_shared_folder(
            &owner_keys,
            IrisProfileId::new_v4(),
            "Projects/Beta",
            "Beta",
            None,
            Vec::new(),
            40,
        )
        .unwrap();
        let shortcut = ShareShortcut::new(folder.share_id, "Shared/Beta", "").unwrap();

        let mut config = crate::config::AppConfig::default();
        assert!(config.upsert_shared_folder(folder.clone()));
        assert!(!config.upsert_shared_folder(folder.clone()));
        assert_eq!(
            config.shared_folder(folder.share_id).unwrap().source_path,
            "Projects/Beta"
        );
        assert!(config.upsert_share_shortcut(shortcut.clone()));
        assert!(!config.upsert_share_shortcut(shortcut));

        let raw = toml::to_string(&config).unwrap();
        let round_tripped: crate::config::AppConfig = toml::from_str(&raw).unwrap();
        assert_eq!(round_tripped.shared_folders, vec![folder]);
        assert_eq!(round_tripped.share_shortcuts.len(), 1);
    }
}
