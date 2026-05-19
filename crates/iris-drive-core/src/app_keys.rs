//! Owner-signed device roster.
//!
//! Iris Drive uses a small in-house variant of the `AppKeys` pattern from
//! `~/src/nostr-double-ratchet`: one owner key, multiple device keys,
//! and a replaceable, owner-signed snapshot listing the authorized
//! devices. This module owns the snapshot **data model and timeline
//! rules** — wire format (Nostr event kind, `d` tag, NIP-44 envelope)
//! is the publishing layer's problem, not this module's.
//!
//! Timeline rules (from nostr-double-ratchet's published guidance):
//!
//! - Order snapshots by `created_at` (Nostr publish time).
//! - Newer fully replaces older.
//! - Same-second collisions merge **monotonically** — the union of
//!   devices is taken, never the intersection. This prevents two
//!   owner-capable devices from racing each other into revoking each
//!   other's pending additions.
//! - A reduced set (fewer devices) is **only** valid when the new
//!   snapshot is strictly newer in time; same-second can never shrink.
//! - First-device bootstrap may publish a single-entry snapshot freely.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One authorized device.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceEntry {
    /// Hex-encoded device pubkey.
    pub pubkey: String,
    /// When this device was first added (unix seconds).
    pub added_at: i64,
    /// Optional human-readable label (e.g. "Mac mini").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A complete, owner-signed roster snapshot. Replaceable by `created_at`.
///
/// Carries the current drive content key (DCK) NIP-44–wrapped to each
/// authorized device's pubkey. Rotating the DCK on every membership
/// change gives forward secrecy against device revocation: a revoked
/// device retains anything it already downloaded but cannot decrypt the
/// drive's new root once the next rotation lands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppKeysSnapshot {
    pub owner_pubkey: String,
    pub created_at: i64,
    pub devices: Vec<DeviceEntry>,
    /// Monotonically-increasing counter. Bumped each time the DCK
    /// rotates (on approve, revoke, or explicit `rotate_dck`).
    #[serde(default)]
    pub dck_generation: u64,
    /// NIP-44 wraps of the current DCK, keyed by device pubkey hex.
    /// Encrypted by the owner secret to each device's pubkey. Devices
    /// not present in the map are effectively revoked from the current
    /// content.
    #[serde(default)]
    pub wrapped_dck: BTreeMap<String, String>,
}

impl AppKeysSnapshot {
    /// Sort device list deterministically (by pubkey). Use after merges
    /// so equality checks and serialization are stable.
    pub fn normalize(&mut self) {
        self.devices.sort_by(|a, b| a.pubkey.cmp(&b.pubkey));
        self.devices.dedup_by(|a, b| a.pubkey == b.pubkey);
    }

    #[must_use]
    pub fn contains(&self, device_pubkey: &str) -> bool {
        self.devices.iter().any(|d| d.pubkey == device_pubkey)
    }

    #[must_use]
    pub fn device(&self, pubkey: &str) -> Option<&DeviceEntry> {
        self.devices.iter().find(|d| d.pubkey == pubkey)
    }
}

/// Outcome of applying an incoming snapshot to the current one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyDecision {
    /// No current snapshot; incoming becomes current.
    Adopted,
    /// Incoming is strictly newer and fully replaces current.
    Replaced,
    /// Incoming is same-second; the union of devices is taken.
    Merged,
    /// Incoming is older or from a different owner; ignored.
    Rejected,
}

/// Apply `incoming` against `current` per the timeline rules.
///
/// `current` is mutated in-place when accepted/merged; returned
/// decision tells callers whether to log the change or fan it out.
pub fn apply_snapshot(
    current: &mut Option<AppKeysSnapshot>,
    incoming: AppKeysSnapshot,
) -> ApplyDecision {
    match current.as_mut() {
        None => {
            let mut snap = incoming;
            snap.normalize();
            *current = Some(snap);
            ApplyDecision::Adopted
        }
        Some(existing) => {
            if existing.owner_pubkey != incoming.owner_pubkey {
                return ApplyDecision::Rejected;
            }
            match incoming.created_at.cmp(&existing.created_at) {
                std::cmp::Ordering::Greater => {
                    let mut snap = incoming;
                    snap.normalize();
                    *existing = snap;
                    ApplyDecision::Replaced
                }
                std::cmp::Ordering::Equal => {
                    merge_same_second(existing, &incoming);
                    ApplyDecision::Merged
                }
                std::cmp::Ordering::Less => ApplyDecision::Rejected,
            }
        }
    }
}

/// Same-second additive merge: union by pubkey, earliest `added_at`
/// wins per device, labels keep first non-empty.
fn merge_same_second(existing: &mut AppKeysSnapshot, incoming: &AppKeysSnapshot) {
    let mut by_pubkey: BTreeMap<String, DeviceEntry> = BTreeMap::new();
    for d in existing.devices.iter().chain(incoming.devices.iter()) {
        by_pubkey
            .entry(d.pubkey.clone())
            .and_modify(|cur| {
                if d.added_at < cur.added_at {
                    cur.added_at = d.added_at;
                }
                if cur.label.is_none() {
                    cur.label.clone_from(&d.label);
                }
            })
            .or_insert_with(|| d.clone());
    }
    existing.devices = by_pubkey.into_values().collect();
    existing.normalize();
}

/// Convenience: select the latest snapshot from an iterator of
/// snapshots. Useful when collecting from multiple relays. Same-second
/// snapshots get merged additively.
pub fn select_latest<I: IntoIterator<Item = AppKeysSnapshot>>(
    snapshots: I,
    owner_pubkey: &str,
) -> Option<AppKeysSnapshot> {
    let mut current = None;
    for snap in snapshots {
        if snap.owner_pubkey == owner_pubkey {
            apply_snapshot(&mut current, snap);
        }
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(owner: &str, created_at: i64, devices: &[(&str, i64)]) -> AppKeysSnapshot {
        AppKeysSnapshot {
            owner_pubkey: owner.into(),
            created_at,
            devices: devices
                .iter()
                .map(|(pk, added)| DeviceEntry {
                    pubkey: (*pk).into(),
                    added_at: *added,
                    label: None,
                })
                .collect(),
            dck_generation: 0,
            wrapped_dck: BTreeMap::new(),
        }
    }

    #[test]
    fn adopts_first_snapshot() {
        let mut current = None;
        let s = snap("owner", 100, &[("dev-a", 100)]);
        assert_eq!(
            apply_snapshot(&mut current, s.clone()),
            ApplyDecision::Adopted
        );
        assert_eq!(current.as_ref().unwrap().devices.len(), 1);
    }

    #[test]
    fn newer_snapshot_replaces() {
        let mut current = Some(snap("owner", 100, &[("dev-a", 100)]));
        let next = snap("owner", 200, &[("dev-a", 100), ("dev-b", 200)]);
        assert_eq!(
            apply_snapshot(&mut current, next),
            ApplyDecision::Replaced
        );
        let s = current.unwrap();
        assert_eq!(s.created_at, 200);
        assert_eq!(s.devices.len(), 2);
    }

    #[test]
    fn older_snapshot_rejected() {
        let mut current = Some(snap("owner", 200, &[("dev-a", 100), ("dev-b", 200)]));
        let stale = snap("owner", 100, &[("dev-a", 100)]);
        assert_eq!(
            apply_snapshot(&mut current, stale),
            ApplyDecision::Rejected
        );
        assert_eq!(current.unwrap().devices.len(), 2);
    }

    #[test]
    fn same_second_merges_additively() {
        let mut current = Some(snap("owner", 200, &[("dev-a", 100)]));
        let racing = snap("owner", 200, &[("dev-b", 200)]);
        assert_eq!(
            apply_snapshot(&mut current, racing),
            ApplyDecision::Merged
        );
        let s = current.unwrap();
        assert_eq!(s.devices.len(), 2);
        assert!(s.contains("dev-a"));
        assert!(s.contains("dev-b"));
    }

    #[test]
    fn same_second_reduced_set_still_keeps_existing() {
        // Two owner-capable devices race; each thinks the other shouldn't
        // exist. Without monotonic merge, one would silently revoke the
        // other.
        let mut current = Some(snap("owner", 200, &[("dev-a", 100), ("dev-b", 150)]));
        let reduced = snap("owner", 200, &[("dev-a", 100)]); // omits dev-b
        assert_eq!(
            apply_snapshot(&mut current, reduced),
            ApplyDecision::Merged
        );
        let s = current.unwrap();
        assert_eq!(s.devices.len(), 2, "dev-b must not be silently revoked");
        assert!(s.contains("dev-b"));
    }

    #[test]
    fn newer_snapshot_can_legitimately_reduce_set() {
        let mut current = Some(snap("owner", 200, &[("dev-a", 100), ("dev-b", 200)]));
        let revoke = snap("owner", 300, &[("dev-a", 100)]);
        assert_eq!(
            apply_snapshot(&mut current, revoke),
            ApplyDecision::Replaced
        );
        let s = current.unwrap();
        assert_eq!(s.devices.len(), 1);
        assert!(!s.contains("dev-b"));
    }

    #[test]
    fn different_owner_rejected() {
        let mut current = Some(snap("owner-a", 100, &[("dev-a", 100)]));
        let foreign = snap("owner-b", 200, &[("dev-x", 200)]);
        assert_eq!(
            apply_snapshot(&mut current, foreign),
            ApplyDecision::Rejected
        );
        assert_eq!(current.unwrap().owner_pubkey, "owner-a");
    }

    #[test]
    fn merge_keeps_earliest_added_at_per_device() {
        let mut current = Some(snap("owner", 200, &[("dev-a", 100)]));
        let mut variant = snap("owner", 200, &[("dev-a", 50)]);
        // dev-a actually first appeared earlier than current's record
        variant.devices[0].added_at = 50;
        apply_snapshot(&mut current, variant);
        assert_eq!(current.unwrap().devices[0].added_at, 50);
    }

    #[test]
    fn select_latest_collapses_relay_set() {
        let s1 = snap("owner", 100, &[("dev-a", 100)]);
        let s2 = snap("owner", 300, &[("dev-a", 100), ("dev-b", 300)]);
        let s3 = snap("owner", 200, &[("dev-a", 100), ("dev-c", 200)]);
        let result = select_latest(vec![s1, s2, s3], "owner").unwrap();
        assert_eq!(result.created_at, 300);
        assert_eq!(result.devices.len(), 2);
        assert!(result.contains("dev-b"));
        assert!(!result.contains("dev-c"));
    }

    #[test]
    fn select_latest_filters_foreign_owners() {
        let mine = snap("owner", 100, &[("dev-a", 100)]);
        let other = snap("attacker", 999, &[("evil", 100)]);
        let result = select_latest(vec![other, mine], "owner").unwrap();
        assert_eq!(result.owner_pubkey, "owner");
        assert_eq!(result.created_at, 100);
    }

    #[test]
    fn normalize_dedupes_and_sorts() {
        let mut s = snap("owner", 100, &[("z", 100), ("a", 100), ("a", 200)]);
        s.normalize();
        assert_eq!(
            s.devices.iter().map(|d| d.pubkey.as_str()).collect::<Vec<_>>(),
            vec!["a", "z"]
        );
    }

    #[test]
    fn round_trip_through_toml() {
        let s = AppKeysSnapshot {
            owner_pubkey: "abc123".into(),
            created_at: 1_700_000_000,
            devices: vec![DeviceEntry {
                pubkey: "dev1".into(),
                added_at: 1_699_000_000,
                label: Some("Mac mini".into()),
            }],
            dck_generation: 1,
            wrapped_dck: BTreeMap::from([("dev1".to_string(), "abcdef".to_string())]),
        };
        let serialized = toml::to_string(&s).unwrap();
        let back: AppKeysSnapshot = toml::from_str(&serialized).unwrap();
        assert_eq!(back, s);
    }
}
