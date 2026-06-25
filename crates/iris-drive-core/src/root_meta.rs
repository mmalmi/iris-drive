//! Root-level causal metadata for snapshot-first sync.
//!
//! The signed drive root remains the canonical truth. This metadata
//! travels inside `.hashtree/root.json` to explain the snapshot's
//! ancestry and per-AppKey observations without making the drive
//! depend on operation-log replay.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

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
