//! Nostr wire format for Iris Drive events.
//!
//! One replaceable event kind (NIP-78 parameterized-replaceable range),
//! separated by Iris-specific `d` tags:
//!
//! - **`KIND_DRIVE_ROOT = 30078`** — AppKey-signed drive-root reference.
//!   d-tag: `"iris-drive/<profile_or_share_id>/<drive_id>/root"`.
//!   Pubkey = `AppKey` pubkey. Content = JSON root hash/key-wrap metadata,
//!   DCK generation, and optional causal fields. The event's `created_at`
//!   doubles as `AppKeyRootRef::published_at`.
//! - **`KIND_SHARE_ACCESS_SNAPSHOT = 30078`** — AppKey-signed canonical share
//!   access snapshot. d-tag: bare share UUID; l-tag:
//!   `"iris-drive/share-access"`. Pubkey = authorized share admin `AppKey`.
//! - **AppKey-link requests** — AppKey-signed identity fact events
//!   (`kind=7368`, `type=nostr_identity_link_request`) encrypted to the
//!   random invite key carried by the invite URL.
//!
//! All events are signed by the appropriate key and verify under the
//! event's own pubkey. Build functions return a signed `Event`; parse
//! functions take an `Event`, verify its signature, and extract the
//! application-level type.

use hashtree_core::{Cid, from_hex, to_hex};
use nostr_identity::{
    FACT_OP_KIND, IDENTITY_GRAPH_LINK_REQUEST_TYPE, build_identity_link_request_event,
    parse_identity_link_request_event_for_invite_pubkey,
};
use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{Event, EventBuilder, Keys, Kind, PublicKey, Tag, TagKind};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::NostrIdentityId;
use crate::app_key_link_transport::AppKeyLinkRequestFrame;
use crate::config::AppKeyRootRef;
use crate::root_meta::{RootObservation, RootParent};

/// NIP-78 parameterized-replaceable kind for AppKey-signed drive roots.
pub const KIND_DRIVE_ROOT: u16 = 30078;

/// Standard hashtree mutable-root kind used by drive.iris.to.
const _: () = assert!(hashtree_nostr::HASHTREE_ROOT_KIND <= u16::MAX as u32);
#[allow(clippy::cast_possible_truncation)]
pub const KIND_HASHTREE_ROOT: u16 = hashtree_nostr::HASHTREE_ROOT_KIND as u16;

#[must_use]
pub fn is_drive_root_event_kind(kind: u16) -> bool {
    kind == KIND_DRIVE_ROOT
}

#[must_use]
pub fn is_drive_root_event_coordinate(event: &Event) -> bool {
    is_drive_root_event_kind(event.kind.as_u16())
        && event
            .tags
            .identifier()
            .is_some_and(|d_tag| parse_drive_root_d_tag(d_tag).is_ok())
}

#[must_use]
pub fn is_app_key_link_request_event_coordinate(event: &Event) -> bool {
    is_identity_app_key_link_request_event(event)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriveRootEventPreview {
    pub app_key_pubkey_hex: String,
    pub root_scope_id: String,
    pub drive_id: String,
    pub published_at: i64,
    pub published_at_ms: Option<u64>,
    pub dck_generation: u64,
    pub app_key_seq: u64,
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
    #[error("app-key-link event signer {signer} does not match request device {device}")]
    AppKeyLinkSignerMismatch { signer: String, device: String },
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
    app_key_seq: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    parents: Vec<RootParent>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    observed: std::collections::BTreeMap<String, RootObservation>,
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_zero(value: &u64) -> bool {
    *value == 0
}

pub fn build_app_key_link_request_event(
    device_keys: &Keys,
    frame: &AppKeyLinkRequestFrame,
) -> Result<Event, WireError> {
    PublicKey::from_hex(&frame.app_key_pubkey)
        .map_err(|e| WireError::InvalidPubkey(e.to_string()))?;
    let signer = device_keys.public_key().to_hex();
    if signer != frame.app_key_pubkey {
        return Err(WireError::AppKeyLinkSignerMismatch {
            signer,
            device: frame.app_key_pubkey.clone(),
        });
    }
    PublicKey::from_hex(&frame.admin_app_key_pubkey)
        .map_err(|e| WireError::InvalidPubkey(e.to_string()))?;
    PublicKey::from_hex(&frame.invite_pubkey)
        .map_err(|e| WireError::InvalidPubkey(e.to_string()))?;
    build_identity_link_request_event(
        device_keys,
        frame.profile_id.as_uuid(),
        frame.admin_app_key_pubkey.clone(),
        frame.invite_pubkey.clone(),
        format!("app-key-link-{}", frame.requested_at),
        frame.label.clone(),
        frame.requested_at,
    )
    .map_err(|e| WireError::Event(e.to_string()))
}

/// Parse + verify a signed app-key-link request event.
pub fn parse_app_key_link_request_event(
    event: &Event,
    invite_keys: &Keys,
) -> Result<AppKeyLinkRequestFrame, WireError> {
    if !is_identity_app_key_link_request_event(event) {
        return Err(WireError::WrongKind {
            expected: FACT_OP_KIND,
            got: event.kind.as_u16(),
        });
    }
    parse_identity_app_key_link_request_event(event, invite_keys)
}

fn parse_identity_app_key_link_request_event(
    event: &Event,
    invite_keys: &Keys,
) -> Result<AppKeyLinkRequestFrame, WireError> {
    event
        .verify()
        .map_err(|e| WireError::SignatureFailed(e.to_string()))?;
    let signed = parse_identity_link_request_event_for_invite_pubkey(
        event,
        invite_keys,
        invite_keys.public_key().to_hex(),
    )
    .map_err(|e| WireError::BadContent(format!("Nostr identity link request: {e}")))?;
    let profile_id = NostrIdentityId::from_uuid(signed.content.identity);
    Ok(AppKeyLinkRequestFrame {
        schema: 1,
        profile_id,
        admin_app_key_pubkey: signed.content.admin_pubkey.clone(),
        app_key_pubkey: signed.content.joining_pubkey.clone(),
        invite_pubkey: signed.content.invite_pubkey.clone(),
        label: signed.content.label.clone(),
        requested_at: signed.content.requested_at,
        url: signed.content.joining_pubkey.clone(),
    })
}

fn is_identity_app_key_link_request_event(event: &Event) -> bool {
    event.kind.as_u16() == FACT_OP_KIND
        && event.tags.iter().any(|tag| {
            let fields = tag.as_slice();
            fields.first().is_some_and(|name| name == "type")
                && fields
                    .get(1)
                    .is_some_and(|value| value == IDENTITY_GRAPH_LINK_REQUEST_TYPE)
        })
}

/// Compute the d-tag for a drive-root event.
#[must_use]
pub fn drive_root_d_tag(root_scope_id: &str, drive_id: &str) -> String {
    format!("iris-drive/{root_scope_id}/{drive_id}/root")
}

/// Build a signed drive-root event. Signed by the **app key**;
/// `device_keys.public_key()` becomes the event author, and the
/// merge engine attributes the published root to that app actor.
///
/// This builder preserves `root.published_at` when present so build/parse
/// roundtrips remain stable. Live publishing should use
/// [`build_drive_root_publish_event`] so the replaceable event advances
/// even when the root CID is unchanged.
pub fn build_drive_root_event(
    device_keys: &Keys,
    root_scope_id: &str,
    drive_id: &str,
    root: &AppKeyRootRef,
    authorized_app_key_pubkeys: &[String],
) -> Result<Event, WireError> {
    build_drive_root_event_at(
        device_keys,
        root_scope_id,
        drive_id,
        root,
        authorized_app_key_pubkeys,
        drive_root_timestamp_from_root(root),
        None,
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
    root: &AppKeyRootRef,
    authorized_app_key_pubkeys: &[String],
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
        authorized_app_key_pubkeys,
        ts,
        Some(unix_now_millis().max(ts.saturating_mul(1000))),
    )
}

fn drive_root_timestamp_from_root(root: &AppKeyRootRef) -> u64 {
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

fn unix_now_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

fn build_drive_root_event_at(
    device_keys: &Keys,
    root_scope_id: &str,
    drive_id: &str,
    root: &AppKeyRootRef,
    authorized_app_key_pubkeys: &[String],
    created_at: u64,
    created_at_ms: Option<u64>,
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
        authorized_app_key_pubkeys.iter().cloned().collect();
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
        app_key_seq: root.app_key_seq,
        parents: root.parents.clone(),
        observed: root.observed.clone(),
    };
    let content_json =
        serde_json::to_string(&content).map_err(|e| WireError::BadContent(e.to_string()))?;
    let d_tag = drive_root_d_tag(root_scope_id, drive_id);
    let ms_tag = created_at_ms.unwrap_or_else(|| created_at.saturating_mul(1000));
    let builder = EventBuilder::new(Kind::from(KIND_DRIVE_ROOT), content_json)
        .tag(Tag::identifier(d_tag))
        .tag(Tag::custom(
            TagKind::Custom("ms".into()),
            vec![ms_tag.to_string()],
        ))
        .custom_created_at(nostr_sdk::Timestamp::from(created_at));
    let event = builder
        .sign_with_keys(device_keys)
        .map_err(|e| WireError::Event(e.to_string()))?;
    Ok(event)
}

/// Build a standard private hashtree mutable-root event for drive.iris.to.
///
/// Iris Drive keeps its richer multi-device drive-root event, but the files
/// app already understands standard hashtree root events with `#l=hashtree`, `hash`,
/// and `selfEncryptedKey` tags. This event points `npub/tree_name` at the
/// current root without publishing the root key in cleartext.
pub fn build_private_hashtree_root_event(
    signer_keys: &Keys,
    tree_name: &str,
    root: &AppKeyRootRef,
) -> Result<Event, WireError> {
    let root_cid =
        Cid::parse(&root.root_cid).map_err(|e| WireError::InvalidRootCid(e.to_string()))?;
    let ts = if root.published_at > 0 {
        Some(u64::try_from(root.published_at).unwrap_or(0))
    } else {
        None
    };
    hashtree_nostr::build_private_hashtree_root_event(signer_keys, tree_name, &root_cid, ts)
        .map_err(|e| WireError::Event(e.to_string()))
}

/// Parse + verify a drive-root event. Returns
/// `(app_key_pubkey_hex, root_scope_id, drive_id, AppKeyRootRef)`.
/// The `AppKey` pubkey is the event's author; the root scope id and
/// drive id are extracted from the d-tag.
pub fn parse_drive_root_event(
    event: &Event,
) -> Result<(String, String, String, AppKeyRootRef), WireError> {
    parse_drive_root_event_inner(event, None)
}

pub fn parse_drive_root_event_for_device(
    event: &Event,
    device_keys: &Keys,
) -> Result<(String, String, String, AppKeyRootRef), WireError> {
    parse_drive_root_event_inner(event, Some(device_keys))
}

pub fn parse_drive_root_event_header(event: &Event) -> Result<(String, String, String), WireError> {
    let (app_key_pubkey_hex, root_scope_id, drive_id, _, _) = parse_drive_root_event_parts(event)?;
    Ok((app_key_pubkey_hex, root_scope_id, drive_id))
}

pub fn parse_drive_root_event_preview(event: &Event) -> Result<DriveRootEventPreview, WireError> {
    let (app_key_pubkey_hex, root_scope_id, drive_id, content, published_at) =
        parse_drive_root_event_parts(event)?;
    Ok(DriveRootEventPreview {
        app_key_pubkey_hex,
        root_scope_id,
        drive_id,
        published_at,
        published_at_ms: read_ms_tag(event),
        dck_generation: content.dck_generation,
        app_key_seq: content.app_key_seq,
    })
}

fn parse_drive_root_event_inner(
    event: &Event,
    device_keys: Option<&Keys>,
) -> Result<(String, String, String, AppKeyRootRef), WireError> {
    let (app_key_pubkey_hex, root_scope_id, drive_id, content, published_at) =
        parse_drive_root_event_parts(event)?;
    let root_cid = root_cid_from_wire_content(event, &content, device_keys)?;
    let app_key_root = AppKeyRootRef {
        root_cid,
        published_at,
        dck_generation: content.dck_generation,
        app_key_seq: content.app_key_seq,
        parents: content.parents,
        observed: content.observed,
        local_only: false,
    };
    Ok((app_key_pubkey_hex, root_scope_id, drive_id, app_key_root))
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
    let d_tag = event.tags.identifier().ok_or(WireError::MissingDTag)?;
    let (root_scope_id, drive_id) = parse_drive_root_d_tag(d_tag)?;
    event
        .verify()
        .map_err(|e| WireError::SignatureFailed(e.to_string()))?;
    let content: DriveRootWireContent =
        serde_json::from_str(&event.content).map_err(|e| WireError::BadContent(e.to_string()))?;
    let app_key_pubkey_hex = event.pubkey.to_hex();
    let published_at = i64::try_from(event.created_at.as_secs()).unwrap_or(i64::MAX);
    Ok((
        app_key_pubkey_hex,
        root_scope_id,
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
    let Some((root_scope_id, drive)) = rest.split_once('/') else {
        return Err(WireError::DTagMalformed(format!(
            "missing root-scope/drive separator: {d_tag}"
        )));
    };
    if root_scope_id.is_empty() || drive.is_empty() {
        return Err(WireError::DTagMalformed(format!(
            "empty root-scope or drive id: {d_tag}"
        )));
    }
    Ok((root_scope_id.to_string(), drive.to_string()))
}

fn read_ms_tag(event: &Event) -> Option<u64> {
    event.tags.iter().find_map(|tag| {
        let fields = tag.as_slice();
        if fields.first().is_some_and(|name| name == "ms") {
            fields.get(1)?.parse::<u64>().ok()
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests;
