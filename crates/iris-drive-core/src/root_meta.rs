//! Root-level causal metadata for snapshot-first sync.
//!
//! The signed drive root remains the canonical truth. This metadata
//! travels inside `.hashtree/root.json` to explain the snapshot's
//! ancestry and per-AppKey observations without making the drive
//! depend on operation-log replay.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Compare causal root references by their block identity.
///
/// Relay metadata may intentionally carry only the public root hash so it can
/// describe ancestry without disclosing the encrypted root capability. A full
/// CID (`hash:key`) and its hash-only form therefore identify the same root for
/// causality, even though only the full CID can be used to read blocks.
#[must_use]
pub fn root_cid_identity_matches(left: &str, right: &str) -> bool {
    if left == right {
        return true;
    }
    hashtree_core::Cid::parse(left)
        .ok()
        .zip(hashtree_core::Cid::parse(right).ok())
        .is_some_and(|(left, right)| left.hash == right.hash)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootParent {
    #[serde(alias = "device_id")]
    pub app_key_pubkey: String,
    #[serde(alias = "device_seq")]
    pub app_key_seq: u64,
    pub root_cid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootObservation {
    #[serde(alias = "device_seq")]
    pub app_key_seq: u64,
    pub root_cid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriveRootMeta {
    pub schema: u16,
    pub drive_id: String,
    #[serde(alias = "device_id")]
    pub app_key_pubkey: String,
    #[serde(alias = "device_seq")]
    pub app_key_seq: u64,
    pub dck_generation: u64,
    /// Local bookkeeping root that should not be announced as this
    /// `AppKey`'s own edit.
    #[serde(default, skip_serializing_if = "is_false")]
    pub local_only: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parents: Vec<RootParent>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub observed: BTreeMap<String, RootObservation>,
    pub created_at: i64,
}

impl DriveRootMeta {
    pub const SCHEMA: u16 = 1;
}

#[allow(clippy::trivially_copy_pass_by_ref)]
fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use super::root_cid_identity_matches;
    use hashtree_core::Cid;

    #[test]
    fn encrypted_cid_matches_its_hash_only_causal_reference() {
        let root = Cid::encrypted([0x42; 32], [0x99; 32]);

        assert!(root_cid_identity_matches(
            &root.to_string(),
            &hashtree_core::to_hex(&root.hash),
        ));
        assert!(!root_cid_identity_matches(
            &root.to_string(),
            &hashtree_core::to_hex(&[0x43; 32]),
        ));
    }
}
