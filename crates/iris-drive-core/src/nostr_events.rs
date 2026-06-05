//! Nostr wire format for Iris Drive events.
//!
//! One replaceable event kind (NIP-78 parameterized-replaceable range),
//! separated by Iris-specific `d` tags:
//!
//! - **`KIND_DRIVE_ROOT = 30078`** — AppKey-signed drive-root reference.
//!   d-tag: `"iris-drive/<profile_or_share_id>/<drive_id>/root"`.
//!   Pubkey = `AppKey` pubkey. Content = JSON root hash/key-wrap metadata,
//!   DCK generation, and optional causal fields. The event's `created_at`
//!   doubles as `DeviceRootRef::published_at`.
//!
//! All events are signed by the appropriate key and verify under the
//! event's own pubkey. Build functions return a signed `Event`; parse
//! functions take an `Event`, verify its signature, and extract the
//! application-level type.

use hashtree_core::{Cid, from_hex, to_hex};
use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{Event, EventBuilder, Keys, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::IrisProfileId;
use crate::config::DeviceRootRef;
use crate::device_link_transport::DeviceLinkRequestFrame;
use crate::root_meta::{RootObservation, RootParent};

/// NIP-78 parameterized-replaceable kind for AppKey-signed drive roots.
pub const KIND_DRIVE_ROOT: u16 = 30078;

/// NIP-78 parameterized-replaceable kind for AppKey-signed join requests.
pub const KIND_DEVICE_LINK_REQUEST: u16 = 30078;

/// Standard hashtree mutable-root kind used by drive.iris.to.
pub const KIND_HASHTREE_ROOT: u16 = 30_078;
const _: () = assert!(hashtree_nostr::HASHTREE_ROOT_KIND == 30_078);

#[must_use]
pub fn device_link_request_d_tag(profile_id: IrisProfileId) -> String {
    format!("iris-drive/{profile_id}/device-link-request")
}

#[must_use]
pub fn is_drive_root_event_kind(kind: u16) -> bool {
    kind == KIND_DRIVE_ROOT
}

#[must_use]
pub fn is_drive_root_event_coordinate(event: &Event) -> bool {
    is_drive_root_event_kind(event.kind.as_u16())
        && event
            .identifier()
            .is_some_and(|d_tag| parse_drive_root_d_tag(d_tag).is_ok())
}

#[must_use]
pub fn is_device_link_request_event_coordinate(event: &Event) -> bool {
    event.kind.as_u16() == KIND_DEVICE_LINK_REQUEST
        && event
            .identifier()
            .is_some_and(|d_tag| parse_device_link_request_d_tag(d_tag).is_ok())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveRootEventPreview {
    pub device_pubkey_hex: String,
    pub owner_pubkey_hex: String,
    pub drive_id: String,
    pub published_at: i64,
    pub dck_generation: u64,
    pub device_seq: u64,
}

#[derive(Debug, Error)]
pub enum WireError {
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
    #[error("signature verification failed: {0}")]
    SignatureFailed(String),
    #[error("invalid pubkey hex: {0}")]
    InvalidPubkey(String),
    #[error("invalid root cid: {0}")]
    InvalidRootCid(String),
    #[error("drive-root event has no root hash")]
    MissingRootHash,
    #[error("drive-root key is not available for this device")]
    RootKeyUnavailable,
    #[error(
        "device-link d-tag profile {d_tag_profile} does not match request profile {frame_profile}"
    )]
    DeviceLinkProfileMismatch {
        d_tag_profile: IrisProfileId,
        frame_profile: IrisProfileId,
    },
    #[error("device-link event signer {signer} does not match request device {device}")]
    DeviceLinkSignerMismatch { signer: String, device: String },
}

#[derive(Debug, Serialize, Deserialize)]
struct DriveRootWireContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_cid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    root_hash: Option<String>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    root_key_wraps: std::collections::BTreeMap<String, String>,
    dck_generation: u64,
    #[serde(default, skip_serializing_if = "is_zero")]
    device_seq: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parents: Vec<RootParent>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    observed: std::collections::BTreeMap<String, RootObservation>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(value: &u64) -> bool {
    *value == 0
}

/// Build a signed device-link request event. Signed by the requesting `AppKey`;
/// the profile-scoped d-tag routes the request to admins for that `IrisProfile`.
pub fn build_device_link_request_event(
    device_keys: &Keys,
    frame: &DeviceLinkRequestFrame,
) -> Result<Event, WireError> {
    PublicKey::from_hex(&frame.owner_pubkey)
        .map_err(|e| WireError::InvalidPubkey(e.to_string()))?;
    PublicKey::from_hex(&frame.device_pubkey)
        .map_err(|e| WireError::InvalidPubkey(e.to_string()))?;
    let content_json =
        serde_json::to_string(frame).map_err(|e| WireError::BadContent(e.to_string()))?;
    let builder = EventBuilder::new(
        Kind::from(KIND_DEVICE_LINK_REQUEST),
        content_json,
        [Tag::identifier(device_link_request_d_tag(frame.profile_id))],
    );
    builder
        .to_event(device_keys)
        .map_err(|e| WireError::Event(e.to_string()))
}

/// Parse + verify a signed device-link request event.
pub fn parse_device_link_request_event(event: &Event) -> Result<DeviceLinkRequestFrame, WireError> {
    let kind = event.kind.as_u16();
    if kind != KIND_DEVICE_LINK_REQUEST {
        return Err(WireError::WrongKind {
            expected: KIND_DEVICE_LINK_REQUEST,
            got: kind,
        });
    }
    let d_tag = event.identifier().ok_or(WireError::MissingDTag)?;
    let d_tag_profile = parse_device_link_request_d_tag(d_tag)?;
    event
        .verify()
        .map_err(|e| WireError::SignatureFailed(e.to_string()))?;
    let frame: DeviceLinkRequestFrame =
        serde_json::from_str(&event.content).map_err(|e| WireError::BadContent(e.to_string()))?;
    PublicKey::from_hex(&frame.owner_pubkey)
        .map_err(|e| WireError::InvalidPubkey(e.to_string()))?;
    PublicKey::from_hex(&frame.device_pubkey)
        .map_err(|e| WireError::InvalidPubkey(e.to_string()))?;
    if d_tag_profile != frame.profile_id {
        return Err(WireError::DeviceLinkProfileMismatch {
            d_tag_profile,
            frame_profile: frame.profile_id,
        });
    }
    let signer = event.pubkey.to_hex();
    if signer != frame.device_pubkey {
        return Err(WireError::DeviceLinkSignerMismatch {
            signer,
            device: frame.device_pubkey,
        });
    }
    Ok(frame)
}

fn parse_device_link_request_d_tag(d_tag: &str) -> Result<IrisProfileId, WireError> {
    let rest = d_tag
        .strip_prefix("iris-drive/")
        .ok_or_else(|| WireError::DTagMalformed(format!("no iris-drive/ prefix: {d_tag}")))?;
    let profile = rest.strip_suffix("/device-link-request").ok_or_else(|| {
        WireError::DTagMalformed(format!("no /device-link-request suffix: {d_tag}"))
    })?;
    if profile.is_empty() {
        return Err(WireError::DTagMalformed(format!(
            "empty device-link profile: {d_tag}"
        )));
    }
    profile
        .parse()
        .map_err(|error| WireError::DTagMalformed(format!("invalid device-link profile: {error}")))
}

/// Compute the d-tag for a drive-root event.
#[must_use]
pub fn drive_root_d_tag(root_scope_id: &str, drive_id: &str) -> String {
    format!("iris-drive/{root_scope_id}/{drive_id}/root")
}

/// Build a signed drive-root event. Signed by the **device key**;
/// `device_keys.public_key()` becomes the event author, and the
/// merge engine attributes the published root to that device.
///
/// This builder preserves `root.published_at` when present so build/parse
/// roundtrips remain stable. Live publishing should use
/// [`build_drive_root_publish_event`] so the replaceable event advances
/// even when the root CID is unchanged.
pub fn build_drive_root_event(
    device_keys: &Keys,
    root_scope_id: &str,
    drive_id: &str,
    root: &DeviceRootRef,
    authorized_device_pubkeys: &[String],
) -> Result<Event, WireError> {
    build_drive_root_event_at(
        device_keys,
        root_scope_id,
        drive_id,
        root,
        authorized_device_pubkeys,
        drive_root_timestamp_from_root(root),
    )
}

/// Build a signed drive-root event for live relay publishing.
///
/// Relays treat Iris Drive roots as replaceable events. If the root CID did
/// not change but the authorized recipient set did, reusing the old
/// `created_at` causes relays to reject the event and the newly linked device
/// never receives its root-key wrap.
pub fn build_drive_root_publish_event(
    device_keys: &Keys,
    root_scope_id: &str,
    drive_id: &str,
    root: &DeviceRootRef,
    authorized_device_pubkeys: &[String],
) -> Result<Event, WireError> {
    let stored_ts = if root.published_at > 0 {
        u64::try_from(root.published_at).unwrap_or(0)
    } else {
        0
    };
    let ts = unix_now_secs().max(stored_ts.saturating_add(1));
    build_drive_root_event_at(
        device_keys,
        root_scope_id,
        drive_id,
        root,
        authorized_device_pubkeys,
        ts,
    )
}

fn drive_root_timestamp_from_root(root: &DeviceRootRef) -> u64 {
    if root.published_at > 0 {
        u64::try_from(root.published_at).unwrap_or(0)
    } else {
        unix_now_secs()
    }
}

fn unix_now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

fn build_drive_root_event_at(
    device_keys: &Keys,
    root_scope_id: &str,
    drive_id: &str,
    root: &DeviceRootRef,
    authorized_device_pubkeys: &[String],
    created_at: u64,
) -> Result<Event, WireError> {
    let root_cid =
        Cid::parse(&root.root_cid).map_err(|e| WireError::InvalidRootCid(e.to_string()))?;
    let Some(root_key) = root_cid.key else {
        return Err(WireError::InvalidRootCid(
            "drive root is unencrypted".into(),
        ));
    };
    let root_key_hex = hex::encode(root_key);
    let mut root_key_wraps = std::collections::BTreeMap::new();
    let mut recipients: std::collections::BTreeSet<String> =
        authorized_device_pubkeys.iter().cloned().collect();
    recipients.insert(device_keys.public_key().to_hex());
    for recipient in recipients {
        let recipient_pk =
            PublicKey::from_hex(&recipient).map_err(|e| WireError::InvalidPubkey(e.to_string()))?;
        let ciphertext = nip44::encrypt(
            device_keys.secret_key(),
            &recipient_pk,
            root_key_hex.clone(),
            Nip44Version::V2,
        )
        .map_err(|e| WireError::Event(format!("root-key wrap: {e}")))?;
        root_key_wraps.insert(recipient, ciphertext);
    }

    let content = DriveRootWireContent {
        root_cid: None,
        root_hash: Some(to_hex(&root_cid.hash)),
        root_key_wraps,
        dck_generation: root.dck_generation,
        device_seq: root.device_seq,
        parents: root.parents.clone(),
        observed: root.observed.clone(),
    };
    let content_json =
        serde_json::to_string(&content).map_err(|e| WireError::BadContent(e.to_string()))?;
    let d_tag = drive_root_d_tag(root_scope_id, drive_id);
    let builder = EventBuilder::new(
        Kind::from(KIND_DRIVE_ROOT),
        content_json,
        [Tag::identifier(d_tag)],
    )
    .custom_created_at(nostr_sdk::Timestamp::from(created_at));
    let event = builder
        .to_event(device_keys)
        .map_err(|e| WireError::Event(e.to_string()))?;
    Ok(event)
}

/// Build a standard private hashtree mutable-root event for drive.iris.to.
///
/// Iris Drive keeps its richer multi-device drive-root event, but the files
/// app already understands kind 30078 tree roots with `#l=hashtree`, `hash`,
/// and `selfEncryptedKey` tags. This event points `npub/tree_name` at the
/// current root without publishing the root key in cleartext.
pub fn build_private_hashtree_root_event(
    owner_keys: &Keys,
    tree_name: &str,
    root: &DeviceRootRef,
) -> Result<Event, WireError> {
    let root_cid =
        Cid::parse(&root.root_cid).map_err(|e| WireError::InvalidRootCid(e.to_string()))?;
    let ts = if root.published_at > 0 {
        Some(u64::try_from(root.published_at).unwrap_or(0))
    } else {
        None
    };
    hashtree_nostr::build_private_hashtree_root_event(owner_keys, tree_name, &root_cid, ts)
        .map_err(|e| WireError::Event(e.to_string()))
}

/// Parse + verify a drive-root event. Returns
/// `(device_pubkey_hex, root_scope_id, drive_id, DeviceRootRef)`.
/// The device pubkey is the event's author; the root scope id and
/// drive id are extracted from the d-tag.
pub fn parse_drive_root_event(
    event: &Event,
) -> Result<(String, String, String, DeviceRootRef), WireError> {
    parse_drive_root_event_inner(event, None)
}

pub fn parse_drive_root_event_for_device(
    event: &Event,
    device_keys: &Keys,
) -> Result<(String, String, String, DeviceRootRef), WireError> {
    parse_drive_root_event_inner(event, Some(device_keys))
}

pub fn parse_drive_root_event_header(event: &Event) -> Result<(String, String, String), WireError> {
    let (device_pubkey_hex, owner_pubkey_hex, drive_id, _, _) =
        parse_drive_root_event_parts(event)?;
    Ok((device_pubkey_hex, owner_pubkey_hex, drive_id))
}

pub fn parse_drive_root_event_preview(event: &Event) -> Result<DriveRootEventPreview, WireError> {
    let (device_pubkey_hex, owner_pubkey_hex, drive_id, content, published_at) =
        parse_drive_root_event_parts(event)?;
    Ok(DriveRootEventPreview {
        device_pubkey_hex,
        owner_pubkey_hex,
        drive_id,
        published_at,
        dck_generation: content.dck_generation,
        device_seq: content.device_seq,
    })
}

fn parse_drive_root_event_inner(
    event: &Event,
    device_keys: Option<&Keys>,
) -> Result<(String, String, String, DeviceRootRef), WireError> {
    let (device_pubkey_hex, owner_pubkey_hex, drive_id, content, published_at) =
        parse_drive_root_event_parts(event)?;
    let root_cid = root_cid_from_wire_content(event, &content, device_keys)?;
    let device_root = DeviceRootRef {
        root_cid,
        published_at,
        dck_generation: content.dck_generation,
        device_seq: content.device_seq,
        parents: content.parents,
        observed: content.observed,
        local_only: false,
    };
    Ok((device_pubkey_hex, owner_pubkey_hex, drive_id, device_root))
}

fn parse_drive_root_event_parts(
    event: &Event,
) -> Result<(String, String, String, DriveRootWireContent, i64), WireError> {
    let kind = event.kind.as_u16();
    if !is_drive_root_event_kind(kind) {
        return Err(WireError::WrongKind {
            expected: KIND_DRIVE_ROOT,
            got: kind,
        });
    }
    let d_tag = event.identifier().ok_or(WireError::MissingDTag)?;
    let (owner_pubkey_hex, drive_id) = parse_drive_root_d_tag(d_tag)?;
    event
        .verify()
        .map_err(|e| WireError::SignatureFailed(e.to_string()))?;
    let content: DriveRootWireContent =
        serde_json::from_str(&event.content).map_err(|e| WireError::BadContent(e.to_string()))?;
    let device_pubkey_hex = event.pubkey.to_hex();
    let published_at = i64::try_from(event.created_at.as_u64()).unwrap_or(i64::MAX);
    Ok((
        device_pubkey_hex,
        owner_pubkey_hex,
        drive_id,
        content,
        published_at,
    ))
}

fn root_cid_from_wire_content(
    event: &Event,
    content: &DriveRootWireContent,
    device_keys: Option<&Keys>,
) -> Result<String, WireError> {
    if let Some(root_cid) = content.root_cid.as_ref() {
        Cid::parse(root_cid).map_err(|e| WireError::InvalidRootCid(e.to_string()))?;
        return Ok(root_cid.clone());
    }

    let root_hash = content
        .root_hash
        .as_ref()
        .ok_or(WireError::MissingRootHash)?;
    let hash = from_hex(root_hash).map_err(|e| WireError::InvalidRootCid(e.to_string()))?;
    let Some(device_keys) = device_keys else {
        return Err(WireError::RootKeyUnavailable);
    };
    let recipient = device_keys.public_key().to_hex();
    let ciphertext = content
        .root_key_wraps
        .get(&recipient)
        .ok_or(WireError::RootKeyUnavailable)?;
    let key_hex = nip44::decrypt(device_keys.secret_key(), &event.pubkey, ciphertext)
        .map_err(|_| WireError::RootKeyUnavailable)?;
    let key = from_hex(&key_hex).map_err(|e| WireError::InvalidRootCid(e.to_string()))?;
    Ok(Cid {
        hash,
        key: Some(key),
    }
    .to_string())
}

fn parse_drive_root_d_tag(d_tag: &str) -> Result<(String, String), WireError> {
    // expected: iris-drive/<profile_or_share_id>/<drive_id>/root
    let rest = d_tag
        .strip_prefix("iris-drive/")
        .ok_or_else(|| WireError::DTagMalformed(format!("no iris-drive/ prefix: {d_tag}")))?;
    let rest = rest
        .strip_suffix("/root")
        .ok_or_else(|| WireError::DTagMalformed(format!("no /root suffix: {d_tag}")))?;
    let Some((owner, drive)) = rest.split_once('/') else {
        return Err(WireError::DTagMalformed(format!(
            "missing owner/drive separator: {d_tag}"
        )));
    };
    if owner.is_empty() || drive.is_empty() {
        return Err(WireError::DTagMalformed(format!(
            "empty owner or drive id: {d_tag}"
        )));
    }
    Ok((owner.to_string(), drive.to_string()))
}

#[cfg(test)]
mod tests;
