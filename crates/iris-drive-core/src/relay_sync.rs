//! Relay-layer sync: publish + fetch + apply.
//!
//! Two layers:
//!
//! - **Apply (offline)** — `apply_remote_iris_profile_roster_op_event`,
//!   `apply_remote_drive_root_event`, and app-key-link helpers take a parsed
//!   Nostr event or direct roster frame plus an `AppConfig` and apply the
//!   event's effect onto the config. These are pure functions over data, fully
//!   covered by unit tests.
//!
//! - **Network (live)** — `publish_iris_profile_roster_ops`,
//!   `publish_drive_root`, `fetch_iris_profile_roster_ops`,
//!   `fetch_iris_profile_restore_candidates`, and `fetch_drive_roots` wrap
//!   nostr-sdk's `Client` for actual relay I/O. Tested manually against real
//!   relays; the wire/apply layers below them are what we cover automatically.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use nostr_sdk::{Client, Event, Filter, JsonUtil, Keys, Options, PublicKey, SingleLetterTag};
use thiserror::Error;

use crate::app_key_link_transport::AppKeyLinkRosterFrame;
use crate::app_keys::{AppKeysProjection, ApplyDecision};
use crate::config::{AppConfig, AppKeyRootRef};
use crate::nostr_events::{
    KIND_APP_KEY_LINK_REQUEST, KIND_DRIVE_ROOT, KIND_HASHTREE_ROOT, app_key_link_request_d_tag,
    build_app_key_link_request_event, build_drive_root_publish_event,
    build_private_hashtree_root_event, drive_root_d_tag, parse_app_key_link_request_event,
    parse_drive_root_event, parse_drive_root_event_for_device, parse_drive_root_event_preview,
};
use crate::profile::app_keys_from_profile_projection;
use crate::{
    IrisProfileId, KIND_IRIS_PROFILE_ROSTER_OP, SignedIrisProfileRosterOp,
    iris_profile_candidate_ids_for_pubkey_from_events, parse_iris_profile_roster_op_event,
    project_iris_profile_roster,
};

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
    IrisProfile(#[from] crate::iris_profile::IrisProfileError),
    #[error("app-key-link roster: {0}")]
    AppKeyLinkRoster(String),
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

/// Result of merging a signed `IrisProfile` roster op from relay/direct sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IrisProfileRosterOpApply {
    /// The op belongs to another profile.
    NotOurProfile,
    /// This op id is already present locally.
    Current,
    /// The verified op was unioned into the local profile log.
    Applied,
}

/// Verified roster evidence for an `IrisProfile` that a recovery/NIP-46
/// pubkey can use to admit a fresh local `AppKey`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrisProfileRestoreCandidate {
    pub profile_id: IrisProfileId,
    pub recovery_pubkey: String,
    pub can_decrypt_key_epochs: bool,
    pub accepted_roster_op_count: usize,
    pub active_app_key_count: usize,
    pub latest_roster_op_created_at: Option<i64>,
    pub profile_roster_ops: Vec<SignedIrisProfileRosterOp>,
}

/// Result of applying an app-key-link request sent over relay metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppKeyLinkRequestApply {
    /// The event is addressed to another profile.
    NotOurProfile,
    /// This install's `AppKey` is not an admin and cannot approve `AppKeys`.
    NotAdmin,
    /// The event did not carry this admin's current invite secret.
    InvalidSecret,
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
    let frame = parse_app_key_link_request_event(event)?;
    let Some(account) = config.profile.as_mut() else {
        return Err(RelayError::NoAccount);
    };
    if frame.profile_id != account.profile_id {
        return Ok(AppKeyLinkRequestApply::NotOurProfile);
    }
    if !account.can_admin_profile() {
        return Ok(AppKeyLinkRequestApply::NotAdmin);
    }
    let expected_secret = account.app_key_link_secret.trim();
    if !expected_secret.is_empty() && frame.link_secret.trim() != expected_secret {
        return Ok(AppKeyLinkRequestApply::InvalidSecret);
    }

    let changed = account.record_inbound_app_key_link_request(
        frame.profile_id,
        &frame.app_key_pubkey,
        frame.label,
        &frame.link_secret,
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
    let incoming_projection = project_iris_profile_roster(frame.profile_id, incoming_ops.clone());
    let incoming_app_keys = app_keys_from_profile_projection(&incoming_projection)
        .ok_or_else(|| RelayError::AppKeyLinkRoster("profile roster has no AppKey epoch".into()))?;
    if !incoming_app_keys.is_admin(admin_app_key_pubkey) {
        return Ok(AppKeyLinkRosterApply::Ignored);
    }

    let has_current_roster = account.app_keys.is_some() || !account.profile_roster_ops.is_empty();
    let pending_from_admin = account
        .outbound_app_key_link_request
        .as_ref()
        .is_some_and(|pending| pending.admin_app_key_pubkey == admin_app_key_pubkey);
    if !has_current_roster && !pending_from_admin {
        return Ok(AppKeyLinkRosterApply::Ignored);
    }
    if pending_from_admin && !incoming_projection.can_write_roots(&account.app_key_pubkey) {
        return Ok(AppKeyLinkRosterApply::Ignored);
    }

    let merged_ops = if account.profile_id == frame.profile_id {
        merge_profile_roster_ops(&account.profile_roster_ops, &incoming_ops)
    } else {
        incoming_ops
    };
    let ops_changed = account.profile_id != frame.profile_id
        || !same_profile_ops(&account.profile_roster_ops, &merged_ops);
    let merged_projection = project_iris_profile_roster(frame.profile_id, merged_ops.clone());
    let merged_app_keys = app_keys_from_profile_projection(&merged_projection)
        .ok_or_else(|| RelayError::AppKeyLinkRoster("profile roster has no AppKey epoch".into()))?;

    if !ops_changed && account.app_keys.as_ref() == Some(&merged_app_keys) {
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
        debug_assert_eq!(account.app_keys.as_ref(), Some(&merged_app_keys));
        account.root_scope_id()
    };
    config.sync_primary_drive_scope(root_scope_id);
    Ok(AppKeyLinkRosterApply::Applied(decision))
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

/// Apply a signed `IrisProfile` roster-op event to the local profile log.
///
/// The op log stores same-profile, signature-valid ops even when the current
/// projection rejects them. That keeps out-of-order delivery mergeable: once a
/// missing parent/add op arrives, deterministic projection can accept the
/// previously rejected op without needing the network to resend it.
pub fn apply_remote_iris_profile_roster_op_event(
    config: &mut AppConfig,
    event: &Event,
) -> Result<IrisProfileRosterOpApply, RelayError> {
    let op = parse_iris_profile_roster_op_event(event)?;
    let Some(account) = config.profile.as_ref() else {
        return Err(RelayError::NoAccount);
    };
    if op.content.profile_id == account.profile_id {
        if account
            .profile_roster_ops
            .iter()
            .any(|current| current.op_id == op.op_id)
        {
            return Ok(IrisProfileRosterOpApply::Current);
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
        return Ok(IrisProfileRosterOpApply::Applied);
    }

    let Some(shared_folder) = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == op.content.profile_id)
    else {
        return Ok(IrisProfileRosterOpApply::NotOurProfile);
    };
    if shared_folder
        .roster_ops
        .iter()
        .any(|current| current.op_id == op.op_id)
    {
        return Ok(IrisProfileRosterOpApply::Current);
    }
    shared_folder.roster_ops = merge_profile_roster_ops(&shared_folder.roster_ops, &[op]);
    crate::refresh_shared_folder_member_statuses_from_roster(shared_folder);
    Ok(IrisProfileRosterOpApply::Applied)
}

fn verified_profile_roster_ops(
    profile_id: crate::IrisProfileId,
    ops: &[SignedIrisProfileRosterOp],
) -> Result<Vec<SignedIrisProfileRosterOp>, RelayError> {
    let mut by_id = BTreeMap::new();
    for op in ops {
        let event = Event::from_json(&op.event_json).map_err(|error| {
            RelayError::AppKeyLinkRoster(format!("parsing profile roster op event: {error}"))
        })?;
        let parsed = crate::parse_iris_profile_roster_op_event(&event)?;
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
    left: &[SignedIrisProfileRosterOp],
    right: &[SignedIrisProfileRosterOp],
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
    current: &[SignedIrisProfileRosterOp],
    incoming: &[SignedIrisProfileRosterOp],
) -> Vec<SignedIrisProfileRosterOp> {
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
        let authorized: BTreeSet<String> = account
            .app_keys
            .as_ref()
            .map(|s| s.app_actors.iter().map(|d| d.pubkey.clone()).collect())
            .unwrap_or_default();
        if !authorized.contains(&app_key_hex) {
            return Ok(DriveRootApply::UnauthorizedAppKey);
        }
        let Some(drive) = config
            .drives
            .iter_mut()
            .find(|d| d.drive_id == preview.drive_id)
        else {
            return Ok(DriveRootApply::UnknownDrive);
        };
        return apply_root_to_app_key_roots(&mut drive.app_key_roots, event, device_keys, &preview);
    }

    let Ok(share_id) = preview.root_scope_id.parse::<IrisProfileId>() else {
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
    let Some(account) = config.profile.as_ref() else {
        return Err(RelayError::NoAccount);
    };
    if parsed.event.pubkey != account.app_key_pubkey {
        return Ok(FilesRootApply::NotOurAppKey);
    }
    let app_key_pubkey = account.app_key_pubkey.clone();
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
    let client = Client::builder()
        .opts(Options::new().wait_for_send(false))
        .build();
    for url in relay_urls {
        client
            .add_relay(url)
            .await
            .map_err(|e| RelayError::Client(format!("add_relay {url}: {e}")))?;
    }
    client.connect().await;
    Ok(client)
}

/// Publish a signed app-key-link request from the requesting `AppKey`.
pub async fn publish_app_key_link_request(
    client: &Client,
    device_keys: &Keys,
    frame: &crate::app_key_link_transport::AppKeyLinkRequestFrame,
) -> Result<nostr_sdk::EventId, RelayError> {
    let event = build_app_key_link_request_event(device_keys, frame)?;
    let output = client
        .send_event(event)
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
    Ok(*output.id())
}

/// Publish the signed `IrisProfile` roster op log.
pub async fn publish_iris_profile_roster_ops(
    client: &Client,
    ops: &[SignedIrisProfileRosterOp],
) -> Result<Vec<nostr_sdk::EventId>, RelayError> {
    let mut event_ids = Vec::with_capacity(ops.len());
    for op in ops {
        let event = Event::from_json(&op.event_json)
            .map_err(|e| RelayError::Client(format!("profile roster op JSON: {e}")))?;
        let parsed = parse_iris_profile_roster_op_event(&event)?;
        if parsed.op_id != op.op_id {
            return Err(RelayError::Client(format!(
                "profile roster op id mismatch: stored {}, parsed {}",
                op.op_id, parsed.op_id
            )));
        }
        let output = client
            .send_event(event)
            .await
            .map_err(|e| RelayError::Client(e.to_string()))?;
        event_ids.push(*output.id());
    }
    Ok(event_ids)
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
        .send_event(event)
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
        .send_event(event)
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
    Ok(*output.id())
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
            [hashtree_nostr::HASHTREE_LABEL],
        );
    let events = client
        .get_events_of(vec![filter], nostr_sdk::EventSource::relays(Some(timeout)))
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
    let latest = events.into_iter().max_by_key(|e| e.created_at.as_u64());
    Ok(latest)
}

/// Fetch all visible `IrisProfile` roster ops for a profile.
pub async fn fetch_iris_profile_roster_ops(
    client: &Client,
    profile_id: IrisProfileId,
    timeout: Duration,
) -> Result<Vec<Event>, RelayError> {
    let events = client
        .get_events_of(
            vec![iris_profile_roster_op_filter(profile_id)],
            nostr_sdk::EventSource::relays(Some(timeout)),
        )
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
    Ok(events
        .into_iter()
        .filter(|event| {
            parse_iris_profile_roster_op_event(event)
                .is_ok_and(|op| op.content.profile_id == profile_id)
        })
        .collect())
}

/// Relay filters for finding `IrisProfile` evidence involving a recovery key.
///
/// The `#p` filter catches roster ops that mention the key and self-signed
/// acceptance breadcrumbs. The author filter catches events signed by the key.
/// Matching events are discovery hints; callers must still fetch/project the
/// profile roster log before trusting them.
pub fn iris_profile_restore_candidate_filters(
    recovery_pubkey_hex: &str,
) -> Result<Vec<Filter>, RelayError> {
    let recovery_pubkey = PublicKey::from_hex(recovery_pubkey_hex)
        .map_err(|e| RelayError::InvalidPubkey(e.to_string()))?;
    Ok(vec![
        Filter::new()
            .kind(nostr_sdk::Kind::from(KIND_IRIS_PROFILE_ROSTER_OP))
            .pubkey(recovery_pubkey),
        Filter::new()
            .kind(nostr_sdk::Kind::from(KIND_IRIS_PROFILE_ROSTER_OP))
            .author(recovery_pubkey),
    ])
}

/// Project fetched relay events into verified restore candidates for a
/// recovery/NIP-46 pubkey. Acceptance events and `p` tags only discover
/// candidate UUIDs; a candidate is returned only when the authoritative roster
/// projection has the pubkey as an active facet that can recover `AppKeys`.
pub fn iris_profile_restore_candidates_from_events(
    recovery_pubkey_hex: &str,
    events: &[Event],
) -> Result<Vec<IrisProfileRestoreCandidate>, RelayError> {
    let candidate_ids: BTreeSet<_> =
        iris_profile_candidate_ids_for_pubkey_from_events(recovery_pubkey_hex, events)?
            .into_iter()
            .collect();
    let mut roster_ops_by_profile =
        BTreeMap::<IrisProfileId, BTreeMap<String, SignedIrisProfileRosterOp>>::new();
    for event in events {
        let Ok(op) = parse_iris_profile_roster_op_event(event) else {
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
        let projection = project_iris_profile_roster(profile_id, profile_roster_ops.clone());
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
        candidates.push(IrisProfileRestoreCandidate {
            profile_id,
            recovery_pubkey: recovery_pubkey_hex.to_string(),
            can_decrypt_key_epochs: facet.capabilities.can_decrypt_key_epochs,
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
pub async fn fetch_iris_profile_restore_candidates(
    client: &Client,
    recovery_pubkey_hex: &str,
    timeout: Duration,
) -> Result<Vec<IrisProfileRestoreCandidate>, RelayError> {
    let discovery_events = client
        .get_events_of(
            iris_profile_restore_candidate_filters(recovery_pubkey_hex)?,
            nostr_sdk::EventSource::relays(Some(timeout)),
        )
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
    let candidate_ids =
        iris_profile_candidate_ids_for_pubkey_from_events(recovery_pubkey_hex, &discovery_events)?;
    if candidate_ids.is_empty() {
        return Ok(Vec::new());
    }
    let roster_events = client
        .get_events_of(
            candidate_ids
                .into_iter()
                .map(iris_profile_roster_op_filter)
                .collect::<Vec<_>>(),
            nostr_sdk::EventSource::relays(Some(timeout)),
        )
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
    let mut events = discovery_events;
    events.extend(roster_events);
    iris_profile_restore_candidates_from_events(recovery_pubkey_hex, &events)
}

/// Build the relay filter set covering profile roster ops and drive-root
/// events for a single profile's primary drive. Full `AppKeys` roster snapshots
/// are intentionally excluded from relays; `IrisProfile` roster ops are the
/// relay roster format.
///
/// The drive-root filter intentionally does **not** narrow by author:
/// the d-tag `iris-drive/<profile_or_share_id>/<drive>/root` already pins the drive,
/// and `apply_remote_drive_root_event` rejects events from unauthorized
/// `AppKeys`. Skipping the author filter means the daemon
/// doesn't need to re-subscribe every time the roster changes — newly
/// approved `AppKeys`' events flow in automatically.
#[must_use]
pub fn subscription_filters(
    current_app_key_pubkey_hex: &str,
    root_scope_id: &str,
    drive_id: &str,
) -> Vec<Filter> {
    subscription_filters_for_shared_roots(current_app_key_pubkey_hex, root_scope_id, drive_id, &[])
}

#[must_use]
pub fn subscription_filters_for_shared_roots(
    current_app_key_pubkey_hex: &str,
    root_scope_id: &str,
    drive_id: &str,
    share_ids: &[IrisProfileId],
) -> Vec<Filter> {
    let mut filters = Vec::new();
    if let Ok(profile_id) = root_scope_id.parse::<IrisProfileId>() {
        filters.push(iris_profile_roster_op_filter(profile_id));
        filters.push(
            Filter::new()
                .kind(nostr_sdk::Kind::from(KIND_APP_KEY_LINK_REQUEST))
                .custom_tag(
                    SingleLetterTag::lowercase(nostr_sdk::Alphabet::D),
                    [app_key_link_request_d_tag(profile_id)],
                ),
        );
    }
    push_drive_root_filters(&mut filters, root_scope_id, drive_id);
    for share_id in share_ids {
        let share_scope = share_id.to_string();
        if share_scope == root_scope_id {
            continue;
        }
        filters.push(iris_profile_roster_op_filter(*share_id));
        push_drive_root_filters(&mut filters, &share_scope, crate::PRIMARY_DRIVE_ID);
    }
    if let Ok(current_app_key) = PublicKey::from_hex(current_app_key_pubkey_hex) {
        filters.push(
            Filter::new()
                .author(current_app_key)
                .kind(nostr_sdk::Kind::from(KIND_HASHTREE_ROOT))
                .identifier(drive_id)
                .custom_tag(
                    SingleLetterTag::lowercase(nostr_sdk::Alphabet::L),
                    [hashtree_nostr::HASHTREE_LABEL],
                ),
        );
    }
    filters
}

fn push_drive_root_filters(filters: &mut Vec<Filter>, root_scope_id: &str, drive_id: &str) {
    let d_tag = drive_root_d_tag(root_scope_id, drive_id);
    filters.push(
        Filter::new()
            .kind(nostr_sdk::Kind::from(KIND_DRIVE_ROOT))
            .custom_tag(SingleLetterTag::lowercase(nostr_sdk::Alphabet::D), [d_tag]),
    );
}

fn iris_profile_roster_op_filter(profile_id: IrisProfileId) -> Filter {
    Filter::new()
        .kind(nostr_sdk::Kind::from(KIND_IRIS_PROFILE_ROSTER_OP))
        .custom_tag(
            SingleLetterTag::lowercase(nostr_sdk::Alphabet::I),
            [profile_id.to_string()],
        )
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
        .custom_tag(SingleLetterTag::lowercase(nostr_sdk::Alphabet::D), [d_tag]);
    let events = client
        .get_events_of(vec![filter], nostr_sdk::EventSource::relays(Some(timeout)))
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
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
        (Ok(candidate), Ok(current)) => candidate
            .app_key_seq
            .cmp(&current.app_key_seq)
            .then_with(|| candidate.published_at.cmp(&current.published_at))
            .then_with(|| candidate.dck_generation.cmp(&current.dck_generation))
            .is_gt(),
        _ => candidate.created_at.as_u64() > current.created_at.as_u64(),
    }
}

#[cfg(test)]
mod tests;
