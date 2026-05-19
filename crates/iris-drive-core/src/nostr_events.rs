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
//!   Pubkey = device pubkey. Content = JSON `{ root_cid, dck_generation }`.
//!   The event's `created_at` doubles as `DeviceRootRef::published_at`.
//!
//! All events are signed by the appropriate key and verify under the
//! event's own pubkey. Build functions return a signed `Event`; parse
//! functions take an `Event`, verify its signature, and extract the
//! application-level type.

use nostr_sdk::{Event, EventBuilder, Keys, Kind, PublicKey, Tag};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::app_keys::{AppKeysSnapshot, DeviceEntry};
use crate::config::DeviceRootRef;

/// NIP-78 parameterized-replaceable kind for owner-signed `AppKeys`.
pub const KIND_APP_KEYS: u16 = 30078;

/// NIP-78 parameterized-replaceable kind for device-signed drive roots.
pub const KIND_DRIVE_ROOT: u16 = 30079;

pub const D_TAG_APP_KEYS: &str = "iris-drive/app-keys";

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
    root_cid: String,
    dck_generation: u64,
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
) -> Result<Event, WireError> {
    let content = DriveRootWireContent {
        root_cid: root.root_cid.clone(),
        dck_generation: root.dck_generation,
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

/// Parse + verify a drive-root event. Returns
/// `(device_pubkey_hex, owner_pubkey_hex, drive_id, DeviceRootRef)`.
/// The device pubkey is the event's author; the owner pubkey and
/// drive id are extracted from the d-tag.
pub fn parse_drive_root_event(
    event: &Event,
) -> Result<(String, String, String, DeviceRootRef), WireError> {
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
    PublicKey::from_hex(&owner_pubkey_hex)
        .map_err(|e| WireError::InvalidPubkey(e.to_string()))?;
    event
        .verify()
        .map_err(|e| WireError::SignatureFailed(e.to_string()))?;
    let content: DriveRootWireContent =
        serde_json::from_str(&event.content).map_err(|e| WireError::BadContent(e.to_string()))?;
    let device_pubkey_hex = event.pubkey.to_hex();
    let device_root = DeviceRootRef {
        root_cid: content.root_cid,
        published_at: i64::try_from(event.created_at.as_u64()).unwrap_or(i64::MAX),
        dck_generation: content.dck_generation,
    };
    Ok((device_pubkey_hex, owner_pubkey_hex, drive_id, device_root))
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
            wrapped_dck: BTreeMap::from([
                ("ab".repeat(32), "base64ciphertext".into()),
            ]),
        }
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
        let content =
            serde_json::to_string(&AppKeysWireContent {
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
        let root = DeviceRootRef {
            root_cid: "0123abcdef".into(),
            // Set an explicit published_at so roundtrip is stable.
            published_at: 1_700_000_000,
            dck_generation: 7,
        };
        let event = build_drive_root_event(&device, &owner_hex, "main", &root).unwrap();
        let (device_pk, parsed_owner, drive_id, parsed_root) =
            parse_drive_root_event(&event).unwrap();
        assert_eq!(device_pk, device.public_key().to_hex());
        assert_eq!(parsed_owner, owner_hex);
        assert_eq!(drive_id, "main");
        assert_eq!(parsed_root.root_cid, root.root_cid);
        assert_eq!(parsed_root.dck_generation, root.dck_generation);
        assert_eq!(parsed_root.published_at, root.published_at);
    }

    #[test]
    fn drive_root_event_with_zero_published_at_falls_back_to_wall_clock() {
        let device = Keys::generate();
        let owner = Keys::generate().public_key().to_hex();
        let root = DeviceRootRef {
            root_cid: "x".into(),
            published_at: 0, // caller hasn't stamped — should fall back
            dck_generation: 1,
        };
        let event = build_drive_root_event(&device, &owner, "main", &root).unwrap();
        let (_, _, _, parsed_root) = parse_drive_root_event(&event).unwrap();
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
            assert!(
                parse_drive_root_d_tag(bad).is_err(),
                "should reject {bad}"
            );
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
        let root = DeviceRootRef {
            root_cid: "x".into(),
            published_at: 0,
            dck_generation: 1,
        };
        let ev_a = build_drive_root_event(&device_a, &owner, "main", &root).unwrap();
        let ev_b = build_drive_root_event(&device_b, &owner, "main", &root).unwrap();
        let (pk_a, _, _, _) = parse_drive_root_event(&ev_a).unwrap();
        let (pk_b, _, _, _) = parse_drive_root_event(&ev_b).unwrap();
        assert_eq!(pk_a, device_a.public_key().to_hex());
        assert_eq!(pk_b, device_b.public_key().to_hex());
        assert_ne!(pk_a, pk_b);
    }
}
