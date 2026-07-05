//! Relay-layer sync: publish + fetch + apply.
//!
//! Two layers:
//!
//! - **Apply (offline)** — `apply_remote_nostr_identity_roster_op_event`,
//!   `apply_remote_share_access_snapshot_event`, `apply_remote_drive_root_event`,
//!   and app-key-link helpers take a parsed Nostr event or direct roster frame and apply the
//!   event's effect onto the config. These are pure functions over data, fully
//!   covered by unit tests.
//!
//! - **Network (live)** — `publish_nostr_identity_roster_ops`,
//!   `publish_drive_root`, `fetch_nostr_identity_roster_ops`,
//!   `fetch_nostr_identity_restore_candidates`,
//!   `fetch_nostr_identity_app_key_approval_candidates`, and `fetch_drive_roots`
//!   wrap nostr-sdk's `Client` for actual relay I/O. Tested manually against
//!   real relays; the wire/apply layers below them are what we cover
//!   automatically.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use nostr_sdk::{Client, ClientOptions, Event, Filter, JsonUtil, Keys, PublicKey, SingleLetterTag};
use thiserror::Error;

use crate::app_key_link_transport::AppKeyLinkRosterFrame;
use crate::app_keys::{AppKeysProjection, ApplyDecision};
use crate::calendar::CALENDAR_TREE_NAME;
use crate::config::{AppConfig, AppKeyRootRef, Drive, DriveRole};
use crate::nostr_events::{
    KIND_DRIVE_ROOT, KIND_HASHTREE_ROOT, build_app_key_link_request_event,
    build_drive_root_publish_event, build_private_hashtree_root_event, drive_root_d_tag,
    parse_app_key_link_request_event, parse_drive_root_event, parse_drive_root_event_for_device,
    parse_drive_root_event_preview,
};
use crate::profile::{app_key_link_invite_keys, app_keys_from_profile_projection};
use crate::relay_filters::{nostr_identity_roster_op_filter, share_access_snapshot_filter};
pub use crate::relay_filters::{subscription_filters, subscription_filters_for_shared_roots};
use crate::{
    KIND_NOSTR_IDENTITY_ROSTER_OP, NostrIdentityId, SignedNostrIdentityRosterOp,
    SignedShareAccessSnapshot, nostr_identity_candidate_ids_for_pubkey_from_events,
    parse_nostr_identity_roster_op_event, parse_share_access_snapshot_event,
    project_nostr_identity_roster,
};

pub const RELAY_SYNC_EVENT_CACHE_LIMIT: usize = 4096;

#[derive(Debug, Error)]
pub enum RelayError {
    #[error("wire: {0}")]
    Wire(#[from] crate::nostr_events::WireError),
    #[error("nostr client: {0}")]
    Client(String),
    #[error("config has no account; run `idrive init` first")]
    NoAccount,
    #[error("invalid pubkey: {0}")]
    InvalidPubkey(String),
    #[error("hashtree root: {0}")]
    HashtreeRoot(String),
    #[error("account: {0}")]
    Profile(#[from] crate::profile::ProfileError),
    #[error("iris profile: {0}")]
    NostrIdentity(#[from] crate::nostr_identity::NostrIdentityError),
    #[error("share access: {0}")]
    ShareAccess(#[from] crate::sharing::SharingError),
    #[error("app-key-link roster: {0}")]
    AppKeyLinkRoster(String),
}

#[must_use]
pub fn relay_source_routes(relay_urls: &[String]) -> Vec<nostr_pubsub::SourceRoute> {
    relay_urls
        .iter()
        .map(|url| {
            nostr_pubsub::SourceRoute::relay(url.clone()).with_reason("iris-drive app relay config")
        })
        .collect()
}

#[must_use]
pub fn relay_urls_from_source_routes(routes: &[nostr_pubsub::SourceRoute]) -> Vec<String> {
    routes
        .iter()
        .filter(|route| route.source.kind == nostr_pubsub::EventSourceKind::Relay)
        .filter_map(|route| route.source.url.clone())
        .collect()
}

#[must_use]
pub fn event_retention_policy(filters: Vec<Filter>) -> nostr_pubsub::EventRetentionPolicy {
    nostr_pubsub::EventRetentionPolicy::new(RELAY_SYNC_EVENT_CACHE_LIMIT, filters)
}

#[must_use]
pub fn relay_event_matches_policy(
    policy: &nostr_pubsub::EventRetentionPolicy,
    event: &Event,
) -> bool {
    if event.verify().is_err() {
        return false;
    }
    nostr_pubsub::VerifiedEvent::try_from(event.clone())
        .is_ok_and(|verified| policy.accepts(&verified))
}

/// Result of applying signed profile roster ops over the direct app-key-link channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKeyLinkRosterApply {
    /// The roster op set is not applicable to this profile/install.
    Ignored,
    /// The local roster already matches this event.
    Current,
    /// The op-log projection changed the local `AppKeys` view.
    Applied(ApplyDecision),
}

/// Result of merging a signed `NostrIdentity` roster op from relay/direct sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NostrIdentityRosterOpApply {
    /// The op belongs to another profile.
    NotOurProfile,
    /// This op id is already present locally.
    Current,
    /// The verified op was unioned into the local profile log.
    Applied,
}

/// Result of applying a signed share access snapshot from relay/direct sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareAccessSnapshotApply {
    /// The snapshot is for a share this config does not know.
    NotOurShare,
    /// The local snapshot is already identical or newer.
    Current,
    /// The event signer is not an admin in the local snapshot.
    NotAdmin,
    /// The canonical access snapshot was replaced.
    Applied,
}

/// Verified roster evidence for an `NostrIdentity` that a recovery/NIP-46
/// pubkey can use to admit a fresh local `AppKey`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrIdentityRestoreCandidate {
    pub profile_id: NostrIdentityId,
    pub recovery_pubkey: String,
    pub can_decrypt_secret_epochs: bool,
    pub accepted_roster_op_count: usize,
    pub active_app_key_count: usize,
    pub latest_roster_op_created_at: Option<i64>,
    pub profile_roster_ops: Vec<SignedNostrIdentityRosterOp>,
}

#[path = "relay_sync/app_key_approval_candidates.rs"]
mod app_key_approval_candidates;
pub use app_key_approval_candidates::{
    NostrIdentityAppKeyApprovalCandidate, fetch_nostr_identity_app_key_approval_candidates,
    nostr_identity_app_key_approval_candidate_filters,
    nostr_identity_app_key_approval_candidates_from_events,
};

/// Result of applying an app-key-link request sent over relay metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKeyLinkRequestApply {
    /// The event is addressed to another profile.
    NotOurProfile,
    /// This install's `AppKey` is not an admin and cannot approve `AppKeys`.
    NotAdmin,
    /// The event was not encrypted for this admin's current invite key.
    InvalidInvite,
    /// The request was already represented locally.
    Current,
    /// The inbound request queue changed.
    Recorded,
}

/// Apply a signed app-key-link request delivered by relay.
pub fn apply_remote_app_key_link_request_event(
    config: &mut AppConfig,
    event: &Event,
) -> Result<AppKeyLinkRequestApply, RelayError> {
    let Some(account) = config.profile.as_mut() else {
        return Err(RelayError::NoAccount);
    };
    let invite_keys = app_key_link_invite_keys(&account.app_key_link_secret)?;
    let frame = match parse_app_key_link_request_event(event, &invite_keys) {
        Ok(frame) => frame,
        Err(crate::nostr_events::WireError::BadContent(_)) => {
            return Ok(AppKeyLinkRequestApply::InvalidInvite);
        }
        Err(error) => return Err(error.into()),
    };
    if frame.profile_id != account.profile_id {
        return Ok(AppKeyLinkRequestApply::NotOurProfile);
    }
    if !account.can_admin_profile() {
        return Ok(AppKeyLinkRequestApply::NotAdmin);
    }
    if !frame.admin_app_key_pubkey.trim().is_empty()
        && frame.admin_app_key_pubkey != account.app_key_pubkey
    {
        return Ok(AppKeyLinkRequestApply::NotAdmin);
    }

    let changed = account.record_inbound_app_key_link_request(
        frame.profile_id,
        &frame.app_key_pubkey,
        frame.label,
        &frame.invite_pubkey,
        Some(frame.url),
        frame.requested_at,
    )?;
    if changed {
        Ok(AppKeyLinkRequestApply::Recorded)
    } else {
        Ok(AppKeyLinkRequestApply::Current)
    }
}

/// Apply a signed roster delivered over app-key-link/FIPS.
///
/// A brand-new linked device only accepts the first roster from the admin it
/// explicitly requested approval from. Once it has a current roster, it must
/// continue accepting newer rosters signed by a current admin so it learns
/// about devices approved after itself.
pub fn apply_app_key_link_roster_frame(
    config: &mut AppConfig,
    frame: &AppKeyLinkRosterFrame,
    admin_app_key_pubkey: &str,
) -> Result<AppKeyLinkRosterApply, RelayError> {
    if frame.schema != 1 {
        return Ok(AppKeyLinkRosterApply::Ignored);
    }
    let Some(account) = config.profile.as_ref() else {
        return Err(RelayError::NoAccount);
    };
    if frame.admin_app_key_pubkey != admin_app_key_pubkey {
        return Ok(AppKeyLinkRosterApply::Ignored);
    }
    if !account.profile_roster_ops.is_empty() && account.profile_id != frame.profile_id {
        return Ok(AppKeyLinkRosterApply::Ignored);
    }

    let incoming_ops = verified_profile_roster_ops(frame.profile_id, &frame.profile_roster_ops)?;
    let incoming_projection = project_nostr_identity_roster(frame.profile_id, incoming_ops.clone());
    let incoming_app_keys = app_keys_from_profile_projection(&incoming_projection)
        .ok_or_else(|| RelayError::AppKeyLinkRoster("profile roster has no AppKey epoch".into()))?;
    if !incoming_app_keys.is_admin(admin_app_key_pubkey) {
        return Ok(AppKeyLinkRosterApply::Ignored);
    }

    let has_current_roster = account.app_keys.is_some() || !account.profile_roster_ops.is_empty();
    let pending_request = account.outbound_app_key_link_request.as_ref();
    let pending_from_admin = pending_request.is_some_and(|pending| {
        !pending.admin_app_key_pubkey.trim().is_empty()
            && pending.admin_app_key_pubkey == admin_app_key_pubkey
    });
    let pending_unbound_manual_join =
        pending_request.is_some_and(|pending| pending.admin_app_key_pubkey.trim().is_empty());
    let pending_allows_first_roster = pending_from_admin || pending_unbound_manual_join;
    if !has_current_roster && !pending_allows_first_roster {
        return Ok(AppKeyLinkRosterApply::Ignored);
    }
    if pending_allows_first_roster && !incoming_projection.can_write_roots(&account.app_key_pubkey)
    {
        return Ok(AppKeyLinkRosterApply::Ignored);
    }

    let merged_ops = if account.profile_id == frame.profile_id {
        merge_profile_roster_ops(&account.profile_roster_ops, &incoming_ops)
    } else {
        incoming_ops
    };
    let ops_changed = account.profile_id != frame.profile_id
        || !same_profile_ops(&account.profile_roster_ops, &merged_ops);
    let merged_projection = project_nostr_identity_roster(frame.profile_id, merged_ops.clone());
    let merged_app_keys = app_keys_from_profile_projection(&merged_projection)
        .ok_or_else(|| RelayError::AppKeyLinkRoster("profile roster has no AppKey epoch".into()))?;

    if !ops_changed
        && account.app_keys.as_ref().is_some_and(|current| {
            app_keys_projection_eq_ignoring_labels(current, &merged_app_keys)
        })
    {
        return Ok(AppKeyLinkRosterApply::Current);
    }

    let decision = app_key_link_roster_apply_decision(
        account.app_keys.as_ref(),
        &merged_app_keys,
        ops_changed,
        !account.profile_roster_ops.is_empty(),
    );
    if decision == ApplyDecision::Rejected {
        return Ok(AppKeyLinkRosterApply::Applied(decision));
    }

    let root_scope_id = {
        let Some(account) = config.profile.as_mut() else {
            return Err(RelayError::NoAccount);
        };
        account.profile_roster_ops = merged_ops;
        account.profile_id = frame.profile_id;
        account.sync_app_keys_from_profile();
        debug_assert!(account.app_keys.as_ref().is_some_and(|current| {
            app_keys_projection_eq_ignoring_labels(current, &merged_app_keys)
        }));
        account.root_scope_id()
    };
    config.sync_primary_drive_scope(root_scope_id);
    Ok(AppKeyLinkRosterApply::Applied(decision))
}

fn app_keys_projection_eq_ignoring_labels(
    left: &AppKeysProjection,
    right: &AppKeysProjection,
) -> bool {
    let mut left = left.clone();
    let mut right = right.clone();
    for actor in &mut left.app_actors {
        actor.label = None;
    }
    for actor in &mut right.app_actors {
        actor.label = None;
    }
    left == right
}

fn app_key_link_roster_apply_decision(
    current: Option<&AppKeysProjection>,
    merged: &AppKeysProjection,
    ops_changed: bool,
    had_profile_ops: bool,
) -> ApplyDecision {
    let Some(current) = current else {
        return ApplyDecision::Adopted;
    };
    if current.profile_id != merged.profile_id {
        return ApplyDecision::Rejected;
    }
    match merged.created_at.cmp(&current.created_at) {
        std::cmp::Ordering::Greater => ApplyDecision::Replaced,
        std::cmp::Ordering::Equal => ApplyDecision::Merged,
        std::cmp::Ordering::Less if ops_changed && had_profile_ops => ApplyDecision::Merged,
        std::cmp::Ordering::Less => ApplyDecision::Rejected,
    }
}

/// Apply a signed `NostrIdentity` roster-op event to the local profile log.
///
/// The op log stores same-profile, signature-valid ops even when the current
/// projection rejects them. That keeps out-of-order delivery mergeable: once a
/// missing parent/add op arrives, deterministic projection can accept the
/// previously rejected op without needing the network to resend it.
pub fn apply_remote_nostr_identity_roster_op_event(
    config: &mut AppConfig,
    event: &Event,
) -> Result<NostrIdentityRosterOpApply, RelayError> {
    let op = parse_nostr_identity_roster_op_event(event)?;
    let Some(account) = config.profile.as_ref() else {
        return Err(RelayError::NoAccount);
    };
    if op.content.profile_id == account.profile_id {
        if account
            .profile_roster_ops
            .iter()
            .any(|current| current.op_id == op.op_id)
        {
            return Ok(NostrIdentityRosterOpApply::Current);
        }

        let root_scope_id = {
            let Some(account) = config.profile.as_mut() else {
                return Err(RelayError::NoAccount);
            };
            account.profile_roster_ops =
                merge_profile_roster_ops(&account.profile_roster_ops, &[op]);
            account.sync_app_keys_from_profile();
            account.recompute_authorization();
            account.root_scope_id()
        };
        config.sync_primary_drive_scope(root_scope_id);
        return Ok(NostrIdentityRosterOpApply::Applied);
    }

    Ok(NostrIdentityRosterOpApply::NotOurProfile)
}

pub fn apply_remote_share_access_snapshot_event(
    config: &mut AppConfig,
    event: &Event,
) -> Result<ShareAccessSnapshotApply, RelayError> {
    let snapshot = parse_share_access_snapshot_event(event)?;
    let Some(shared_folder) = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == snapshot.content.resource_id)
    else {
        return Ok(ShareAccessSnapshotApply::NotOurShare);
    };
    if snapshot.content.updated_at < shared_folder.access.updated_at
        || snapshot.content == shared_folder.access
    {
        return Ok(ShareAccessSnapshotApply::Current);
    }
    if !crate::shared_folder_app_key_can_admin(shared_folder, &snapshot.signer_pubkey) {
        return Ok(ShareAccessSnapshotApply::NotAdmin);
    }
    shared_folder.access = snapshot.content;
    Ok(ShareAccessSnapshotApply::Applied)
}

fn verified_profile_roster_ops(
    profile_id: crate::NostrIdentityId,
    ops: &[SignedNostrIdentityRosterOp],
) -> Result<Vec<SignedNostrIdentityRosterOp>, RelayError> {
    let mut by_id = BTreeMap::new();
    for op in ops {
        let event = Event::from_json(&op.event_json).map_err(|error| {
            RelayError::AppKeyLinkRoster(format!("parsing profile roster op event: {error}"))
        })?;
        let parsed = crate::parse_nostr_identity_roster_op_event(&event)?;
        if parsed.content.profile_id != profile_id {
            return Err(RelayError::AppKeyLinkRoster(format!(
                "profile roster op {} belongs to {}, expected {profile_id}",
                parsed.op_id, parsed.content.profile_id
            )));
        }
        by_id.insert(parsed.op_id.clone(), parsed);
    }
    Ok(by_id.into_values().collect())
}

fn same_profile_ops(
    left: &[SignedNostrIdentityRosterOp],
    right: &[SignedNostrIdentityRosterOp],
) -> bool {
    let left_ids = left
        .iter()
        .map(|op| op.op_id.as_str())
        .collect::<BTreeSet<_>>();
    let right_ids = right
        .iter()
        .map(|op| op.op_id.as_str())
        .collect::<BTreeSet<_>>();
    left_ids == right_ids
}

fn merge_profile_roster_ops(
    current: &[SignedNostrIdentityRosterOp],
    incoming: &[SignedNostrIdentityRosterOp],
) -> Vec<SignedNostrIdentityRosterOp> {
    let mut by_id = BTreeMap::new();
    for op in current.iter().chain(incoming.iter()) {
        by_id.insert(op.op_id.clone(), op.clone());
    }
    by_id.into_values().collect()
}

/// Result of applying a remote drive-root event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveRootApply {
    /// Root scope id in the d-tag is neither our profile nor a known share.
    NotOurScope,
    /// Drive id in the d-tag isn't configured locally — ignored.
    UnknownDrive,
    /// `AppKey` pubkey isn't in the current `AppKeys` roster — ignored.
    /// Protects against forged events from unauthorized app actors.
    UnauthorizedAppKey,
    /// Older than what we already have for this device — ignored.
    /// Causal roots compare by `app_key_seq`; legacy roots compare by
    /// timestamp.
    StaleTimestamp,
    /// The event is for an authorized device, but this local device has not
    /// received a DCK wrap that can decrypt it yet.
    KeyUnavailable,
    /// Applied; the device's root entry was updated/inserted.
    Applied,
}

/// Result of applying a web-compatible hashtree root event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilesRootApply {
    /// Event author doesn't match our current `AppKey`.
    NotOurAppKey,
    /// The tree name does not match a configured drive id.
    UnknownDrive,
    /// Older than the local root we already mapped to this device.
    StaleTimestamp,
    /// Applied as this device's current root.
    Applied,
}

/// Apply a remote drive-root event to `config`. Drops events for foreign root
/// scopes, unknown drives, unauthorized app actors, or roots older than what's
/// already recorded.
pub fn apply_remote_drive_root_event(
    config: &mut AppConfig,
    event: &Event,
    device_keys: Option<&Keys>,
) -> Result<DriveRootApply, RelayError> {
    let preview = parse_drive_root_event_preview(event)?;
    let app_key_hex = preview.app_key_pubkey_hex.clone();
    let Some(account) = config.profile.as_ref() else {
        return Err(RelayError::NoAccount);
    };
    if preview.root_scope_id == account.root_scope_id() {
        let Some(drive_index) = config
            .drives
            .iter()
            .position(|drive| drive.drive_id == preview.drive_id)
        else {
            return Ok(DriveRootApply::UnknownDrive);
        };
        if !crate::drive_root_app_key_can_write_roots(
            account,
            &config.drives[drive_index],
            &app_key_hex,
        ) {
            return Ok(DriveRootApply::UnauthorizedAppKey);
        }
        let drive = &mut config.drives[drive_index];
        return apply_root_to_app_key_roots(&mut drive.app_key_roots, event, device_keys, &preview);
    }

    let Ok(share_id) = preview.root_scope_id.parse::<NostrIdentityId>() else {
        return Ok(DriveRootApply::NotOurScope);
    };
    if preview.drive_id != crate::PRIMARY_DRIVE_ID {
        return Ok(DriveRootApply::UnknownDrive);
    }
    let Some(shared_folder) = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == share_id)
    else {
        return Ok(DriveRootApply::NotOurScope);
    };
    let authorization =
        crate::shared_folder_app_key_write_authorization(shared_folder, &app_key_hex);
    if !authorization.is_authorized() {
        return Ok(DriveRootApply::UnauthorizedAppKey);
    }
    apply_root_to_app_key_roots(
        &mut shared_folder.app_key_roots,
        event,
        device_keys,
        &preview,
    )
}

fn apply_root_to_app_key_roots(
    app_key_roots: &mut BTreeMap<String, AppKeyRootRef>,
    event: &Event,
    device_keys: Option<&Keys>,
    preview: &crate::nostr_events::DriveRootEventPreview,
) -> Result<DriveRootApply, RelayError> {
    let app_key_hex = preview.app_key_pubkey_hex.clone();
    if let Some(existing) = app_key_roots.get(&app_key_hex)
        && incoming_root_is_stale(existing, preview.app_key_seq, preview.published_at)
    {
        return Ok(DriveRootApply::StaleTimestamp);
    }
    let (_, _, _, incoming_root) = if let Some(keys) = device_keys {
        match parse_drive_root_event_for_device(event, keys) {
            Ok(parsed) => parsed,
            Err(crate::nostr_events::WireError::RootKeyUnavailable) => {
                return Ok(DriveRootApply::KeyUnavailable);
            }
            Err(error) => return Err(error.into()),
        }
    } else {
        parse_drive_root_event(event)?
    };
    if app_key_roots
        .get(&app_key_hex)
        .is_some_and(|existing| existing.root_cid == incoming_root.root_cid)
    {
        return Ok(DriveRootApply::StaleTimestamp);
    }
    app_key_roots.insert(app_key_hex, incoming_root);
    Ok(DriveRootApply::Applied)
}

/// Apply a standard web hashtree root event to the local primary `AppKey` root.
///
/// Web Iris apps publish one signer-scoped mutable root per tree. Native Iris
/// Drive stores roots per authorized `AppKey`, so this bridge imports a
/// current-`AppKey` web root as that `AppKey`'s native contribution. Native
/// drive-root events remain the richer multi-AppKey protocol.
pub fn apply_remote_files_root_event(
    config: &mut AppConfig,
    event: &Event,
    local_keys: Option<&Keys>,
) -> Result<FilesRootApply, RelayError> {
    let parsed = hashtree_nostr::parse_verified_hashtree_root_event(event)
        .map_err(|e| RelayError::HashtreeRoot(e.to_string()))?
        .ok_or_else(|| RelayError::HashtreeRoot("not a hashtree root event".to_string()))?;
    let Some(account) = config.profile.as_ref() else {
        return Err(RelayError::NoAccount);
    };
    if parsed.event.pubkey != account.app_key_pubkey {
        return Ok(FilesRootApply::NotOurAppKey);
    }
    let root_cid = if let Some(local_keys) = local_keys {
        hashtree_nostr::resolve_self_encrypted_root_cid(&parsed, local_keys)
            .map_err(|e| RelayError::HashtreeRoot(e.to_string()))?
    } else {
        parsed.root_cid.clone()
    };
    if root_cid.key.is_none() {
        return Err(RelayError::HashtreeRoot(
            "hashtree root key is unavailable".to_string(),
        ));
    }
    let incoming_root = AppKeyRootRef {
        root_cid: root_cid.to_string(),
        published_at: i64::try_from(parsed.event.created_at).unwrap_or(i64::MAX),
        dck_generation: 0,
        app_key_seq: 0,
        parents: Vec::new(),
        observed: std::collections::BTreeMap::new(),
        local_only: false,
    };
    let app_key_pubkey = account.app_key_pubkey.clone();
    if config.drive(&parsed.tree_name).is_none() {
        if parsed.tree_name != CALENDAR_TREE_NAME {
            return Ok(FilesRootApply::UnknownDrive);
        }
        config.upsert_drive(Drive {
            root_scope_id: account.root_scope_id(),
            drive_id: CALENDAR_TREE_NAME.into(),
            display_name: "Calendar".into(),
            role: DriveRole::Owner,
            app_key_roots: BTreeMap::new(),
            last_root_cid: None,
            key_hex: None,
        });
    }
    let Some(drive) = config
        .drives
        .iter_mut()
        .find(|d| d.drive_id == parsed.tree_name)
    else {
        return Ok(FilesRootApply::UnknownDrive);
    };
    if let Some(existing) = drive.app_key_roots.get(&app_key_pubkey) {
        if existing.root_cid == incoming_root.root_cid {
            return Ok(FilesRootApply::StaleTimestamp);
        }
        if existing.app_key_seq > 0 {
            return Ok(FilesRootApply::StaleTimestamp);
        }
        if existing.published_at >= incoming_root.published_at {
            return Ok(FilesRootApply::StaleTimestamp);
        }
    }
    drive.last_root_cid = Some(incoming_root.root_cid.clone());
    drive.app_key_roots.insert(app_key_pubkey, incoming_root);
    Ok(FilesRootApply::Applied)
}

fn incoming_root_is_stale(
    existing: &AppKeyRootRef,
    incoming_app_key_seq: u64,
    incoming_published_at: i64,
) -> bool {
    if existing.app_key_seq > 0 || incoming_app_key_seq > 0 {
        if incoming_app_key_seq == 0 {
            return true;
        }
        if existing.app_key_seq == 0 {
            return false;
        }
        return incoming_app_key_seq <= existing.app_key_seq;
    }
    existing.published_at >= incoming_published_at
}

// ----- Live relay layer -----

/// Connect a fresh client to the given relays. Caller manages the
/// client's lifecycle (disconnect when done).
pub async fn connect(relay_urls: &[String]) -> Result<Client, RelayError> {
    let source_routes = relay_source_routes(relay_urls);
    let relay_urls = relay_urls_from_source_routes(&source_routes);
    let client = Client::builder().opts(ClientOptions::new()).build();
    for url in &relay_urls {
        client
            .add_relay(url)
            .await
            .map_err(|e| RelayError::Client(format!("add_relay {url}: {e}")))?;
    }
    client.connect().await;
    Ok(client)
}

pub async fn shutdown_client(client: &Client) {
    client.shutdown().await;
}

/// Shut down relay tasks before daemon process exit and keep one handle alive.
///
/// nostr-relay-pool 0.44 performs async cleanup from `Drop`; if the last cloned
/// client disappears while the Tokio runtime is unwinding, that destructor path
/// can abort the helper process. The daemon is exiting anyway, so keeping one
/// already-shutdown handle alive until process teardown is preferable to a crash
/// report on normal app/parent shutdown.
pub async fn shutdown_client_for_process_exit(client: Client) {
    client.shutdown().await;
    std::mem::forget(client);
}

/// Publish a signed app-key-link request from the requesting `AppKey`.
pub async fn publish_app_key_link_request(
    client: &Client,
    device_keys: &Keys,
    frame: &crate::app_key_link_transport::AppKeyLinkRequestFrame,
) -> Result<nostr_sdk::EventId, RelayError> {
    let event = build_app_key_link_request_event(device_keys, frame)?;
    let output = client
        .send_event(&event)
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
    Ok(*output.id())
}

/// Publish the signed `NostrIdentity` roster op log.
pub async fn publish_nostr_identity_roster_ops(
    client: &Client,
    ops: &[SignedNostrIdentityRosterOp],
) -> Result<Vec<nostr_sdk::EventId>, RelayError> {
    let mut event_ids = Vec::with_capacity(ops.len());
    for op in ops {
        let event = Event::from_json(&op.event_json)
            .map_err(|e| RelayError::Client(format!("profile roster op JSON: {e}")))?;
        let parsed = parse_nostr_identity_roster_op_event(&event)?;
        if parsed.op_id != op.op_id {
            return Err(RelayError::Client(format!(
                "profile roster op id mismatch: stored {}, parsed {}",
                op.op_id, parsed.op_id
            )));
        }
        let output = client
            .send_event(&event)
            .await
            .map_err(|e| RelayError::Client(e.to_string()))?;
        event_ids.push(*output.id());
    }
    Ok(event_ids)
}

/// Publish a signed canonical share access snapshot.
pub async fn publish_share_access_snapshot(
    client: &Client,
    snapshot: &SignedShareAccessSnapshot,
) -> Result<nostr_sdk::EventId, RelayError> {
    let event = Event::from_json(&snapshot.event_json)
        .map_err(|e| RelayError::Client(format!("share access snapshot JSON: {e}")))?;
    let parsed = parse_share_access_snapshot_event(&event)?;
    if parsed.snapshot_id != snapshot.snapshot_id {
        return Err(RelayError::Client(format!(
            "share access snapshot id mismatch: stored {}, parsed {}",
            snapshot.snapshot_id, parsed.snapshot_id
        )));
    }
    let output = client
        .send_event(&event)
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
    Ok(*output.id())
}

/// Publish a signed drive-root event for this device's current root.
pub async fn publish_drive_root(
    client: &Client,
    device_keys: &Keys,
    root_scope_id: &str,
    drive_id: &str,
    root: &AppKeyRootRef,
    authorized_app_key_pubkeys: &[String],
) -> Result<nostr_sdk::EventId, RelayError> {
    let event = build_drive_root_publish_event(
        device_keys,
        root_scope_id,
        drive_id,
        root,
        authorized_app_key_pubkeys,
    )?;
    let output = client
        .send_event(&event)
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
    Ok(*output.id())
}

/// Publish the current-AppKey private drive.iris.to-compatible mutable tree root.
pub async fn publish_files_root(
    client: &Client,
    app_key: &Keys,
    tree_name: &str,
    root: &AppKeyRootRef,
) -> Result<nostr_sdk::EventId, RelayError> {
    let event = build_private_hashtree_root_event(app_key, tree_name, root)?;
    let output = client
        .send_event(&event)
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
    Ok(*output.id())
}

async fn fetch_events(
    client: &Client,
    filters: Vec<Filter>,
    timeout: Duration,
) -> Result<Vec<Event>, RelayError> {
    let policy = event_retention_policy(filters);
    let mut events = Vec::new();
    for filter in policy.filters.clone() {
        for event in client
            .fetch_events(filter, timeout)
            .await
            .map_err(|e| RelayError::Client(e.to_string()))?
        {
            if !relay_event_matches_policy(&policy, &event) {
                continue;
            }
            events.push(event);
            if events.len() >= policy.max_events {
                return Ok(events);
            }
        }
    }
    Ok(events)
}

/// Fetch the latest standard hashtree root for `app_key_pubkey_hex/tree_name`.
pub async fn fetch_latest_files_root(
    client: &Client,
    app_key_pubkey_hex: &str,
    tree_name: &str,
    timeout: Duration,
) -> Result<Option<Event>, RelayError> {
    let app_key = PublicKey::from_hex(app_key_pubkey_hex)
        .map_err(|e| RelayError::InvalidPubkey(e.to_string()))?;
    let filter = Filter::new()
        .author(app_key)
        .kind(nostr_sdk::Kind::from(KIND_HASHTREE_ROOT))
        .identifier(tree_name)
        .custom_tag(
            SingleLetterTag::lowercase(nostr_sdk::Alphabet::L),
            hashtree_nostr::HASHTREE_LABEL,
        );
    let events = fetch_events(client, vec![filter], timeout).await?;
    let latest = events.into_iter().max_by_key(|e| e.created_at.as_secs());
    Ok(latest)
}

/// Fetch all visible `NostrIdentity` roster ops for a profile.
pub async fn fetch_nostr_identity_roster_ops(
    client: &Client,
    profile_id: NostrIdentityId,
    timeout: Duration,
) -> Result<Vec<Event>, RelayError> {
    let events = fetch_events(
        client,
        vec![nostr_identity_roster_op_filter(profile_id)],
        timeout,
    )
    .await?;
    Ok(events
        .into_iter()
        .filter(|event| {
            parse_nostr_identity_roster_op_event(event)
                .is_ok_and(|op| op.content.profile_id == profile_id)
        })
        .collect())
}

/// Fetch signed canonical share access snapshots for one share.
pub async fn fetch_share_access_snapshots(
    client: &Client,
    share_id: NostrIdentityId,
    timeout: Duration,
) -> Result<Vec<Event>, RelayError> {
    let events = fetch_events(
        client,
        vec![share_access_snapshot_filter(share_id)],
        timeout,
    )
    .await?;
    Ok(events
        .into_iter()
        .filter(|event| {
            parse_share_access_snapshot_event(event)
                .is_ok_and(|snapshot| snapshot.content.resource_id == share_id)
        })
        .collect())
}

/// Relay filters for finding `NostrIdentity` evidence involving a recovery key.
///
/// The `#p` filter catches roster ops that mention the key and self-signed
/// acceptance breadcrumbs. The author filter catches events signed by the key.
/// Matching events are discovery hints; callers must still fetch/project the
/// profile roster log before trusting them.
pub fn nostr_identity_restore_candidate_filters(
    recovery_pubkey_hex: &str,
) -> Result<Vec<Filter>, RelayError> {
    let recovery_pubkey = PublicKey::from_hex(recovery_pubkey_hex)
        .map_err(|e| RelayError::InvalidPubkey(e.to_string()))?;
    Ok(vec![
        Filter::new()
            .kind(nostr_sdk::Kind::from(KIND_NOSTR_IDENTITY_ROSTER_OP))
            .pubkey(recovery_pubkey),
        Filter::new()
            .kind(nostr_sdk::Kind::from(KIND_NOSTR_IDENTITY_ROSTER_OP))
            .author(recovery_pubkey),
    ])
}

/// Project fetched relay events into verified restore candidates for a
/// recovery/NIP-46 pubkey. Acceptance events and `p` tags only discover
/// candidate UUIDs; a candidate is returned only when the authoritative roster
/// projection has the pubkey as an active facet that can recover `AppKeys`.
pub fn nostr_identity_restore_candidates_from_events(
    recovery_pubkey_hex: &str,
    events: &[Event],
) -> Result<Vec<NostrIdentityRestoreCandidate>, RelayError> {
    let candidate_ids: BTreeSet<_> =
        nostr_identity_candidate_ids_for_pubkey_from_events(recovery_pubkey_hex, events)?
            .into_iter()
            .collect();
    let mut roster_ops_by_profile =
        BTreeMap::<NostrIdentityId, BTreeMap<String, SignedNostrIdentityRosterOp>>::new();
    for event in events {
        let Ok(op) = parse_nostr_identity_roster_op_event(event) else {
            continue;
        };
        if candidate_ids.contains(&op.content.profile_id) {
            roster_ops_by_profile
                .entry(op.content.profile_id)
                .or_default()
                .insert(op.op_id.clone(), op);
        }
    }

    let mut candidates = Vec::new();
    for profile_id in candidate_ids {
        let profile_roster_ops = roster_ops_by_profile
            .remove(&profile_id)
            .unwrap_or_default()
            .into_values()
            .collect::<Vec<_>>();
        let projection = project_nostr_identity_roster(profile_id, profile_roster_ops.clone());
        let Some(facet) = projection.active_facets.get(recovery_pubkey_hex) else {
            continue;
        };
        if !facet.capabilities.can_recover_app_keys {
            continue;
        }
        let accepted_op_ids = projection
            .accepted_op_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let latest_roster_op_created_at = profile_roster_ops
            .iter()
            .filter(|op| accepted_op_ids.contains(op.op_id.as_str()))
            .map(|op| op.content.created_at)
            .max();
        candidates.push(NostrIdentityRestoreCandidate {
            profile_id,
            recovery_pubkey: recovery_pubkey_hex.to_string(),
            can_decrypt_secret_epochs: facet.capabilities.can_decrypt_secret_epochs,
            accepted_roster_op_count: projection.accepted_op_ids.len(),
            active_app_key_count: projection.active_app_key_pubkeys().len(),
            latest_roster_op_created_at,
            profile_roster_ops,
        });
    }
    candidates.sort_by(|left, right| {
        right
            .latest_roster_op_created_at
            .cmp(&left.latest_roster_op_created_at)
            .then_with(|| {
                right
                    .accepted_roster_op_count
                    .cmp(&left.accepted_roster_op_count)
            })
            .then_with(|| right.active_app_key_count.cmp(&left.active_app_key_count))
            .then_with(|| left.profile_id.cmp(&right.profile_id))
    });
    Ok(candidates)
}

/// Fetch restore candidates for a recovery/NIP-46 pubkey from relays.
///
/// This is a two-step query: first find profile IDs through events that mention
/// or are authored by the key, then fetch the full roster op log for each
/// discovered profile ID and project the logs locally.
pub async fn fetch_nostr_identity_restore_candidates(
    client: &Client,
    recovery_pubkey_hex: &str,
    timeout: Duration,
) -> Result<Vec<NostrIdentityRestoreCandidate>, RelayError> {
    let discovery_events = fetch_events(
        client,
        nostr_identity_restore_candidate_filters(recovery_pubkey_hex)?,
        timeout,
    )
    .await?;
    let candidate_ids = nostr_identity_candidate_ids_for_pubkey_from_events(
        recovery_pubkey_hex,
        &discovery_events,
    )?;
    if candidate_ids.is_empty() {
        return Ok(Vec::new());
    }
    let roster_events = fetch_events(
        client,
        candidate_ids
            .into_iter()
            .map(nostr_identity_roster_op_filter)
            .collect::<Vec<_>>(),
        timeout,
    )
    .await?;
    let mut events = discovery_events;
    events.extend(roster_events);
    nostr_identity_restore_candidates_from_events(recovery_pubkey_hex, &events)
}

/// Fetch drive-root events from any of `authorized_app_keys` for
/// `(root_scope_id, drive_id)`. Returns the latest event from each
/// device (one event per device).
pub async fn fetch_drive_roots(
    client: &Client,
    root_scope_id: &str,
    drive_id: &str,
    authorized_app_keys: &[String],
    timeout: Duration,
) -> Result<Vec<Event>, RelayError> {
    if authorized_app_keys.is_empty() {
        return Ok(Vec::new());
    }
    let mut authors = Vec::with_capacity(authorized_app_keys.len());
    for hex in authorized_app_keys {
        authors
            .push(PublicKey::from_hex(hex).map_err(|e| RelayError::InvalidPubkey(e.to_string()))?);
    }
    let d_tag = drive_root_d_tag(root_scope_id, drive_id);
    let filter = Filter::new()
        .authors(authors)
        .kind(nostr_sdk::Kind::from(KIND_DRIVE_ROOT))
        .custom_tag(SingleLetterTag::lowercase(nostr_sdk::Alphabet::D), d_tag);
    let events = fetch_events(client, vec![filter], timeout).await?;
    // Pick the latest root per author. Device roots carry a monotonic
    // per-device sequence; use it before wall-clock timestamps so two publishes
    // in the same second cannot make us fetch an older snapshot.
    let mut latest_per_author: std::collections::HashMap<PublicKey, Event> =
        std::collections::HashMap::new();
    for ev in events {
        latest_per_author
            .entry(ev.pubkey)
            .and_modify(|cur| {
                if drive_root_event_is_newer(&ev, cur) {
                    *cur = ev.clone();
                }
            })
            .or_insert(ev);
    }
    Ok(latest_per_author.into_values().collect())
}

fn drive_root_event_is_newer(candidate: &Event, current: &Event) -> bool {
    match (
        parse_drive_root_event_preview(candidate),
        parse_drive_root_event_preview(current),
    ) {
        (Ok(candidate), Ok(current)) => {
            let same_app_key = candidate.app_key_pubkey_hex == current.app_key_pubkey_hex;
            let seq_order = if same_app_key {
                candidate.app_key_seq.cmp(&current.app_key_seq)
            } else {
                std::cmp::Ordering::Equal
            };
            seq_order
                .then_with(|| candidate.published_at.cmp(&current.published_at))
                .then_with(|| {
                    drive_root_preview_ms(&candidate).cmp(&drive_root_preview_ms(&current))
                })
                .then_with(|| candidate.dck_generation.cmp(&current.dck_generation))
                .then_with(|| {
                    candidate
                        .app_key_pubkey_hex
                        .cmp(&current.app_key_pubkey_hex)
                })
                .is_gt()
        }
        _ => candidate.created_at.as_secs() > current.created_at.as_secs(),
    }
}

fn drive_root_preview_ms(preview: &crate::nostr_events::DriveRootEventPreview) -> u64 {
    preview.published_at_ms.unwrap_or_else(|| {
        u64::try_from(preview.published_at)
            .unwrap_or(0)
            .saturating_mul(1000)
    })
}

#[cfg(test)]
#[path = "relay_sync/app_key_approval_candidate_tests.rs"]
mod app_key_approval_candidate_tests;
#[cfg(test)]
mod calendar_tests;
#[cfg(test)]
mod tests;
