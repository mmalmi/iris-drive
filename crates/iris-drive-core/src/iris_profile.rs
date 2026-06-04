//! App-agnostic Iris profile authority.
//!
//! An `IrisProfile` is identified by a UUID and owns a signed, append-only
//! roster of key facets. App installs use `AppKeys` for normal CRDT/root
//! authorship; recovery phrases and NIP-46 signers may help admit or recover
//! `AppKeys` without becoming root writers themselves.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use nostr_sdk::{Event, EventBuilder, JsonUtil, Keys, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const IRIS_PROFILE_ROSTER_SCHEMA: u32 = 1;
pub const KIND_IRIS_PROFILE_ROSTER_OP: u16 = 30_078;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct IrisProfileId(Uuid);

impl IrisProfileId {
    #[must_use]
    pub fn new_v4() -> Self {
        Self(Uuid::new_v4())
    }

    #[must_use]
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }

    #[must_use]
    pub fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl fmt::Display for IrisProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl FromStr for IrisProfileId {
    type Err = uuid::Error;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Uuid::parse_str(value).map(Self)
    }
}

#[derive(Debug, Error)]
pub enum IrisProfileError {
    #[error("nostr event: {0}")]
    Event(String),
    #[error("invalid kind: expected {expected}, got {got}")]
    WrongKind { expected: u16, got: u16 },
    #[error("missing d tag")]
    MissingDTag,
    #[error("d tag malformed: {0}")]
    DTagMalformed(String),
    #[error("content not JSON-decodable: {0}")]
    BadContent(String),
    #[error("unsupported IrisProfile roster schema {0}")]
    UnsupportedSchema(u32),
    #[error("signature verification failed: {0}")]
    SignatureFailed(String),
    #[error("invalid pubkey hex: {0}")]
    InvalidPubkey(String),
    #[error("event signer {signer} does not match op actor {actor}")]
    ActorSignerMismatch { signer: String, actor: String },
    #[error("d-tag profile {d_tag_profile} does not match content profile {content_profile}")]
    ProfileMismatch {
        d_tag_profile: IrisProfileId,
        content_profile: IrisProfileId,
    },
    #[error("d-tag nonce {d_tag_nonce} does not match content nonce {content_nonce}")]
    NonceMismatch {
        d_tag_nonce: String,
        content_nonce: String,
    },
    #[error(
        "event created_at {event_created_at} does not match content created_at {content_created_at}"
    )]
    CreatedAtMismatch {
        event_created_at: i64,
        content_created_at: i64,
    },
    #[error("op profile {op_profile} does not match log profile {log_profile}")]
    LogProfileMismatch {
        log_profile: IrisProfileId,
        op_profile: IrisProfileId,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IrisProfileKeyPurpose {
    AppKey,
    RecoveryPhrase,
    Nip46Signer,
    SocialProfile,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct IrisProfileCapabilities {
    #[serde(default, skip_serializing_if = "is_false")]
    pub can_write_roots: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub can_admin_profile: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub can_recover_app_keys: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub can_receive_key_wraps: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub can_decrypt_key_epochs: bool,
}

impl IrisProfileCapabilities {
    #[must_use]
    pub fn app_admin() -> Self {
        Self {
            can_write_roots: true,
            can_admin_profile: true,
            can_recover_app_keys: false,
            can_receive_key_wraps: true,
            can_decrypt_key_epochs: true,
        }
    }

    #[must_use]
    pub fn app_writer() -> Self {
        Self {
            can_write_roots: true,
            can_admin_profile: false,
            can_recover_app_keys: false,
            can_receive_key_wraps: true,
            can_decrypt_key_epochs: true,
        }
    }

    #[must_use]
    pub fn recovery_phrase() -> Self {
        Self {
            can_write_roots: false,
            can_admin_profile: true,
            can_recover_app_keys: true,
            can_receive_key_wraps: true,
            can_decrypt_key_epochs: true,
        }
    }

    #[must_use]
    pub fn nip46_recovery(can_decrypt_key_epochs: bool) -> Self {
        Self {
            can_write_roots: false,
            can_admin_profile: true,
            can_recover_app_keys: true,
            can_receive_key_wraps: can_decrypt_key_epochs,
            can_decrypt_key_epochs,
        }
    }

    #[must_use]
    pub fn social_profile() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn can_change_key_epochs(&self) -> bool {
        self.can_admin_profile || (self.can_recover_app_keys && self.can_decrypt_key_epochs)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IrisProfileFacet {
    pub pubkey: String,
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub purposes: BTreeSet<IrisProfileKeyPurpose>,
    #[serde(default)]
    pub capabilities: IrisProfileCapabilities,
    pub added_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl IrisProfileFacet {
    #[must_use]
    pub fn app_key(
        pubkey: impl Into<String>,
        added_at: i64,
        label: Option<String>,
        capabilities: IrisProfileCapabilities,
    ) -> Self {
        Self::with_purposes(
            pubkey,
            [IrisProfileKeyPurpose::AppKey],
            capabilities,
            added_at,
            label,
        )
    }

    #[must_use]
    pub fn recovery_phrase(pubkey: impl Into<String>, added_at: i64) -> Self {
        Self::with_purposes(
            pubkey,
            [IrisProfileKeyPurpose::RecoveryPhrase],
            IrisProfileCapabilities::recovery_phrase(),
            added_at,
            Some("Recovery phrase".to_string()),
        )
    }

    #[must_use]
    pub fn nip46(
        pubkey: impl Into<String>,
        added_at: i64,
        label: Option<String>,
        can_decrypt_key_epochs: bool,
    ) -> Self {
        Self::with_purposes(
            pubkey,
            [IrisProfileKeyPurpose::Nip46Signer],
            IrisProfileCapabilities::nip46_recovery(can_decrypt_key_epochs),
            added_at,
            label,
        )
    }

    #[must_use]
    pub fn social_profile(pubkey: impl Into<String>, added_at: i64, label: Option<String>) -> Self {
        Self::with_purposes(
            pubkey,
            [IrisProfileKeyPurpose::SocialProfile],
            IrisProfileCapabilities::social_profile(),
            added_at,
            label,
        )
    }

    #[must_use]
    pub fn with_purposes<I>(
        pubkey: impl Into<String>,
        purposes: I,
        capabilities: IrisProfileCapabilities,
        added_at: i64,
        label: Option<String>,
    ) -> Self
    where
        I: IntoIterator<Item = IrisProfileKeyPurpose>,
    {
        Self {
            pubkey: pubkey.into(),
            purposes: purposes.into_iter().collect(),
            capabilities,
            added_at,
            label,
        }
    }

    #[must_use]
    pub fn has_purpose(&self, purpose: IrisProfileKeyPurpose) -> bool {
        self.purposes.contains(&purpose)
    }

    #[must_use]
    pub fn is_app_key(&self) -> bool {
        self.has_purpose(IrisProfileKeyPurpose::AppKey)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IrisProfileKeyEpoch {
    pub epoch: u64,
    pub created_at: i64,
    pub signed_by_pubkey: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub wrapped_dck: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IrisProfileTombstone {
    pub pubkey: String,
    pub removed_by_pubkey: String,
    pub removed_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum IrisProfileRosterOp {
    AddFacet {
        facet: IrisProfileFacet,
    },
    TombstoneFacet {
        pubkey: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    SetCapabilities {
        pubkey: String,
        capabilities: IrisProfileCapabilities,
    },
    RotateKeyEpoch {
        epoch: u64,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        wrapped_dck: BTreeMap<String, String>,
    },
    RepairKeyWraps {
        epoch: u64,
        #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
        wrapped_dck: BTreeMap<String, String>,
    },
}

impl IrisProfileRosterOp {
    #[must_use]
    pub fn target_pubkey(&self) -> Option<&str> {
        match self {
            Self::AddFacet { facet } => Some(&facet.pubkey),
            Self::TombstoneFacet { pubkey, .. } | Self::SetCapabilities { pubkey, .. } => {
                Some(pubkey)
            }
            Self::RotateKeyEpoch { .. } | Self::RepairKeyWraps { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IrisProfileRosterOpContent {
    pub schema: u32,
    pub profile_id: IrisProfileId,
    pub actor_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_seq: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<String>,
    pub client_nonce: String,
    pub created_at: i64,
    pub op: IrisProfileRosterOp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedIrisProfileRosterOp {
    pub op_id: String,
    pub signer_pubkey: String,
    pub content: IrisProfileRosterOpContent,
    pub event_json: String,
}

pub fn build_iris_profile_roster_op_event(
    signer_keys: &Keys,
    profile_id: IrisProfileId,
    parents: Vec<String>,
    actor_seq: Option<u64>,
    op: IrisProfileRosterOp,
    created_at: i64,
) -> Result<Event, IrisProfileError> {
    let client_nonce = Uuid::new_v4().to_string();
    let content = IrisProfileRosterOpContent {
        schema: IRIS_PROFILE_ROSTER_SCHEMA,
        profile_id,
        actor_pubkey: signer_keys.public_key().to_hex(),
        actor_seq,
        parents,
        client_nonce: client_nonce.clone(),
        created_at,
        op,
    };
    let content_json =
        serde_json::to_string(&content).map_err(|e| IrisProfileError::BadContent(e.to_string()))?;
    let ts = u64::try_from(created_at).unwrap_or(0);
    EventBuilder::new(
        Kind::from(KIND_IRIS_PROFILE_ROSTER_OP),
        content_json,
        [Tag::identifier(iris_profile_roster_op_d_tag(
            profile_id,
            &client_nonce,
        ))],
    )
    .custom_created_at(nostr_sdk::Timestamp::from(ts))
    .to_event(signer_keys)
    .map_err(|e| IrisProfileError::Event(e.to_string()))
}

#[must_use]
pub fn iris_profile_roster_op_d_tag(profile_id: IrisProfileId, client_nonce: &str) -> String {
    format!("iris-profile/{profile_id}/roster-op/{client_nonce}")
}

pub fn parse_iris_profile_roster_op_event(
    event: &Event,
) -> Result<SignedIrisProfileRosterOp, IrisProfileError> {
    let kind = event.kind.as_u16();
    if kind != KIND_IRIS_PROFILE_ROSTER_OP {
        return Err(IrisProfileError::WrongKind {
            expected: KIND_IRIS_PROFILE_ROSTER_OP,
            got: kind,
        });
    }
    let d_tag = event.identifier().ok_or(IrisProfileError::MissingDTag)?;
    let (d_tag_profile, d_tag_nonce) = parse_iris_profile_roster_op_d_tag(d_tag)?;
    event
        .verify()
        .map_err(|e| IrisProfileError::SignatureFailed(e.to_string()))?;
    let content: IrisProfileRosterOpContent = serde_json::from_str(&event.content)
        .map_err(|e| IrisProfileError::BadContent(format!("IrisProfile roster op content: {e}")))?;
    if content.schema != IRIS_PROFILE_ROSTER_SCHEMA {
        return Err(IrisProfileError::UnsupportedSchema(content.schema));
    }
    if content.profile_id != d_tag_profile {
        return Err(IrisProfileError::ProfileMismatch {
            d_tag_profile,
            content_profile: content.profile_id,
        });
    }
    if content.client_nonce != d_tag_nonce {
        return Err(IrisProfileError::NonceMismatch {
            d_tag_nonce,
            content_nonce: content.client_nonce,
        });
    }
    let event_created_at = i64::try_from(event.created_at.as_u64()).unwrap_or(i64::MAX);
    if content.created_at != event_created_at {
        return Err(IrisProfileError::CreatedAtMismatch {
            event_created_at,
            content_created_at: content.created_at,
        });
    }
    let signer_pubkey = event.pubkey.to_hex();
    if signer_pubkey != content.actor_pubkey {
        return Err(IrisProfileError::ActorSignerMismatch {
            signer: signer_pubkey,
            actor: content.actor_pubkey,
        });
    }
    validate_pubkey(&content.actor_pubkey)?;
    if let Some(target) = content.op.target_pubkey() {
        validate_pubkey(target)?;
    }
    Ok(SignedIrisProfileRosterOp {
        op_id: event.id.to_hex(),
        signer_pubkey,
        content,
        event_json: event.as_json(),
    })
}

fn parse_iris_profile_roster_op_d_tag(
    d_tag: &str,
) -> Result<(IrisProfileId, String), IrisProfileError> {
    let rest = d_tag
        .strip_prefix("iris-profile/")
        .ok_or_else(|| IrisProfileError::DTagMalformed(format!("missing prefix: {d_tag}")))?;
    let (profile, nonce) = rest
        .split_once("/roster-op/")
        .ok_or_else(|| IrisProfileError::DTagMalformed(format!("missing roster op: {d_tag}")))?;
    if nonce.is_empty() || nonce.contains('/') {
        return Err(IrisProfileError::DTagMalformed(format!(
            "invalid nonce: {d_tag}"
        )));
    }
    let profile_id = IrisProfileId::from_str(profile)
        .map_err(|e| IrisProfileError::DTagMalformed(format!("invalid profile UUID: {e}")))?;
    Ok((profile_id, nonce.to_string()))
}

#[derive(Debug, Clone)]
pub struct IrisProfileRosterLog {
    pub profile_id: IrisProfileId,
    ops: BTreeMap<String, SignedIrisProfileRosterOp>,
}

impl IrisProfileRosterLog {
    #[must_use]
    pub fn new(profile_id: IrisProfileId) -> Self {
        Self {
            profile_id,
            ops: BTreeMap::new(),
        }
    }

    pub fn insert_event(&mut self, event: &Event) -> Result<bool, IrisProfileError> {
        self.insert_signed_op(parse_iris_profile_roster_op_event(event)?)
    }

    pub fn insert_signed_op(
        &mut self,
        op: SignedIrisProfileRosterOp,
    ) -> Result<bool, IrisProfileError> {
        if op.content.profile_id != self.profile_id {
            return Err(IrisProfileError::LogProfileMismatch {
                log_profile: self.profile_id,
                op_profile: op.content.profile_id,
            });
        }
        let existed = self.ops.contains_key(&op.op_id);
        self.ops.insert(op.op_id.clone(), op);
        Ok(!existed)
    }

    pub fn merge(&mut self, other: &Self) -> Result<(), IrisProfileError> {
        if other.profile_id != self.profile_id {
            return Err(IrisProfileError::LogProfileMismatch {
                log_profile: self.profile_id,
                op_profile: other.profile_id,
            });
        }
        self.ops.extend(other.ops.clone());
        Ok(())
    }

    #[must_use]
    pub fn project(&self) -> IrisProfileRosterProjection {
        project_iris_profile_roster(self.profile_id, self.ops.values().cloned())
    }

    #[must_use]
    pub fn op_ids(&self) -> Vec<String> {
        self.ops.keys().cloned().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrisProfileRosterProjection {
    pub profile_id: IrisProfileId,
    pub active_facets: BTreeMap<String, IrisProfileFacet>,
    pub tombstones: BTreeMap<String, IrisProfileTombstone>,
    pub key_epochs: BTreeMap<u64, IrisProfileKeyEpoch>,
    pub accepted_op_ids: Vec<String>,
    pub rejected_op_ids: Vec<String>,
}

impl IrisProfileRosterProjection {
    #[must_use]
    pub fn can_write_roots(&self, pubkey: &str) -> bool {
        self.active_facets
            .get(pubkey)
            .is_some_and(|facet| facet.is_app_key() && facet.capabilities.can_write_roots)
    }

    #[must_use]
    pub fn can_admin_profile(&self, pubkey: &str) -> bool {
        self.active_facets
            .get(pubkey)
            .is_some_and(|facet| facet.capabilities.can_admin_profile)
    }

    #[must_use]
    pub fn active_app_key_pubkeys(&self) -> Vec<String> {
        self.active_facets
            .values()
            .filter(|facet| facet.is_app_key())
            .map(|facet| facet.pubkey.clone())
            .collect()
    }

    #[must_use]
    pub fn active_key_recipients_missing_wraps(&self, epoch: u64) -> Vec<String> {
        let Some(key_epoch) = self.key_epochs.get(&epoch) else {
            return Vec::new();
        };
        self.active_facets
            .values()
            .filter(|facet| facet.capabilities.can_receive_key_wraps)
            .filter(|facet| !key_epoch.wrapped_dck.contains_key(&facet.pubkey))
            .map(|facet| facet.pubkey.clone())
            .collect()
    }

    #[must_use]
    pub fn key_wrap_status(&self, pubkey: &str, epoch: u64) -> KeyWrapStatus {
        if self.tombstones.contains_key(pubkey) {
            return KeyWrapStatus::Tombstoned;
        }
        let Some(facet) = self.active_facets.get(pubkey) else {
            return KeyWrapStatus::NoSuchFacet;
        };
        if !facet.capabilities.can_receive_key_wraps {
            return KeyWrapStatus::NotAKeyRecipient;
        }
        let Some(key_epoch) = self.key_epochs.get(&epoch) else {
            return KeyWrapStatus::NoSuchEpoch;
        };
        if key_epoch.wrapped_dck.contains_key(pubkey) {
            KeyWrapStatus::Available
        } else {
            KeyWrapStatus::RepairNeeded
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyWrapStatus {
    Available,
    RepairNeeded,
    NotAKeyRecipient,
    Tombstoned,
    NoSuchFacet,
    NoSuchEpoch,
}

#[must_use]
pub fn project_iris_profile_roster<I>(
    profile_id: IrisProfileId,
    ops: I,
) -> IrisProfileRosterProjection
where
    I: IntoIterator<Item = SignedIrisProfileRosterOp>,
{
    let mut projection = IrisProfileRosterProjection {
        profile_id,
        active_facets: BTreeMap::new(),
        tombstones: BTreeMap::new(),
        key_epochs: BTreeMap::new(),
        accepted_op_ids: Vec::new(),
        rejected_op_ids: Vec::new(),
    };
    let mut ops: Vec<_> = ops
        .into_iter()
        .filter(|op| op.content.profile_id == profile_id)
        .collect();
    ops.sort_by(|left, right| {
        left.content
            .created_at
            .cmp(&right.content.created_at)
            .then_with(|| left.op_id.cmp(&right.op_id))
    });

    for op in ops {
        if apply_projected_op(&mut projection, &op) {
            projection.accepted_op_ids.push(op.op_id);
        } else {
            projection.rejected_op_ids.push(op.op_id);
        }
    }
    projection
}

fn apply_projected_op(
    projection: &mut IrisProfileRosterProjection,
    signed: &SignedIrisProfileRosterOp,
) -> bool {
    if !signer_can_apply(projection, signed) {
        return false;
    }
    match &signed.content.op {
        IrisProfileRosterOp::AddFacet { facet } => {
            if projection.tombstones.contains_key(&facet.pubkey) {
                return false;
            }
            if !facet_capabilities_are_valid(facet) {
                return false;
            }
            projection
                .active_facets
                .entry(facet.pubkey.clone())
                .or_insert_with(|| facet.clone());
            true
        }
        IrisProfileRosterOp::TombstoneFacet { pubkey, reason } => {
            projection.active_facets.remove(pubkey);
            projection.tombstones.insert(
                pubkey.clone(),
                IrisProfileTombstone {
                    pubkey: pubkey.clone(),
                    removed_by_pubkey: signed.signer_pubkey.clone(),
                    removed_at: signed.content.created_at,
                    reason: reason.clone(),
                },
            );
            true
        }
        IrisProfileRosterOp::SetCapabilities {
            pubkey,
            capabilities,
        } => {
            if projection.tombstones.contains_key(pubkey) {
                return false;
            }
            let Some(facet) = projection.active_facets.get_mut(pubkey) else {
                return false;
            };
            if !capabilities_are_valid_for_purposes(&facet.purposes, *capabilities) {
                return false;
            }
            facet.capabilities = *capabilities;
            true
        }
        IrisProfileRosterOp::RotateKeyEpoch { epoch, wrapped_dck } => {
            projection.key_epochs.insert(
                *epoch,
                IrisProfileKeyEpoch {
                    epoch: *epoch,
                    created_at: signed.content.created_at,
                    signed_by_pubkey: signed.signer_pubkey.clone(),
                    wrapped_dck: wrapped_dck.clone(),
                },
            );
            true
        }
        IrisProfileRosterOp::RepairKeyWraps { epoch, wrapped_dck } => {
            let key_epoch =
                projection
                    .key_epochs
                    .entry(*epoch)
                    .or_insert_with(|| IrisProfileKeyEpoch {
                        epoch: *epoch,
                        created_at: signed.content.created_at,
                        signed_by_pubkey: signed.signer_pubkey.clone(),
                        wrapped_dck: BTreeMap::new(),
                    });
            key_epoch.wrapped_dck.extend(wrapped_dck.clone());
            true
        }
    }
}

fn signer_can_apply(
    projection: &IrisProfileRosterProjection,
    signed: &SignedIrisProfileRosterOp,
) -> bool {
    if projection.active_facets.is_empty() {
        return is_valid_bootstrap_op(signed);
    }
    let Some(signing_facet) = projection.active_facets.get(&signed.signer_pubkey) else {
        return false;
    };
    match &signed.content.op {
        IrisProfileRosterOp::AddFacet { facet } => {
            if facet.is_app_key() {
                signing_facet.capabilities.can_admin_profile
                    || signing_facet.capabilities.can_recover_app_keys
            } else {
                signing_facet.capabilities.can_admin_profile
            }
        }
        IrisProfileRosterOp::TombstoneFacet { .. }
        | IrisProfileRosterOp::SetCapabilities { .. } => {
            signing_facet.capabilities.can_admin_profile
        }
        IrisProfileRosterOp::RotateKeyEpoch { .. } | IrisProfileRosterOp::RepairKeyWraps { .. } => {
            signing_facet.capabilities.can_change_key_epochs()
        }
    }
}

fn is_valid_bootstrap_op(signed: &SignedIrisProfileRosterOp) -> bool {
    let IrisProfileRosterOp::AddFacet { facet } = &signed.content.op else {
        return false;
    };
    signed.signer_pubkey == facet.pubkey
        && facet.is_app_key()
        && facet.capabilities.can_admin_profile
        && facet.capabilities.can_write_roots
        && facet.capabilities.can_receive_key_wraps
        && facet_capabilities_are_valid(facet)
}

fn facet_capabilities_are_valid(facet: &IrisProfileFacet) -> bool {
    capabilities_are_valid_for_purposes(&facet.purposes, facet.capabilities)
}

fn capabilities_are_valid_for_purposes(
    purposes: &BTreeSet<IrisProfileKeyPurpose>,
    capabilities: IrisProfileCapabilities,
) -> bool {
    if capabilities.can_write_roots && !purposes.contains(&IrisProfileKeyPurpose::AppKey) {
        return false;
    }
    if capabilities.can_decrypt_key_epochs && !capabilities.can_receive_key_wraps {
        return false;
    }
    true
}

fn validate_pubkey(pubkey: &str) -> Result<(), IrisProfileError> {
    PublicKey::from_hex(pubkey).map_err(|e| IrisProfileError::InvalidPubkey(e.to_string()))?;
    Ok(())
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed_op(
        signer: &Keys,
        profile_id: IrisProfileId,
        op: IrisProfileRosterOp,
        created_at: i64,
    ) -> SignedIrisProfileRosterOp {
        let event = build_iris_profile_roster_op_event(
            signer,
            profile_id,
            Vec::new(),
            None,
            op,
            created_at,
        )
        .unwrap();
        parse_iris_profile_roster_op_event(&event).unwrap()
    }

    fn bootstrap_op(
        signer: &Keys,
        profile_id: IrisProfileId,
        created_at: i64,
    ) -> SignedIrisProfileRosterOp {
        signed_op(
            signer,
            profile_id,
            IrisProfileRosterOp::AddFacet {
                facet: IrisProfileFacet::app_key(
                    signer.public_key().to_hex(),
                    created_at,
                    Some("native app".to_string()),
                    IrisProfileCapabilities::app_admin(),
                ),
            },
            created_at,
        )
    }

    fn project(
        profile_id: IrisProfileId,
        ops: Vec<SignedIrisProfileRosterOp>,
    ) -> IrisProfileRosterProjection {
        project_iris_profile_roster(profile_id, ops)
    }

    #[test]
    fn profile_id_is_standard_uuid_v4() {
        let profile_id = IrisProfileId::new_v4();
        assert_eq!(profile_id.as_uuid().get_version_num(), 4);
        assert_eq!(profile_id.to_string().len(), 36);
    }

    #[test]
    fn signed_roster_op_roundtrips_as_verified_nostr_event() {
        let profile_id = IrisProfileId::new_v4();
        let app = Keys::generate();
        let op = bootstrap_op(&app, profile_id, 10);

        assert_eq!(op.signer_pubkey, app.public_key().to_hex());
        assert_eq!(op.content.profile_id, profile_id);
        assert_eq!(op.content.actor_pubkey, app.public_key().to_hex());
        assert!(!op.op_id.is_empty());
        assert!(op.event_json.contains("iris-profile/"));
    }

    #[test]
    fn bootstrap_creates_first_app_key_admin() {
        let profile_id = IrisProfileId::new_v4();
        let app = Keys::generate();
        let projection = project(profile_id, vec![bootstrap_op(&app, profile_id, 10)]);
        let app_pubkey = app.public_key().to_hex();

        assert!(projection.can_write_roots(&app_pubkey));
        assert!(projection.can_admin_profile(&app_pubkey));
        assert_eq!(projection.active_app_key_pubkeys(), vec![app_pubkey]);
        assert_eq!(projection.accepted_op_ids.len(), 1);
        assert!(projection.rejected_op_ids.is_empty());
    }

    #[test]
    fn non_admin_app_key_can_write_roots_but_cannot_mutate_roster() {
        let profile_id = IrisProfileId::new_v4();
        let admin = Keys::generate();
        let member = Keys::generate();
        let stranger = Keys::generate();
        let member_pubkey = member.public_key().to_hex();
        let stranger_pubkey = stranger.public_key().to_hex();
        let ops = vec![
            bootstrap_op(&admin, profile_id, 10),
            signed_op(
                &admin,
                profile_id,
                IrisProfileRosterOp::AddFacet {
                    facet: IrisProfileFacet::app_key(
                        member_pubkey.clone(),
                        11,
                        Some("web app".to_string()),
                        IrisProfileCapabilities::app_writer(),
                    ),
                },
                11,
            ),
            signed_op(
                &member,
                profile_id,
                IrisProfileRosterOp::AddFacet {
                    facet: IrisProfileFacet::app_key(
                        stranger_pubkey.clone(),
                        12,
                        None,
                        IrisProfileCapabilities::app_writer(),
                    ),
                },
                12,
            ),
        ];

        let projection = project(profile_id, ops);

        assert!(projection.can_write_roots(&member_pubkey));
        assert!(!projection.can_admin_profile(&member_pubkey));
        assert!(!projection.active_facets.contains_key(&stranger_pubkey));
        assert_eq!(projection.accepted_op_ids.len(), 2);
        assert_eq!(projection.rejected_op_ids.len(), 1);
    }

    #[test]
    fn recovery_phrase_authorizes_fresh_app_key_without_becoming_root_writer() {
        let profile_id = IrisProfileId::new_v4();
        let admin = Keys::generate();
        let recovery = Keys::generate();
        let recovered_app = Keys::generate();
        let recovery_pubkey = recovery.public_key().to_hex();
        let recovered_pubkey = recovered_app.public_key().to_hex();
        let ops = vec![
            bootstrap_op(&admin, profile_id, 10),
            signed_op(
                &admin,
                profile_id,
                IrisProfileRosterOp::AddFacet {
                    facet: IrisProfileFacet::recovery_phrase(recovery_pubkey.clone(), 11),
                },
                11,
            ),
            signed_op(
                &recovery,
                profile_id,
                IrisProfileRosterOp::AddFacet {
                    facet: IrisProfileFacet::app_key(
                        recovered_pubkey.clone(),
                        12,
                        Some("restored laptop".to_string()),
                        IrisProfileCapabilities::app_admin(),
                    ),
                },
                12,
            ),
        ];

        let projection = project(profile_id, ops);

        assert!(!projection.can_write_roots(&recovery_pubkey));
        assert!(projection.can_admin_profile(&recovery_pubkey));
        assert!(projection.can_write_roots(&recovered_pubkey));
        assert!(projection.can_admin_profile(&recovered_pubkey));
    }

    #[test]
    fn nip46_can_be_recovery_capable_and_receive_epoch_wraps() {
        let profile_id = IrisProfileId::new_v4();
        let admin = Keys::generate();
        let nip46 = Keys::generate();
        let nip46_pubkey = nip46.public_key().to_hex();
        let ops = vec![
            bootstrap_op(&admin, profile_id, 10),
            signed_op(
                &admin,
                profile_id,
                IrisProfileRosterOp::AddFacet {
                    facet: IrisProfileFacet::nip46(
                        nip46_pubkey.clone(),
                        11,
                        Some("bunker".to_string()),
                        true,
                    ),
                },
                11,
            ),
            signed_op(
                &admin,
                profile_id,
                IrisProfileRosterOp::RotateKeyEpoch {
                    epoch: 1,
                    wrapped_dck: BTreeMap::from([
                        (admin.public_key().to_hex(), "wrap-admin".to_string()),
                        (nip46_pubkey.clone(), "wrap-nip46".to_string()),
                    ]),
                },
                12,
            ),
        ];

        let projection = project(profile_id, ops);

        assert!(!projection.can_write_roots(&nip46_pubkey));
        assert!(projection.can_admin_profile(&nip46_pubkey));
        assert_eq!(
            projection.key_wrap_status(&nip46_pubkey, 1),
            KeyWrapStatus::Available
        );
    }

    #[test]
    fn social_profile_facet_cannot_authorize_drive_access() {
        let profile_id = IrisProfileId::new_v4();
        let admin = Keys::generate();
        let social = Keys::generate();
        let attempted_app = Keys::generate();
        let social_pubkey = social.public_key().to_hex();
        let attempted_pubkey = attempted_app.public_key().to_hex();
        let ops = vec![
            bootstrap_op(&admin, profile_id, 10),
            signed_op(
                &admin,
                profile_id,
                IrisProfileRosterOp::AddFacet {
                    facet: IrisProfileFacet::social_profile(
                        social_pubkey.clone(),
                        11,
                        Some("nostr profile".to_string()),
                    ),
                },
                11,
            ),
            signed_op(
                &social,
                profile_id,
                IrisProfileRosterOp::AddFacet {
                    facet: IrisProfileFacet::app_key(
                        attempted_pubkey.clone(),
                        12,
                        None,
                        IrisProfileCapabilities::app_writer(),
                    ),
                },
                12,
            ),
        ];

        let projection = project(profile_id, ops);

        assert!(projection.active_facets.contains_key(&social_pubkey));
        assert!(!projection.can_admin_profile(&social_pubkey));
        assert!(!projection.can_write_roots(&social_pubkey));
        assert!(!projection.active_facets.contains_key(&attempted_pubkey));
    }

    #[test]
    fn divergent_roster_logs_merge_by_union_and_project_deterministically() {
        let profile_id = IrisProfileId::new_v4();
        let admin = Keys::generate();
        let phone = Keys::generate();
        let recovery = Keys::generate();
        let bootstrap = bootstrap_op(&admin, profile_id, 10);
        let phone_op = signed_op(
            &admin,
            profile_id,
            IrisProfileRosterOp::AddFacet {
                facet: IrisProfileFacet::app_key(
                    phone.public_key().to_hex(),
                    11,
                    Some("phone".to_string()),
                    IrisProfileCapabilities::app_writer(),
                ),
            },
            11,
        );
        let recovery_op = signed_op(
            &admin,
            profile_id,
            IrisProfileRosterOp::AddFacet {
                facet: IrisProfileFacet::recovery_phrase(recovery.public_key().to_hex(), 11),
            },
            11,
        );
        let mut left = IrisProfileRosterLog::new(profile_id);
        left.insert_signed_op(bootstrap.clone()).unwrap();
        left.insert_signed_op(phone_op).unwrap();
        let mut right = IrisProfileRosterLog::new(profile_id);
        right.insert_signed_op(bootstrap).unwrap();
        right.insert_signed_op(recovery_op).unwrap();

        left.merge(&right).unwrap();
        let projection = left.project();

        assert!(projection.can_write_roots(&phone.public_key().to_hex()));
        assert!(projection.can_admin_profile(&recovery.public_key().to_hex()));
        assert_eq!(projection.accepted_op_ids.len(), 3);
    }

    #[test]
    fn tombstone_prevents_stale_or_later_resurrection() {
        let profile_id = IrisProfileId::new_v4();
        let admin = Keys::generate();
        let phone = Keys::generate();
        let phone_pubkey = phone.public_key().to_hex();
        let add_phone = signed_op(
            &admin,
            profile_id,
            IrisProfileRosterOp::AddFacet {
                facet: IrisProfileFacet::app_key(
                    phone_pubkey.clone(),
                    11,
                    Some("phone".to_string()),
                    IrisProfileCapabilities::app_writer(),
                ),
            },
            11,
        );
        let remove_phone = signed_op(
            &admin,
            profile_id,
            IrisProfileRosterOp::TombstoneFacet {
                pubkey: phone_pubkey.clone(),
                reason: Some("lost".to_string()),
            },
            12,
        );
        let stale_resurrection = signed_op(
            &admin,
            profile_id,
            IrisProfileRosterOp::AddFacet {
                facet: IrisProfileFacet::app_key(
                    phone_pubkey.clone(),
                    13,
                    Some("same old key".to_string()),
                    IrisProfileCapabilities::app_writer(),
                ),
            },
            13,
        );

        let projection = project(
            profile_id,
            vec![
                bootstrap_op(&admin, profile_id, 10),
                add_phone,
                remove_phone,
                stale_resurrection,
            ],
        );

        assert!(projection.tombstones.contains_key(&phone_pubkey));
        assert!(!projection.active_facets.contains_key(&phone_pubkey));
        assert_eq!(
            projection.key_wrap_status(&phone_pubkey, 1),
            KeyWrapStatus::Tombstoned
        );
    }

    #[test]
    fn active_key_without_epoch_wrap_is_repair_needed_until_wrap_repair() {
        let profile_id = IrisProfileId::new_v4();
        let admin = Keys::generate();
        let phone = Keys::generate();
        let admin_pubkey = admin.public_key().to_hex();
        let phone_pubkey = phone.public_key().to_hex();
        let base_ops = vec![
            bootstrap_op(&admin, profile_id, 10),
            signed_op(
                &admin,
                profile_id,
                IrisProfileRosterOp::AddFacet {
                    facet: IrisProfileFacet::app_key(
                        phone_pubkey.clone(),
                        11,
                        Some("phone".to_string()),
                        IrisProfileCapabilities::app_writer(),
                    ),
                },
                11,
            ),
            signed_op(
                &admin,
                profile_id,
                IrisProfileRosterOp::RotateKeyEpoch {
                    epoch: 1,
                    wrapped_dck: BTreeMap::from([(admin_pubkey, "wrap-admin".to_string())]),
                },
                12,
            ),
        ];
        let needs_repair = project(profile_id, base_ops.clone());

        assert_eq!(
            needs_repair.key_wrap_status(&phone_pubkey, 1),
            KeyWrapStatus::RepairNeeded
        );
        assert_eq!(
            needs_repair.active_key_recipients_missing_wraps(1),
            vec![phone_pubkey.clone()]
        );

        let mut repaired_ops = base_ops;
        repaired_ops.push(signed_op(
            &admin,
            profile_id,
            IrisProfileRosterOp::RepairKeyWraps {
                epoch: 1,
                wrapped_dck: BTreeMap::from([(phone_pubkey.clone(), "wrap-phone".to_string())]),
            },
            13,
        ));
        let repaired = project(profile_id, repaired_ops);

        assert_eq!(
            repaired.key_wrap_status(&phone_pubkey, 1),
            KeyWrapStatus::Available
        );
        assert!(repaired.active_key_recipients_missing_wraps(1).is_empty());
    }
}
