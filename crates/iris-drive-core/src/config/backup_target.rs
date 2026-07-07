use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupTargetKind {
    Blossom,
    Fips,
    Filesystem,
    Lmdb,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupTarget {
    pub id: String,
    pub kind: BackupTargetKind,
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync: Option<BackupTargetSync>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_check: Option<BackupTargetCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupTargetSync {
    pub state: String,
    pub root_cid: String,
    pub synced_at: i64,
    pub total_hashes: usize,
    pub uploaded: usize,
    pub already_present: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupTargetCheck {
    pub state: String,
    pub root_cid: String,
    pub checked_at: i64,
    pub total_hashes: usize,
    pub sample_size: usize,
    pub sampled_hashes: usize,
    pub present: usize,
    pub missing: usize,
    pub unknown: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download_bytes_per_second: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn default_true() -> bool {
    true
}
