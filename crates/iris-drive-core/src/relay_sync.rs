//! Relay-layer sync: publish + fetch + apply.
//!
//! Two layers:
//!
//! - **Apply (offline)** — `apply_remote_app_keys_event` and
//!   `apply_remote_drive_root_event` take a parsed Nostr event and an
//!   `AppConfig` and apply the event's effect onto the config. These are
//!   pure functions over data, fully covered by unit tests.
//!
//! - **Network (live)** — `publish_app_keys`, `publish_drive_root`,
//!   `fetch_latest_app_keys`, `fetch_drive_roots` wrap nostr-sdk's
//!   `Client` for actual relay I/O. Tested manually against real relays;
//!   the wire/apply layers below them are what we cover automatically.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use nostr_sdk::{Client, Event, Filter, JsonUtil, Keys, Options, PublicKey, SingleLetterTag};
use thiserror::Error;

use crate::account::app_keys_from_profile_roster;
use crate::app_keys::{AppKeysEventRecord, ApplyDecision, apply_snapshot};
use crate::config::{AppConfig, DeviceRootRef, Drive};
use crate::device_link_transport::DeviceLinkRosterFrame;
use crate::nostr_events::{
    KIND_APP_KEYS, KIND_DEVICE_LINK_REQUEST, KIND_DRIVE_ROOT, KIND_HASHTREE_ROOT,
    KIND_LEGACY_DRIVE_ROOT, app_keys_d_tag, build_app_keys_event, build_device_link_request_event,
    build_drive_root_publish_event, build_private_hashtree_root_event, device_link_request_d_tag,
    drive_root_d_tag, parse_app_keys_event, parse_device_link_request_event,
    parse_drive_root_event, parse_drive_root_event_for_device, parse_drive_root_event_preview,
};
use crate::{
    AppKeysSnapshot, IrisProfileId, KIND_IRIS_PROFILE_ROSTER_OP, SignedIrisProfileRosterOp,
    parse_iris_profile_roster_op_event,
};

#[derive(Debug, Error)]
pub enum RelayError {
    #[error("wire: {0}")]
    Wire(#[from] crate::nostr_events::WireError),
    #[error("nostr client: {0}")]
    Client(String),
    #[error("config has no account; run `idrive init` first")]
    NoAccount,
    #[error("this device is not an admin; cannot sign AppKeys events")]
    NoOwnerAuthority,
    #[error("invalid pubkey: {0}")]
    InvalidPubkey(String),
    #[error("hashtree root: {0}")]
    HashtreeRoot(String),
    #[error("account: {0}")]
    Account(#[from] crate::account::AccountError),
    #[error("iris profile: {0}")]
    IrisProfile(#[from] crate::iris_profile::IrisProfileError),
    #[error("device-link roster: {0}")]
    DeviceLinkRoster(String),
}

/// Result of applying a remote `AppKeys` event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppKeysApply {
    /// Event from someone other than our owner — silently ignored.
    NotOurOwner,
    /// Event is for our account, but it was not signed by an accepted admin.
    UnauthorizedSigner,
    /// Applied to the local state; carries the apply decision so callers
    /// can log "first snapshot adopted", "newer snapshot replaced", etc.
    Applied(ApplyDecision),
}

/// Result of applying an admin roster sent over the direct device-link channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceLinkRosterApply {
    /// The event is not applicable to this device/account.
    Ignored,
    /// The local roster already matches this event.
    Current,
    /// The event was accepted by the `AppKeys` timeline rules.
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

/// Apply a remote `AppKeys` event to `config`. The event may be signed by any
/// current admin device. The parsed snapshot carries the stable account id;
/// the event author becomes the roster signer and DCK wrapping key.
pub fn apply_remote_app_keys_event(
    config: &mut AppConfig,
    event: &Event,
) -> Result<AppKeysApply, RelayError> {
    let snapshot = parse_app_keys_event(event)?;
    let signer_pubkey = event.pubkey.to_hex();
    let Some(account) = config.account.as_mut() else {
        return Err(RelayError::NoAccount);
    };
    if snapshot.owner_pubkey != account.owner_pubkey {
        return Ok(AppKeysApply::NotOurOwner);
    }
    if !can_accept_app_keys_from(account, &signer_pubkey, &snapshot) {
        return Ok(AppKeysApply::UnauthorizedSigner);
    }
    let record = AppKeysEventRecord {
        event_id: event.id.to_hex(),
        signer_pubkey,
        event_json: event.as_json(),
    };
    let decision = account.apply_signed_app_keys(snapshot, record);
    Ok(AppKeysApply::Applied(decision))
}

/// Result of applying a device-link request sent over relay metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceLinkRequestApply {
    /// The event is addressed to another account.
    NotOurOwner,
    /// This install is not an admin device and cannot approve devices.
    NotAdmin,
    /// The event did not carry this admin's current invite secret.
    InvalidSecret,
    /// The request was already represented locally.
    Current,
    /// The inbound request queue changed.
    Recorded,
}

/// Apply a signed device-link request delivered by relay.
pub fn apply_remote_device_link_request_event(
    config: &mut AppConfig,
    event: &Event,
) -> Result<DeviceLinkRequestApply, RelayError> {
    let frame = parse_device_link_request_event(event)?;
    let Some(account) = config.account.as_mut() else {
        return Err(RelayError::NoAccount);
    };
    if frame.owner_pubkey != account.owner_pubkey {
        return Ok(DeviceLinkRequestApply::NotOurOwner);
    }
    if !account.can_manage_devices() {
        return Ok(DeviceLinkRequestApply::NotAdmin);
    }
    let expected_secret = account.device_link_secret.trim();
    if !expected_secret.is_empty() && frame.link_secret.trim() != expected_secret {
        return Ok(DeviceLinkRequestApply::InvalidSecret);
    }

    let changed = account.record_inbound_device_link_request(
        &frame.owner_pubkey,
        &frame.device_pubkey,
        frame.label,
        &frame.link_secret,
        frame.requested_at,
    )?;
    if changed {
        Ok(DeviceLinkRequestApply::Recorded)
    } else {
        Ok(DeviceLinkRequestApply::Current)
    }
}

/// Apply a signed roster delivered over device-link/FIPS.
///
/// A brand-new linked device only accepts the first roster from the admin it
/// explicitly requested approval from. Once it has a current roster, it must
/// continue accepting newer rosters signed by a current admin so it learns
/// about devices approved after itself.
pub fn apply_device_link_roster_frame(
    config: &mut AppConfig,
    frame: &DeviceLinkRosterFrame,
    event: &Event,
    admin_device_pubkey: &str,
) -> Result<DeviceLinkRosterApply, RelayError> {
    if frame.schema != 1 {
        return Ok(DeviceLinkRosterApply::Ignored);
    }
    let snapshot = parse_app_keys_event(event)?;
    let signer_pubkey = event.pubkey.to_hex();
    let event_id = event.id.to_hex();
    let Some(account) = config.account.as_ref() else {
        return Err(RelayError::NoAccount);
    };
    if frame.app_keys != snapshot
        || frame.app_keys_event_id != event_id
        || frame.owner_pubkey != snapshot.owner_pubkey
        || frame.admin_device_pubkey != admin_device_pubkey
        || snapshot.owner_pubkey != account.owner_pubkey
        || signer_pubkey != admin_device_pubkey
        || !snapshot.is_admin(admin_device_pubkey)
    {
        return Ok(DeviceLinkRosterApply::Ignored);
    }
    if !account.profile_roster_ops.is_empty() && account.profile_id != frame.profile_id {
        return Ok(DeviceLinkRosterApply::Ignored);
    }

    let incoming_ops = verified_profile_roster_ops(frame.profile_id, &frame.profile_roster_ops)?;
    let projected_snapshot =
        app_keys_from_profile_roster(&snapshot.owner_pubkey, frame.profile_id, &incoming_ops)
            .ok_or_else(|| {
                RelayError::DeviceLinkRoster("profile roster has no AppKey epoch".into())
            })?;
    if projected_snapshot != snapshot {
        return Ok(DeviceLinkRosterApply::Ignored);
    }

    let has_current_roster = account.app_keys.is_some() || !account.profile_roster_ops.is_empty();
    let pending_from_admin = account
        .outbound_device_link_request
        .as_ref()
        .is_some_and(|pending| pending.admin_device_pubkey == admin_device_pubkey);
    if !has_current_roster && !pending_from_admin {
        return Ok(DeviceLinkRosterApply::Ignored);
    }
    if pending_from_admin && !snapshot.contains(&account.device_pubkey) {
        return Ok(DeviceLinkRosterApply::Ignored);
    }

    if account.app_keys.as_ref() == Some(&snapshot)
        && account.profile_id == frame.profile_id
        && same_profile_ops(&account.profile_roster_ops, &incoming_ops)
    {
        return Ok(DeviceLinkRosterApply::Current);
    }

    let mut current = account.app_keys.clone();
    let decision = apply_snapshot(&mut current, snapshot.clone());
    if decision == ApplyDecision::Rejected {
        return Ok(DeviceLinkRosterApply::Applied(decision));
    }

    let record = AppKeysEventRecord {
        event_id,
        signer_pubkey,
        event_json: event.as_json(),
    };
    let root_scope_id = {
        let Some(account) = config.account.as_mut() else {
            return Err(RelayError::NoAccount);
        };
        account.profile_roster_ops = if account.profile_id == frame.profile_id {
            merge_profile_roster_ops(&account.profile_roster_ops, &incoming_ops)
        } else {
            incoming_ops
        };
        account.profile_id = frame.profile_id;
        account.app_keys = Some(snapshot);
        account.app_keys_event = Some(record);
        account.recompute_authorization();
        account.root_scope_id()
    };
    sync_primary_drive_scope(config, root_scope_id);
    Ok(DeviceLinkRosterApply::Applied(decision))
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
    let Some(account) = config.account.as_ref() else {
        return Err(RelayError::NoAccount);
    };
    if op.content.profile_id != account.profile_id {
        return Ok(IrisProfileRosterOpApply::NotOurProfile);
    }
    if account
        .profile_roster_ops
        .iter()
        .any(|current| current.op_id == op.op_id)
    {
        return Ok(IrisProfileRosterOpApply::Current);
    }

    let root_scope_id = {
        let Some(account) = config.account.as_mut() else {
            return Err(RelayError::NoAccount);
        };
        account.profile_roster_ops = merge_profile_roster_ops(&account.profile_roster_ops, &[op]);
        account.sync_app_keys_from_profile();
        account.recompute_authorization();
        account.root_scope_id()
    };
    sync_primary_drive_scope(config, root_scope_id);
    Ok(IrisProfileRosterOpApply::Applied)
}

pub fn apply_device_link_roster_event(
    config: &mut AppConfig,
    event: &Event,
    admin_device_pubkey: &str,
) -> Result<DeviceLinkRosterApply, RelayError> {
    let snapshot = parse_app_keys_event(event)?;
    let signer_pubkey = event.pubkey.to_hex();
    let Some(account) = config.account.as_ref() else {
        return Err(RelayError::NoAccount);
    };
    if snapshot.owner_pubkey != account.owner_pubkey
        || signer_pubkey != admin_device_pubkey
        || !snapshot.is_admin(admin_device_pubkey)
    {
        return Ok(DeviceLinkRosterApply::Ignored);
    }

    if account.app_keys.as_ref() == Some(&snapshot) {
        return Ok(DeviceLinkRosterApply::Current);
    }

    let has_current_roster = account.app_keys.is_some();
    let pending_from_admin = account
        .outbound_device_link_request
        .as_ref()
        .is_some_and(|pending| pending.admin_device_pubkey == admin_device_pubkey);
    if !has_current_roster && !pending_from_admin {
        return Ok(DeviceLinkRosterApply::Ignored);
    }
    if pending_from_admin && !snapshot.contains(&account.device_pubkey) {
        return Ok(DeviceLinkRosterApply::Ignored);
    }

    match apply_remote_app_keys_event(config, event)? {
        AppKeysApply::Applied(decision) => Ok(DeviceLinkRosterApply::Applied(decision)),
        AppKeysApply::NotOurOwner | AppKeysApply::UnauthorizedSigner => {
            Ok(DeviceLinkRosterApply::Ignored)
        }
    }
}

fn verified_profile_roster_ops(
    profile_id: crate::IrisProfileId,
    ops: &[SignedIrisProfileRosterOp],
) -> Result<Vec<SignedIrisProfileRosterOp>, RelayError> {
    let mut by_id = BTreeMap::new();
    for op in ops {
        let event = Event::from_json(&op.event_json).map_err(|error| {
            RelayError::DeviceLinkRoster(format!("parsing profile roster op event: {error}"))
        })?;
        let parsed = crate::parse_iris_profile_roster_op_event(&event)?;
        if parsed.content.profile_id != profile_id {
            return Err(RelayError::DeviceLinkRoster(format!(
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

fn sync_primary_drive_scope(config: &mut AppConfig, root_scope_id: String) {
    if let Some(drive) = config
        .drives
        .iter_mut()
        .find(|drive| drive.drive_id == crate::daemon::PRIMARY_DRIVE_ID)
    {
        drive.owner_pubkey = root_scope_id;
    } else {
        config.upsert_drive(Drive::primary(root_scope_id));
    }
}

fn can_accept_app_keys_from(
    account: &crate::account::AccountState,
    signer_pubkey: &str,
    _snapshot: &AppKeysSnapshot,
) -> bool {
    if let Some(current) = account.app_keys.as_ref() {
        return current.is_admin(signer_pubkey);
    }
    // A bare AppKeys snapshot has no IrisProfileId. New links must be
    // admitted by DeviceLinkRosterFrame/profile roster ops so they do not
    // become authorized under their temporary local profile id.
    false
}

/// Result of applying a remote drive-root event.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriveRootApply {
    /// Owner pubkey in the d-tag doesn't match our account — ignored.
    NotOurOwner,
    /// Drive id in the d-tag isn't configured locally — ignored.
    UnknownDrive,
    /// Device pubkey isn't in the current `AppKeys` roster — ignored.
    /// Protects against forged events from unauthorized devices.
    UnauthorizedDevice,
    /// Older than what we already have for this device — ignored.
    /// Causal roots compare by `device_seq`; legacy roots compare by
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
    /// Event author doesn't match our owner.
    NotOurOwner,
    /// The tree name does not match a configured drive id.
    UnknownDrive,
    /// Older than the local root we already mapped to this device.
    StaleTimestamp,
    /// Applied as this device's current root.
    Applied,
}

/// Apply a remote drive-root event to `config`. Drops events from
/// foreign owners, unknown drives, unauthorized devices, or that are
/// older than what's already recorded.
pub fn apply_remote_drive_root_event(
    config: &mut AppConfig,
    event: &Event,
    device_keys: Option<&Keys>,
) -> Result<DriveRootApply, RelayError> {
    let preview = parse_drive_root_event_preview(event)?;
    let device_hex = preview.device_pubkey_hex.clone();
    let Some(account) = config.account.as_ref() else {
        return Err(RelayError::NoAccount);
    };
    if preview.owner_pubkey_hex == account.root_scope_id() {
        let authorized: BTreeSet<String> = account
            .app_keys
            .as_ref()
            .map(|s| s.devices.iter().map(|d| d.pubkey.clone()).collect())
            .unwrap_or_default();
        if !authorized.contains(&device_hex) {
            return Ok(DriveRootApply::UnauthorizedDevice);
        }
        let Some(drive) = config
            .drives
            .iter_mut()
            .find(|d| d.drive_id == preview.drive_id)
        else {
            return Ok(DriveRootApply::UnknownDrive);
        };
        return apply_root_to_device_roots(&mut drive.device_roots, event, device_keys, &preview);
    }

    let Ok(share_id) = preview.owner_pubkey_hex.parse::<IrisProfileId>() else {
        return Ok(DriveRootApply::NotOurOwner);
    };
    if preview.drive_id != crate::PRIMARY_DRIVE_ID {
        return Ok(DriveRootApply::UnknownDrive);
    }
    let Some(shared_folder) = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == share_id)
    else {
        return Ok(DriveRootApply::NotOurOwner);
    };
    if !shared_folder.projection().can_write_roots(&device_hex) {
        return Ok(DriveRootApply::UnauthorizedDevice);
    }
    apply_root_to_device_roots(
        &mut shared_folder.device_roots,
        event,
        device_keys,
        &preview,
    )
}

fn apply_root_to_device_roots(
    device_roots: &mut BTreeMap<String, DeviceRootRef>,
    event: &Event,
    device_keys: Option<&Keys>,
    preview: &crate::nostr_events::DriveRootEventPreview,
) -> Result<DriveRootApply, RelayError> {
    let device_hex = preview.device_pubkey_hex.clone();
    if let Some(existing) = device_roots.get(&device_hex)
        && incoming_root_is_stale(existing, preview.device_seq, preview.published_at)
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
    if device_roots
        .get(&device_hex)
        .is_some_and(|existing| existing.root_cid == incoming_root.root_cid)
    {
        return Ok(DriveRootApply::StaleTimestamp);
    }
    device_roots.insert(device_hex, incoming_root);
    Ok(DriveRootApply::Applied)
}

/// Apply a standard web hashtree root event to the local primary device root.
///
/// Web Iris apps publish one owner-signed mutable root per tree. Native Iris
/// Drive stores roots per authorized device, so an owner-capable native client
/// imports that web root as its own current device contribution. Native
/// drive-root events remain the richer multi-device protocol; this bridge makes
/// the web root an interoperable source of truth for restored web accounts and
/// browser-origin edits.
pub fn apply_remote_files_root_event(
    config: &mut AppConfig,
    event: &Event,
    owner_keys: Option<&Keys>,
) -> Result<FilesRootApply, RelayError> {
    let parsed = hashtree_nostr::parse_verified_hashtree_root_event(event)
        .map_err(|e| RelayError::HashtreeRoot(e.to_string()))?
        .ok_or_else(|| RelayError::HashtreeRoot("not a hashtree root event".to_string()))?;
    let root_cid = if let Some(owner_keys) = owner_keys {
        hashtree_nostr::resolve_self_encrypted_root_cid(&parsed, owner_keys)
            .map_err(|e| RelayError::HashtreeRoot(e.to_string()))?
    } else {
        parsed.root_cid.clone()
    };
    if root_cid.key.is_none() {
        return Err(RelayError::HashtreeRoot(
            "hashtree root key is unavailable".to_string(),
        ));
    }
    let incoming_root = DeviceRootRef {
        root_cid: root_cid.to_string(),
        published_at: i64::try_from(parsed.event.created_at).unwrap_or(i64::MAX),
        dck_generation: 0,
        device_seq: 0,
        parents: Vec::new(),
        observed: std::collections::BTreeMap::new(),
        local_only: false,
    };
    let Some(account) = config.account.as_ref() else {
        return Err(RelayError::NoAccount);
    };
    if parsed.event.pubkey != account.owner_pubkey {
        return Ok(FilesRootApply::NotOurOwner);
    }
    let device_pubkey = account.device_pubkey.clone();
    let Some(drive) = config
        .drives
        .iter_mut()
        .find(|d| d.drive_id == parsed.tree_name)
    else {
        return Ok(FilesRootApply::UnknownDrive);
    };
    if let Some(existing) = drive.device_roots.get(&device_pubkey) {
        if existing.root_cid == incoming_root.root_cid {
            return Ok(FilesRootApply::StaleTimestamp);
        }
        if existing.device_seq > 0 {
            return Ok(FilesRootApply::StaleTimestamp);
        }
        if existing.published_at >= incoming_root.published_at {
            return Ok(FilesRootApply::StaleTimestamp);
        }
    }
    drive.last_root_cid = Some(incoming_root.root_cid.clone());
    drive.device_roots.insert(device_pubkey, incoming_root);
    Ok(FilesRootApply::Applied)
}

fn incoming_root_is_stale(
    existing: &DeviceRootRef,
    incoming_device_seq: u64,
    incoming_published_at: i64,
) -> bool {
    if existing.device_seq > 0 || incoming_device_seq > 0 {
        if incoming_device_seq == 0 {
            return true;
        }
        if existing.device_seq == 0 {
            return false;
        }
        return incoming_device_seq <= existing.device_seq;
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

/// Publish a signed `AppKeys` event for the current snapshot.
pub async fn publish_app_keys(
    client: &Client,
    admin_keys: &Keys,
    snapshot: &AppKeysSnapshot,
) -> Result<nostr_sdk::EventId, RelayError> {
    let event = build_app_keys_event(admin_keys, snapshot)?;
    let output = client
        .send_event(event)
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
    Ok(*output.id())
}

/// Publish a signed device-link request from the requesting device.
pub async fn publish_device_link_request(
    client: &Client,
    device_keys: &Keys,
    frame: &crate::device_link_transport::DeviceLinkRequestFrame,
) -> Result<nostr_sdk::EventId, RelayError> {
    let event = build_device_link_request_event(device_keys, frame)?;
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
    root: &DeviceRootRef,
    authorized_device_pubkeys: &[String],
) -> Result<nostr_sdk::EventId, RelayError> {
    let event = build_drive_root_publish_event(
        device_keys,
        root_scope_id,
        drive_id,
        root,
        authorized_device_pubkeys,
    )?;
    let output = client
        .send_event(event)
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
    Ok(*output.id())
}

/// Publish the owner-private drive.iris.to-compatible mutable tree root.
pub async fn publish_files_root(
    client: &Client,
    owner_keys: &Keys,
    tree_name: &str,
    root: &DeviceRootRef,
) -> Result<nostr_sdk::EventId, RelayError> {
    let event = build_private_hashtree_root_event(owner_keys, tree_name, root)?;
    let output = client
        .send_event(event)
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
    Ok(*output.id())
}

/// Fetch the latest `AppKeys` event for `owner_pubkey_hex` across all
/// connected relays. Kept for compatibility experiments; production sync
/// publishes and fetches `IrisProfile` roster ops instead of legacy
/// `AppKeys` snapshots.
pub async fn fetch_latest_app_keys(
    client: &Client,
    owner_pubkey_hex: &str,
    timeout: Duration,
) -> Result<Option<Event>, RelayError> {
    let filter = Filter::new()
        .kind(nostr_sdk::Kind::from(KIND_APP_KEYS))
        .identifier(app_keys_d_tag(owner_pubkey_hex));
    let legacy_filter = PublicKey::from_hex(owner_pubkey_hex)
        .map(|owner| {
            Filter::new()
                .author(owner)
                .kind(nostr_sdk::Kind::from(KIND_APP_KEYS))
                .identifier(crate::nostr_events::D_TAG_APP_KEYS)
        })
        .map_err(|e| RelayError::InvalidPubkey(e.to_string()))?;
    let events = client
        .get_events_of(
            vec![filter, legacy_filter],
            nostr_sdk::EventSource::relays(Some(timeout)),
        )
        .await
        .map_err(|e| RelayError::Client(e.to_string()))?;
    // Among returned events, pick the newest one that actually claims this
    // account id. Admin authorization is checked when applying because it
    // depends on the local roster.
    let latest = events
        .into_iter()
        .filter(|event| {
            parse_app_keys_event(event)
                .is_ok_and(|snapshot| snapshot.owner_pubkey == owner_pubkey_hex)
        })
        .max_by_key(|e| e.created_at.as_u64());
    Ok(latest)
}

/// Fetch the latest standard hashtree root for `owner_pubkey_hex/tree_name`.
pub async fn fetch_latest_files_root(
    client: &Client,
    owner_pubkey_hex: &str,
    tree_name: &str,
    timeout: Duration,
) -> Result<Option<Event>, RelayError> {
    let owner = PublicKey::from_hex(owner_pubkey_hex)
        .map_err(|e| RelayError::InvalidPubkey(e.to_string()))?;
    let filter = Filter::new()
        .author(owner)
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

/// Build the relay filter set covering profile roster ops and drive-root
/// events for a single profile's primary drive. Legacy `AppKeys` snapshots are
/// intentionally excluded from relays; `IrisProfile` roster ops are the relay
/// roster format.
///
/// The drive-root filter intentionally does **not** narrow by author:
/// the d-tag `iris-drive/<owner>/<drive>/root` already pins the drive,
/// and `apply_remote_drive_root_event` rejects events from
/// unauthorized devices. Skipping the author filter means the daemon
/// doesn't need to re-subscribe every time the roster changes — newly
/// approved devices' events flow in automatically.
#[must_use]
pub fn subscription_filters(
    owner_pubkey_hex: &str,
    root_scope_id: &str,
    drive_id: &str,
) -> Vec<Filter> {
    let mut filters = Vec::new();
    if let Ok(profile_id) = root_scope_id.parse::<IrisProfileId>() {
        filters.push(iris_profile_roster_op_filter(profile_id));
    }
    filters.push(
        Filter::new()
            .kind(nostr_sdk::Kind::from(KIND_DEVICE_LINK_REQUEST))
            .custom_tag(
                SingleLetterTag::lowercase(nostr_sdk::Alphabet::D),
                [device_link_request_d_tag(owner_pubkey_hex)],
            ),
    );
    let d_tag = drive_root_d_tag(root_scope_id, drive_id);
    filters.push(
        Filter::new()
            .kind(nostr_sdk::Kind::from(KIND_DRIVE_ROOT))
            .custom_tag(
                SingleLetterTag::lowercase(nostr_sdk::Alphabet::D),
                [d_tag.clone()],
            ),
    );
    filters.push(
        Filter::new()
            .kind(nostr_sdk::Kind::from(KIND_LEGACY_DRIVE_ROOT))
            .custom_tag(SingleLetterTag::lowercase(nostr_sdk::Alphabet::D), [d_tag]),
    );
    if let Ok(owner) = PublicKey::from_hex(owner_pubkey_hex) {
        filters.push(
            Filter::new()
                .author(owner)
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

fn iris_profile_roster_op_filter(profile_id: IrisProfileId) -> Filter {
    Filter::new()
        .kind(nostr_sdk::Kind::from(KIND_IRIS_PROFILE_ROSTER_OP))
        .custom_tag(
            SingleLetterTag::lowercase(nostr_sdk::Alphabet::I),
            [profile_id.to_string()],
        )
}

/// Fetch drive-root events from any of `authorized_devices` for
/// `(owner_pubkey, drive_id)`. Returns the latest event from each
/// device (one event per device).
pub async fn fetch_drive_roots(
    client: &Client,
    root_scope_id: &str,
    drive_id: &str,
    authorized_devices: &[String],
    timeout: Duration,
) -> Result<Vec<Event>, RelayError> {
    if authorized_devices.is_empty() {
        return Ok(Vec::new());
    }
    let mut authors = Vec::with_capacity(authorized_devices.len());
    for hex in authorized_devices {
        authors
            .push(PublicKey::from_hex(hex).map_err(|e| RelayError::InvalidPubkey(e.to_string()))?);
    }
    let d_tag = drive_root_d_tag(root_scope_id, drive_id);
    let new_filter = Filter::new()
        .authors(authors)
        .kind(nostr_sdk::Kind::from(KIND_DRIVE_ROOT))
        .custom_tag(
            SingleLetterTag::lowercase(nostr_sdk::Alphabet::D),
            [d_tag.clone()],
        );
    let mut legacy_authors = Vec::with_capacity(authorized_devices.len());
    for hex in authorized_devices {
        legacy_authors
            .push(PublicKey::from_hex(hex).map_err(|e| RelayError::InvalidPubkey(e.to_string()))?);
    }
    let legacy_filter = Filter::new()
        .authors(legacy_authors)
        .kind(nostr_sdk::Kind::from(KIND_LEGACY_DRIVE_ROOT))
        .custom_tag(SingleLetterTag::lowercase(nostr_sdk::Alphabet::D), [d_tag]);
    let events = client
        .get_events_of(
            vec![new_filter, legacy_filter],
            nostr_sdk::EventSource::relays(Some(timeout)),
        )
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
            .device_seq
            .cmp(&current.device_seq)
            .then_with(|| candidate.published_at.cmp(&current.published_at))
            .then_with(|| candidate.dck_generation.cmp(&current.dck_generation))
            .is_gt(),
        _ => candidate.created_at.as_u64() > current.created_at.as_u64(),
    }
}

#[cfg(test)]
mod tests;
