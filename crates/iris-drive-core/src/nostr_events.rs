//! Nostr wire format for Iris Drive events.
//!
//! Two replaceable event kinds (NIP-78 parameterized-replaceable range):
//!
//! - **`KIND_APP_KEYS = 30078`** — owner-signed `AppKeys` roster.
//!   d-tag: `"iris-drive/app-keys"`. Pubkey = owner pubkey. Content = JSON
//!   `{ devices, dck_generation, wrapped_dck }`. The event's `created_at`
//!   doubles as the snapshot's `created_at`.
//!
//! - **`KIND_DRIVE_ROOT = 30079`** — device-signed drive-root reference.
//!   d-tag: `"iris-drive/<owner_pubkey_hex>/<drive_id>/root"`.
//!   Pubkey = device pubkey. Content = JSON root hash/key-wrap metadata,
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

use crate::app_keys::{AppKeysSnapshot, DeviceEntry};
use crate::config::DeviceRootRef;
use crate::root_meta::{RootObservation, RootParent};

/// NIP-78 parameterized-replaceable kind for owner-signed `AppKeys`.
pub const KIND_APP_KEYS: u16 = 30078;

/// NIP-78 parameterized-replaceable kind for device-signed drive roots.
pub const KIND_DRIVE_ROOT: u16 = 30079;

/// Standard hashtree mutable-root kind used by drive.iris.to.
pub const KIND_HASHTREE_ROOT: u16 = 30_078;
const _: () = assert!(hashtree_nostr::HASHTREE_ROOT_KIND == 30_078);

pub const D_TAG_APP_KEYS: &str = "iris-drive/app-keys";

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
}

#[derive(Debug, Serialize, Deserialize)]
struct AppKeysWireContent {
    devices: Vec<DeviceEntry>,
    #[serde(default)]
    dck_generation: u64,
    #[serde(default)]
    wrapped_dck: std::collections::BTreeMap<String, String>,
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

/// Build a signed `AppKeys` event from a snapshot. The owner key must
/// match `snapshot.owner_pubkey`; passing a key for someone else
/// produces an event the application layer will then ignore via the
/// `AppKeys` allowlist check.
pub fn build_app_keys_event(
    owner_keys: &Keys,
    snapshot: &AppKeysSnapshot,
) -> Result<Event, WireError> {
    let content = AppKeysWireContent {
        devices: snapshot.devices.clone(),
        dck_generation: snapshot.dck_generation,
        wrapped_dck: snapshot.wrapped_dck.clone(),
    };
    let content_json =
        serde_json::to_string(&content).map_err(|e| WireError::BadContent(e.to_string()))?;
    // The snapshot's created_at IS the canonical timestamp for the
    // event; preserve it across build/parse so applying the same
    // snapshot produces a stable, idempotent result.
    let ts = u64::try_from(snapshot.created_at).unwrap_or(0);
    let builder = EventBuilder::new(
        Kind::from(KIND_APP_KEYS),
        content_json,
        [Tag::identifier(D_TAG_APP_KEYS)],
    )
    .custom_created_at(nostr_sdk::Timestamp::from(ts));
    let event = builder
        .to_event(owner_keys)
        .map_err(|e| WireError::Event(e.to_string()))?;
    Ok(event)
}

/// Parse + verify an `AppKeys` event into a snapshot. The event must
/// have the right kind, the `iris-drive/app-keys` d-tag, and a valid
/// signature. The snapshot's `owner_pubkey` is the event's author.
pub fn parse_app_keys_event(event: &Event) -> Result<AppKeysSnapshot, WireError> {
    let kind = event.kind.as_u16();
    if kind != KIND_APP_KEYS {
        return Err(WireError::WrongKind {
            expected: KIND_APP_KEYS,
            got: kind,
        });
    }
    let d_tag = event.identifier().ok_or(WireError::MissingDTag)?;
    if d_tag != D_TAG_APP_KEYS {
        return Err(WireError::DTagMalformed(format!(
            "expected {D_TAG_APP_KEYS}, got {d_tag}"
        )));
    }
    event
        .verify()
        .map_err(|e| WireError::SignatureFailed(e.to_string()))?;
    let content: AppKeysWireContent =
        serde_json::from_str(&event.content).map_err(|e| WireError::BadContent(e.to_string()))?;
    let snapshot = AppKeysSnapshot {
        owner_pubkey: event.pubkey.to_hex(),
        created_at: i64::try_from(event.created_at.as_u64()).unwrap_or(i64::MAX),
        devices: content.devices,
        dck_generation: content.dck_generation,
        wrapped_dck: content.wrapped_dck,
    };
    Ok(snapshot)
}

/// Compute the d-tag for a drive-root event.
#[must_use]
pub fn drive_root_d_tag(owner_pubkey_hex: &str, drive_id: &str) -> String {
    format!("iris-drive/{owner_pubkey_hex}/{drive_id}/root")
}

/// Build a signed drive-root event. Signed by the **device key**;
/// `device_keys.public_key()` becomes the event author, and the
/// merge engine attributes the published root to that device. The
/// `published_at` field of the resulting `DeviceRootRef` is set to
/// the moment this event was built, by way of the event's
/// `created_at`.
pub fn build_drive_root_event(
    device_keys: &Keys,
    owner_pubkey_hex: &str,
    drive_id: &str,
    root: &DeviceRootRef,
    authorized_device_pubkeys: &[String],
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
    let d_tag = drive_root_d_tag(owner_pubkey_hex, drive_id);
    // If the caller hasn't set published_at, fall back to wall-clock
    // now so the event carries a meaningful timestamp; otherwise echo
    // the application-level value to keep build/parse stable.
    let ts = if root.published_at > 0 {
        u64::try_from(root.published_at).unwrap_or(0)
    } else {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    };
    let builder = EventBuilder::new(
        Kind::from(KIND_DRIVE_ROOT),
        content_json,
        [Tag::identifier(d_tag)],
    )
    .custom_created_at(nostr_sdk::Timestamp::from(ts));
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
/// `(device_pubkey_hex, owner_pubkey_hex, drive_id, DeviceRootRef)`.
/// The device pubkey is the event's author; the owner pubkey and
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
        materialized_only: false,
    };
    Ok((device_pubkey_hex, owner_pubkey_hex, drive_id, device_root))
}

fn parse_drive_root_event_parts(
    event: &Event,
) -> Result<(String, String, String, DriveRootWireContent, i64), WireError> {
    let kind = event.kind.as_u16();
    if kind != KIND_DRIVE_ROOT {
        return Err(WireError::WrongKind {
            expected: KIND_DRIVE_ROOT,
            got: kind,
        });
    }
    let d_tag = event.identifier().ok_or(WireError::MissingDTag)?;
    let (owner_pubkey_hex, drive_id) = parse_drive_root_d_tag(d_tag)?;
    // Sanity-check the owner pubkey is well-formed before trusting it.
    PublicKey::from_hex(&owner_pubkey_hex).map_err(|e| WireError::InvalidPubkey(e.to_string()))?;
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
    // expected: iris-drive/<owner_pubkey>/<drive_id>/root
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
mod tests {
    use super::*;
    use nostr_sdk::JsonUtil;
    use std::collections::BTreeMap;

    fn fake_snapshot(owner_pubkey: &str) -> AppKeysSnapshot {
        AppKeysSnapshot {
            owner_pubkey: owner_pubkey.to_string(),
            created_at: 1_700_000_000,
            devices: vec![DeviceEntry {
                pubkey: "ab".repeat(32),
                added_at: 1_699_000_000,
                label: Some("Mac mini".into()),
            }],
            dck_generation: 5,
            wrapped_dck: BTreeMap::from([("ab".repeat(32), "base64ciphertext".into())]),
        }
    }

    fn tag_value(event: &Event, tag_name: &str) -> Option<String> {
        event.tags.iter().find_map(|tag| {
            let fields = tag.as_slice();
            if fields.first().is_some_and(|name| name == tag_name) {
                fields.get(1).cloned()
            } else {
                None
            }
        })
    }

    #[test]
    fn app_keys_event_roundtrip() {
        let owner = Keys::generate();
        let snap = fake_snapshot(&owner.public_key().to_hex());
        let event = build_app_keys_event(&owner, &snap).unwrap();
        let parsed = parse_app_keys_event(&event).unwrap();

        // owner_pubkey comes from the event author.
        assert_eq!(parsed.owner_pubkey, owner.public_key().to_hex());
        // The snapshot's created_at IS the event's created_at — round-trip stable.
        assert_eq!(parsed.created_at, snap.created_at);
        assert_eq!(parsed.devices, snap.devices);
        assert_eq!(parsed.dck_generation, snap.dck_generation);
        assert_eq!(parsed.wrapped_dck, snap.wrapped_dck);
    }

    #[test]
    fn event_author_attributes_to_actual_signer() {
        // Confirm the wire-format accurately reports who signed an event.
        // The application's AppKeys allowlist is what then rejects events
        // from non-owners — the parse step doesn't filter; it just
        // surfaces the signer faithfully.
        let owner = Keys::generate();
        let snap = fake_snapshot(&owner.public_key().to_hex());
        let attacker = Keys::generate();
        let attacker_event = build_app_keys_event(&attacker, &snap).unwrap();
        let parsed = parse_app_keys_event(&attacker_event).unwrap();
        assert_ne!(parsed.owner_pubkey, owner.public_key().to_hex());
        assert_eq!(parsed.owner_pubkey, attacker.public_key().to_hex());
    }

    #[test]
    fn app_keys_event_wrong_kind_rejected() {
        let owner = Keys::generate();
        let other_kind_event = EventBuilder::new(
            Kind::from(1u16),
            "{}".to_string(),
            [Tag::identifier(D_TAG_APP_KEYS)],
        )
        .to_event(&owner)
        .unwrap();
        match parse_app_keys_event(&other_kind_event) {
            Err(WireError::WrongKind { expected, got }) => {
                assert_eq!(expected, KIND_APP_KEYS);
                assert_eq!(got, 1);
            }
            other => panic!("expected WrongKind, got {other:?}"),
        }
    }

    #[test]
    fn app_keys_event_missing_d_tag_rejected() {
        let owner = Keys::generate();
        let snap = fake_snapshot(&owner.public_key().to_hex());
        let content = serde_json::to_string(&AppKeysWireContent {
            devices: snap.devices.clone(),
            dck_generation: snap.dck_generation,
            wrapped_dck: snap.wrapped_dck.clone(),
        })
        .unwrap();
        let event = EventBuilder::new(Kind::from(KIND_APP_KEYS), content, [])
            .to_event(&owner)
            .unwrap();
        match parse_app_keys_event(&event) {
            Err(WireError::MissingDTag) => {}
            other => panic!("expected MissingDTag, got {other:?}"),
        }
    }

    #[test]
    fn drive_root_event_roundtrip() {
        let device = Keys::generate();
        let owner = Keys::generate();
        let owner_hex = owner.public_key().to_hex();
        let authorized_devices = vec![device.public_key().to_hex()];
        let root = DeviceRootRef::legacy(
            Cid::encrypted([0x12; 32], [0x34; 32]).to_string(),
            // Set an explicit published_at so roundtrip is stable.
            1_700_000_000,
            7,
        );
        let event = build_drive_root_event(&device, &owner_hex, "main", &root, &authorized_devices)
            .unwrap();
        let (device_pk, parsed_owner, drive_id, parsed_root) =
            parse_drive_root_event_for_device(&event, &device).unwrap();
        assert_eq!(device_pk, device.public_key().to_hex());
        assert_eq!(parsed_owner, owner_hex);
        assert_eq!(drive_id, "main");
        assert_eq!(parsed_root.root_cid, root.root_cid);
        assert_eq!(parsed_root.dck_generation, root.dck_generation);
        assert_eq!(parsed_root.published_at, root.published_at);
    }

    #[test]
    fn drive_root_event_roundtrip_preserves_causal_fields() {
        let device = Keys::generate();
        let owner = Keys::generate();
        let owner_hex = owner.public_key().to_hex();
        let parent = RootParent {
            device_id: device.public_key().to_hex(),
            device_seq: 2,
            root_cid: Cid::encrypted([0x20; 32], [0x21; 32]).to_string(),
        };
        let observed_device = Keys::generate().public_key().to_hex();
        let observed = BTreeMap::from([(
            observed_device.clone(),
            RootObservation {
                device_seq: 9,
                root_cid: Cid::encrypted([0x30; 32], [0x31; 32]).to_string(),
            },
        )]);
        let root = DeviceRootRef {
            root_cid: Cid::encrypted([0x12; 32], [0x34; 32]).to_string(),
            published_at: 1_700_000_000,
            dck_generation: 7,
            device_seq: 3,
            parents: vec![parent.clone()],
            observed: observed.clone(),
            materialized_only: false,
        };

        let event = build_drive_root_event(
            &device,
            &owner_hex,
            "main",
            &root,
            &[device.public_key().to_hex(), observed_device],
        )
        .unwrap();
        let (_, _, _, parsed_root) = parse_drive_root_event_for_device(&event, &device).unwrap();
        assert_eq!(parsed_root.device_seq, 3);
        assert_eq!(parsed_root.parents, vec![parent]);
        assert_eq!(parsed_root.observed, observed);
    }

    #[test]
    fn drive_root_event_does_not_publish_root_key_in_cleartext() {
        let device = Keys::generate();
        let owner = Keys::generate().public_key().to_hex();
        let root_key = [0x44; 32];
        let root = DeviceRootRef::legacy(
            Cid::encrypted([0x33; 32], root_key).to_string(),
            1_700_000_000,
            1,
        );

        let event = build_drive_root_event(
            &device,
            &owner,
            "main",
            &root,
            &[device.public_key().to_hex()],
        )
        .unwrap();

        assert!(!event.content.contains(&root.root_cid));
        assert!(!event.content.contains(&hex::encode(root_key)));
        assert!(parse_drive_root_event(&event).is_err());

        let (_, _, _, parsed_root) = parse_drive_root_event_for_device(&event, &device).unwrap();
        assert_eq!(parsed_root.root_cid, root.root_cid);
    }

    #[test]
    fn private_hashtree_root_event_is_files_app_compatible() {
        let owner = Keys::generate();
        let root_key = [0x44; 32];
        let root_hash = [0x33; 32];
        let root = DeviceRootRef::legacy(
            Cid::encrypted(root_hash, root_key).to_string(),
            1_700_000_000,
            1,
        );

        let event = build_private_hashtree_root_event(&owner, "main", &root).unwrap();
        assert_eq!(event.kind.as_u16(), 30078);
        assert_eq!(event.pubkey, owner.public_key());
        assert_eq!(event.identifier(), Some("main"));
        assert_eq!(event.content, "");
        assert_eq!(tag_value(&event, "l").as_deref(), Some("hashtree"));
        assert_eq!(tag_value(&event, "hash"), Some(hex::encode(root_hash)));
        assert!(tag_value(&event, "key").is_none());
        assert!(!event.as_json().contains(&hex::encode(root_key)));

        let parsed = hashtree_nostr::parse_verified_hashtree_root_event(&event)
            .unwrap()
            .unwrap();
        let resolved = hashtree_nostr::resolve_self_encrypted_root_cid(&parsed, &owner).unwrap();
        assert_eq!(parsed.event.pubkey, owner.public_key().to_hex());
        assert_eq!(parsed.tree_name, "main");
        assert_eq!(resolved.to_string(), root.root_cid);
    }

    #[test]
    fn drive_root_event_builder_rejects_unencrypted_root() {
        let device = Keys::generate();
        let owner = Keys::generate().public_key().to_hex();
        let root = DeviceRootRef::legacy(Cid::public([0x11; 32]).to_string(), 1_700_000_000, 1);

        assert!(
            build_drive_root_event(
                &device,
                &owner,
                "main",
                &root,
                &[device.public_key().to_hex()]
            )
            .is_err()
        );
    }

    #[test]
    fn drive_root_event_builder_always_wraps_for_signing_device() {
        let device = Keys::generate();
        let owner = Keys::generate().public_key().to_hex();
        let root = DeviceRootRef::legacy(
            Cid::encrypted([0x22; 32], [0x33; 32]).to_string(),
            1_700_000_000,
            1,
        );

        let event = build_drive_root_event(&device, &owner, "main", &root, &[]).unwrap();
        let (_, _, _, parsed_root) = parse_drive_root_event_for_device(&event, &device).unwrap();
        assert_eq!(parsed_root.root_cid, root.root_cid);
    }

    #[test]
    fn drive_root_event_with_zero_published_at_falls_back_to_wall_clock() {
        let device = Keys::generate();
        let owner = Keys::generate().public_key().to_hex();
        let root = DeviceRootRef::legacy(
            Cid::encrypted([0x56; 32], [0x78; 32]).to_string(),
            0, // caller hasn't stamped — should fall back
            1,
        );
        let event = build_drive_root_event(
            &device,
            &owner,
            "main",
            &root,
            &[device.public_key().to_hex()],
        )
        .unwrap();
        let (_, _, _, parsed_root) = parse_drive_root_event_for_device(&event, &device).unwrap();
        // Should be roughly now, not 0.
        assert!(parsed_root.published_at > 1_500_000_000);
    }

    #[test]
    fn drive_root_d_tag_format() {
        let owner = "aa".repeat(32);
        let tag = drive_root_d_tag(&owner, "main");
        assert_eq!(tag, format!("iris-drive/{owner}/main/root"));
    }

    #[test]
    fn drive_root_d_tag_parse_round_trip() {
        let owner = "bb".repeat(32);
        let drive_id = "shared-photos";
        let tag = drive_root_d_tag(&owner, drive_id);
        let (parsed_owner, parsed_drive) = parse_drive_root_d_tag(&tag).unwrap();
        assert_eq!(parsed_owner, owner);
        assert_eq!(parsed_drive, drive_id);
    }

    #[test]
    fn drive_root_d_tag_malformed_rejected() {
        for bad in &[
            "wrong-prefix/abc/main/root",
            "iris-drive/abc/main",
            "iris-drive//main/root",
            "iris-drive/abc//root",
            "iris-drive/abc",
        ] {
            assert!(parse_drive_root_d_tag(bad).is_err(), "should reject {bad}");
        }
    }

    #[test]
    fn drive_root_event_wrong_kind_rejected() {
        let device = Keys::generate();
        let other = EventBuilder::new(
            Kind::from(1u16),
            "{}".to_string(),
            [Tag::identifier(drive_root_d_tag(
                &device.public_key().to_hex(),
                "main",
            ))],
        )
        .to_event(&device)
        .unwrap();
        assert!(matches!(
            parse_drive_root_event(&other),
            Err(WireError::WrongKind { .. })
        ));
    }

    #[test]
    fn drive_root_event_attributes_to_device_signer() {
        // Important property: even if two devices publish for the same
        // owner+drive, the event's author is the device pubkey, so the
        // merge engine can attribute each root to the right device.
        let device_a = Keys::generate();
        let device_b = Keys::generate();
        let owner = Keys::generate().public_key().to_hex();
        let root = DeviceRootRef::legacy(Cid::encrypted([0x88; 32], [0x99; 32]).to_string(), 0, 1);
        let ev_a = build_drive_root_event(
            &device_a,
            &owner,
            "main",
            &root,
            &[device_a.public_key().to_hex()],
        )
        .unwrap();
        let ev_b = build_drive_root_event(
            &device_b,
            &owner,
            "main",
            &root,
            &[device_b.public_key().to_hex()],
        )
        .unwrap();
        let (pk_a, _, _, _) = parse_drive_root_event_for_device(&ev_a, &device_a).unwrap();
        let (pk_b, _, _, _) = parse_drive_root_event_for_device(&ev_b, &device_b).unwrap();
        assert_eq!(pk_a, device_a.public_key().to_hex());
        assert_eq!(pk_b, device_b.public_key().to_hex());
        assert_ne!(pk_a, pk_b);
    }
}
