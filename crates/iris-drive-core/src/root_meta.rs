//! Root-level causal metadata for snapshot-first sync.
//!
//! The signed drive root remains the canonical truth. This metadata
//! travels inside `.hashtree/root.json` to explain the snapshot's
//! ancestry and per-device observations without making the drive
//! depend on operation-log replay.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootParent {
    pub device_id: String,
    pub device_seq: u64,
    pub root_cid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RootObservation {
    pub device_seq: u64,
    pub root_cid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriveRootMeta {
    pub schema: u16,
    pub drive_id: String,
    pub device_id: String,
    pub device_seq: u64,
    pub dck_generation: u64,
    /// Local bookkeeping root that should not be announced as this
    /// device's own edit.
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
