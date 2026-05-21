//! Rebuildable local sync cache.
//!
//! Signed drive roots remain the authority. This cache only memoizes the
//! current roots, per-path file identity, and locally accepted base state so
//! sync can resume without replaying an operation log.

use std::path::Path;

use hashtree_core::{Cid, CidParseError, HashTree, HashTreeError, Store, from_hex, sha256, to_hex};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::config::{AppConfig, DeviceRootRef};
use crate::conflict::FileSnapshot;
use crate::merge::walk_device_tree;

pub const SOURCE_STATE_AVAILABLE: &str = "available";
pub const SOURCE_STATE_UNKNOWN: &str = "unknown";
pub const SOURCE_STATE_MISSING: &str = "missing";
pub const SOURCE_STATE_POISONED: &str = "poisoned";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetrievalOutcome {
    /// Bytes were received and verified against the requested hash.
    Verified,
    /// The source did not answer, timed out, or relay state was
    /// inconclusive. This is not evidence that content does not exist.
    Unknown,
    /// The source explicitly reported that it does not have this item.
    SourceMiss,
    /// Bytes were received but failed content-address verification.
    InvalidContent,
}

impl RetrievalOutcome {
    const fn source_state(self) -> &'static str {
        match self {
            Self::Verified => SOURCE_STATE_AVAILABLE,
            Self::Unknown => SOURCE_STATE_UNKNOWN,
            Self::SourceMiss => SOURCE_STATE_MISSING,
            Self::InvalidContent => SOURCE_STATE_POISONED,
        }
    }

    const fn clears_need(self) -> bool {
        matches!(self, Self::Verified)
    }
}

#[derive(Debug, Error)]
pub enum SyncCacheError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("schema version {found} not supported (expected {expected})")]
    SchemaMismatch { found: u32, expected: u32 },
    #[error("invalid root cid for drive {drive_id} device {device_id}: {root_cid}: {source}")]
    RootCid {
        drive_id: String,
        device_id: String,
        root_cid: String,
        source: CidParseError,
    },
    #[error("tree: {0}")]
    Tree(#[from] HashTreeError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCache {
    pub schema: u32,
    pub roots: Vec<CachedRoot>,
    pub path_state: Vec<CachedPathState>,
    pub base_state: Vec<CachedBaseState>,
    #[serde(default)]
    pub base_anchors: Vec<CachedBaseAnchor>,
    pub needs: Vec<ContentNeed>,
    pub source_availability: Vec<SourceAvailability>,
}

impl Default for SyncCache {
    fn default() -> Self {
        Self::empty()
    }
}

impl SyncCache {
    pub const SCHEMA: u32 = 1;

    #[must_use]
    pub fn empty() -> Self {
        Self {
            schema: Self::SCHEMA,
            roots: Vec::new(),
            path_state: Vec::new(),
            base_state: Vec::new(),
            base_anchors: Vec::new(),
            needs: Vec::new(),
            source_availability: Vec::new(),
        }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, SyncCacheError> {
        let raw = std::fs::read_to_string(path)?;
        let cache: Self = serde_json::from_str(&raw)?;
        if cache.schema != Self::SCHEMA {
            return Err(SyncCacheError::SchemaMismatch {
                found: cache.schema,
                expected: Self::SCHEMA,
            });
        }
        Ok(cache)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), SyncCacheError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut raw = serde_json::to_vec_pretty(self)?;
        raw.push(b'\n');
        std::fs::write(path, raw)?;
        Ok(())
    }

    pub async fn rebuild_from_config<S: Store>(
        tree: &HashTree<S>,
        config: &AppConfig,
        seen_at: i64,
    ) -> Result<Self, SyncCacheError> {
        let mut cache = Self::empty();
        for drive in &config.drives {
            for (device_id, root) in roots_for_drive(config, drive) {
                let root_cid =
                    Cid::parse(&root.root_cid).map_err(|source| SyncCacheError::RootCid {
                        drive_id: drive.drive_id.clone(),
                        device_id: device_id.clone(),
                        root_cid: root.root_cid.clone(),
                        source,
                    })?;
                cache.roots.push(CachedRoot {
                    drive_id: drive.drive_id.clone(),
                    device_id: device_id.clone(),
                    device_seq: root.device_seq,
                    root_cid: root.root_cid.clone(),
                    dck_generation: root.dck_generation,
                    seen_at,
                    observed_json: serde_json::to_value(&root.observed)?,
                });

                let (files, _tombstones) = walk_device_tree(tree, &root_cid).await?;
                for file in files {
                    cache.path_state.push(CachedPathState {
                        drive_id: drive.drive_id.clone(),
                        device_id: device_id.clone(),
                        path: file.path,
                        root_cid: root.root_cid.clone(),
                        whole_file_hash: file.whole_file_hash.map(|hash| to_hex(&hash)),
                        content_cid_hash: to_hex(&file.hash),
                        size: file.size,
                        metadata_json: json!({}),
                    });
                }
            }
        }
        cache.sort_rows();
        Ok(cache)
    }

    pub fn set_current_device_base(&mut self, drive_id: &str, device_id: &str) {
        let root_cid = self
            .roots
            .iter()
            .find(|row| row.drive_id == drive_id && row.device_id == device_id)
            .map(|row| row.root_cid.clone());
        self.base_state.retain(|row| row.drive_id != drive_id);
        self.base_state.extend(
            self.path_state
                .iter()
                .filter(|row| row.drive_id == drive_id && row.device_id == device_id)
                .map(|row| CachedBaseState {
                    drive_id: row.drive_id.clone(),
                    path: row.path.clone(),
                    base_root_cid: row.root_cid.clone(),
                    whole_file_hash: row.whole_file_hash.clone(),
                    content_cid_hash: row.content_cid_hash.clone(),
                    size: row.size,
                }),
        );
        self.base_anchors.retain(|row| row.drive_id != drive_id);
        if let Some(root_cid) = root_cid {
            self.base_anchors.push(CachedBaseAnchor {
                drive_id: drive_id.to_string(),
                base_root_cid: root_cid,
            });
        }
        self.sort_rows();
    }

    pub fn replace_base_state_for_drive(
        &mut self,
        drive_id: &str,
        rows: impl IntoIterator<Item = CachedBaseState>,
    ) {
        let rows: Vec<_> = rows
            .into_iter()
            .map(|mut row| {
                row.drive_id = drive_id.to_string();
                row
            })
            .collect();
        let anchor = uniform_base_root_cid(&rows).map(str::to_string);
        self.replace_base_state_for_drive_inner(drive_id, rows, anchor.as_deref());
    }

    pub fn replace_base_state_for_drive_at_anchor(
        &mut self,
        drive_id: &str,
        base_root_cid: &str,
        rows: impl IntoIterator<Item = CachedBaseState>,
    ) {
        let rows = rows.into_iter().map(|mut row| {
            row.drive_id = drive_id.to_string();
            row.base_root_cid = base_root_cid.to_string();
            row
        });
        self.replace_base_state_for_drive_inner(drive_id, rows, Some(base_root_cid));
    }

    fn replace_base_state_for_drive_inner(
        &mut self,
        drive_id: &str,
        rows: impl IntoIterator<Item = CachedBaseState>,
        base_root_cid: Option<&str>,
    ) {
        self.base_state.retain(|row| row.drive_id != drive_id);
        self.base_state.extend(rows);
        self.base_anchors.retain(|row| row.drive_id != drive_id);
        if let Some(base_root_cid) = base_root_cid {
            self.base_anchors.push(CachedBaseAnchor {
                drive_id: drive_id.to_string(),
                base_root_cid: base_root_cid.to_string(),
            });
        }
        self.sort_rows();
    }

    #[must_use]
    pub fn base_anchor_for_drive(&self, drive_id: &str) -> Option<&str> {
        self.base_anchors
            .iter()
            .find(|row| row.drive_id == drive_id)
            .map(|row| row.base_root_cid.as_str())
            .or_else(|| uniform_base_root_cid_for_drive(&self.base_state, drive_id))
    }

    pub fn record_content_need(
        &mut self,
        hash_or_cid: &str,
        source_hint: Option<&str>,
        priority: i32,
        now: i64,
    ) {
        match self
            .needs
            .iter_mut()
            .find(|row| row.hash_or_cid == hash_or_cid)
        {
            Some(row) => {
                row.priority = row.priority.min(priority);
                if row.source_hint.is_none() {
                    row.source_hint = source_hint.map(str::to_string);
                }
            }
            None => self.needs.push(ContentNeed {
                hash_or_cid: hash_or_cid.to_string(),
                source_hint: source_hint.map(str::to_string),
                priority,
                first_seen_at: now,
                last_attempt_at: None,
            }),
        }
        self.sort_rows();
    }

    pub fn record_retrieval_outcome(
        &mut self,
        hash_or_cid: &str,
        source_id: &str,
        outcome: RetrievalOutcome,
        now: i64,
    ) {
        self.record_source_availability(hash_or_cid, source_id, outcome.source_state(), now);
        if outcome.clears_need() {
            self.needs.retain(|row| row.hash_or_cid != hash_or_cid);
        } else if let Some(row) = self
            .needs
            .iter_mut()
            .find(|row| row.hash_or_cid == hash_or_cid)
        {
            if row.source_hint.is_none() {
                row.source_hint = Some(source_id.to_string());
            }
            row.last_attempt_at = Some(now);
        } else {
            self.needs.push(ContentNeed {
                hash_or_cid: hash_or_cid.to_string(),
                source_hint: Some(source_id.to_string()),
                priority: 0,
                first_seen_at: now,
                last_attempt_at: Some(now),
            });
        }
        self.sort_rows();
    }

    #[must_use]
    pub fn record_hash_response(
        &mut self,
        expected_hash_hex: &str,
        source_id: &str,
        bytes: &[u8],
        now: i64,
    ) -> bool {
        let verified = from_hex(expected_hash_hex).is_ok_and(|expected| sha256(bytes) == expected);
        let outcome = if verified {
            RetrievalOutcome::Verified
        } else {
            RetrievalOutcome::InvalidContent
        };
        self.record_retrieval_outcome(expected_hash_hex, source_id, outcome, now);
        verified
    }

    fn record_source_availability(
        &mut self,
        hash_or_cid: &str,
        source_id: &str,
        state: &str,
        now: i64,
    ) {
        match self
            .source_availability
            .iter_mut()
            .find(|row| row.hash_or_cid == hash_or_cid && row.source_id == source_id)
        {
            Some(row) => {
                row.state = state.to_string();
                row.updated_at = now;
            }
            None => self.source_availability.push(SourceAvailability {
                hash_or_cid: hash_or_cid.to_string(),
                source_id: source_id.to_string(),
                state: state.to_string(),
                updated_at: now,
            }),
        }
    }

    #[must_use]
    pub fn base_snapshots_for_drive(
        &self,
        drive_id: &str,
    ) -> std::collections::BTreeMap<String, FileSnapshot> {
        self.base_state
            .iter()
            .filter(|row| row.drive_id == drive_id)
            .map(|row| {
                (
                    row.path.clone(),
                    FileSnapshot {
                        content_hash: row
                            .whole_file_hash
                            .clone()
                            .unwrap_or_else(|| row.content_cid_hash.clone()),
                        mtime: 0,
                    },
                )
            })
            .collect()
    }

    fn sort_rows(&mut self) {
        self.roots.sort_by(|left, right| {
            (
                &left.drive_id,
                &left.device_id,
                left.device_seq,
                &left.root_cid,
            )
                .cmp(&(
                    &right.drive_id,
                    &right.device_id,
                    right.device_seq,
                    &right.root_cid,
                ))
        });
        self.path_state.sort_by(|left, right| {
            (&left.drive_id, &left.path, &left.device_id, &left.root_cid).cmp(&(
                &right.drive_id,
                &right.path,
                &right.device_id,
                &right.root_cid,
            ))
        });
        self.base_state.sort_by(|left, right| {
            (&left.drive_id, &left.path, &left.base_root_cid).cmp(&(
                &right.drive_id,
                &right.path,
                &right.base_root_cid,
            ))
        });
        self.base_anchors
            .sort_by(|left, right| left.drive_id.cmp(&right.drive_id));
        self.needs.sort_by(|left, right| {
            (&left.hash_or_cid, left.priority).cmp(&(&right.hash_or_cid, right.priority))
        });
        self.source_availability.sort_by(|left, right| {
            (&left.hash_or_cid, &left.source_id).cmp(&(&right.hash_or_cid, &right.source_id))
        });
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedRoot {
    pub drive_id: String,
    pub device_id: String,
    pub device_seq: u64,
    pub root_cid: String,
    pub dck_generation: u64,
    pub seen_at: i64,
    pub observed_json: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedPathState {
    pub drive_id: String,
    pub device_id: String,
    pub path: String,
    pub root_cid: String,
    pub whole_file_hash: Option<String>,
    pub content_cid_hash: String,
    pub size: u64,
    pub metadata_json: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedBaseState {
    pub drive_id: String,
    pub path: String,
    pub base_root_cid: String,
    pub whole_file_hash: Option<String>,
    pub content_cid_hash: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedBaseAnchor {
    pub drive_id: String,
    pub base_root_cid: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentNeed {
    pub hash_or_cid: String,
    pub source_hint: Option<String>,
    pub priority: i32,
    pub first_seen_at: i64,
    pub last_attempt_at: Option<i64>,
}

fn uniform_base_root_cid(rows: &[CachedBaseState]) -> Option<&str> {
    let mut roots = rows.iter().map(|row| row.base_root_cid.as_str());
    let first = roots.next()?;
    if roots.all(|root| root == first) {
        Some(first)
    } else {
        None
    }
}

fn uniform_base_root_cid_for_drive<'a>(
    rows: &'a [CachedBaseState],
    drive_id: &str,
) -> Option<&'a str> {
    let mut roots = rows
        .iter()
        .filter(|row| row.drive_id == drive_id)
        .map(|row| row.base_root_cid.as_str());
    let first = roots.next()?;
    if roots.all(|root| root == first) {
        Some(first)
    } else {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceAvailability {
    pub hash_or_cid: String,
    pub source_id: String,
    pub state: String,
    pub updated_at: i64,
}

fn roots_for_drive(
    config: &AppConfig,
    drive: &crate::config::Drive,
) -> Vec<(String, DeviceRootRef)> {
    if !drive.device_roots.is_empty() {
        return drive
            .device_roots
            .iter()
            .map(|(device_id, root)| (device_id.clone(), root.clone()))
            .collect();
    }

    let Some(root_cid) = drive.last_root_cid.clone() else {
        return Vec::new();
    };
    let device_id = config.account.as_ref().map_or_else(
        || "legacy".to_string(),
        |account| account.device_pubkey.clone(),
    );
    vec![(device_id, DeviceRootRef::legacy(root_cid, 0, 0))]
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashtree_core::{sha256, to_hex};

    #[test]
    fn base_snapshots_for_drive_prefer_whole_file_hash() {
        let mut cache = SyncCache::empty();
        cache.base_state = vec![
            CachedBaseState {
                drive_id: "main".into(),
                path: "a.txt".into(),
                base_root_cid: "root-a".into(),
                whole_file_hash: Some("whole-a".into()),
                content_cid_hash: "cid-a".into(),
                size: 1,
            },
            CachedBaseState {
                drive_id: "main".into(),
                path: "b.txt".into(),
                base_root_cid: "root-b".into(),
                whole_file_hash: None,
                content_cid_hash: "cid-b".into(),
                size: 2,
            },
            CachedBaseState {
                drive_id: "other".into(),
                path: "ignored.txt".into(),
                base_root_cid: "root-c".into(),
                whole_file_hash: Some("whole-c".into()),
                content_cid_hash: "cid-c".into(),
                size: 3,
            },
        ];

        let snapshots = cache.base_snapshots_for_drive("main");
        assert_eq!(snapshots.len(), 2);
        assert_eq!(snapshots["a.txt"].content_hash, "whole-a");
        assert_eq!(snapshots["b.txt"].content_hash, "cid-b");
    }

    #[test]
    fn replace_base_state_for_drive_keeps_other_drives_and_sorts() {
        let mut cache = SyncCache::empty();
        cache.base_state = vec![CachedBaseState {
            drive_id: "other".into(),
            path: "other.txt".into(),
            base_root_cid: "root-other".into(),
            whole_file_hash: Some("whole-other".into()),
            content_cid_hash: "cid-other".into(),
            size: 1,
        }];

        cache.replace_base_state_for_drive(
            "main",
            vec![
                CachedBaseState {
                    drive_id: "ignored".into(),
                    path: "z.txt".into(),
                    base_root_cid: "root-z".into(),
                    whole_file_hash: None,
                    content_cid_hash: "cid-z".into(),
                    size: 1,
                },
                CachedBaseState {
                    drive_id: "ignored".into(),
                    path: "a.txt".into(),
                    base_root_cid: "root-a".into(),
                    whole_file_hash: None,
                    content_cid_hash: "cid-a".into(),
                    size: 1,
                },
            ],
        );

        let paths: Vec<_> = cache
            .base_state
            .iter()
            .map(|row| (row.drive_id.as_str(), row.path.as_str()))
            .collect();
        assert_eq!(
            paths,
            vec![("main", "a.txt"), ("main", "z.txt"), ("other", "other.txt")]
        );
    }

    #[test]
    fn replace_base_state_for_drive_at_anchor_keeps_empty_anchor() {
        let mut cache = SyncCache::empty();
        cache.base_anchors = vec![CachedBaseAnchor {
            drive_id: "other".into(),
            base_root_cid: "root-other".into(),
        }];

        cache.replace_base_state_for_drive_at_anchor(
            "main",
            "root-empty",
            Vec::<CachedBaseState>::new(),
        );

        assert!(cache.base_snapshots_for_drive("main").is_empty());
        assert_eq!(cache.base_anchor_for_drive("main"), Some("root-empty"));
        assert_eq!(cache.base_anchor_for_drive("other"), Some("root-other"));
    }

    #[test]
    fn legacy_cache_without_base_anchors_deserializes() {
        let raw = r#"{
            "schema": 1,
            "roots": [],
            "path_state": [],
            "base_state": [],
            "needs": [],
            "source_availability": []
        }"#;

        let cache: SyncCache = serde_json::from_str(raw).unwrap();

        assert!(cache.base_anchors.is_empty());
        assert!(cache.base_anchor_for_drive("main").is_none());
    }

    #[test]
    fn unknown_retrieval_keeps_need_retryable() {
        let mut cache = SyncCache::empty();
        cache.record_content_need("hash-a", Some("peer-a"), 10, 100);

        cache.record_retrieval_outcome("hash-a", "peer-a", RetrievalOutcome::Unknown, 110);

        assert_eq!(cache.needs.len(), 1);
        assert_eq!(cache.needs[0].hash_or_cid, "hash-a");
        assert_eq!(cache.needs[0].source_hint.as_deref(), Some("peer-a"));
        assert_eq!(cache.needs[0].priority, 10);
        assert_eq!(cache.needs[0].first_seen_at, 100);
        assert_eq!(cache.needs[0].last_attempt_at, Some(110));
        assert_eq!(cache.source_availability.len(), 1);
        assert_eq!(cache.source_availability[0].state, SOURCE_STATE_UNKNOWN);
    }

    #[test]
    fn verified_retrieval_clears_need_and_marks_source_available() {
        let mut cache = SyncCache::empty();
        cache.record_content_need("hash-a", Some("peer-a"), 10, 100);

        cache.record_retrieval_outcome("hash-a", "peer-a", RetrievalOutcome::Verified, 110);

        assert!(cache.needs.is_empty());
        assert_eq!(cache.source_availability.len(), 1);
        assert_eq!(cache.source_availability[0].state, SOURCE_STATE_AVAILABLE);
        assert_eq!(cache.source_availability[0].updated_at, 110);
    }

    #[test]
    fn source_miss_keeps_need_and_marks_source_missing() {
        let mut cache = SyncCache::empty();
        cache.record_content_need("hash-a", Some("peer-a"), 10, 100);

        cache.record_retrieval_outcome("hash-a", "peer-a", RetrievalOutcome::SourceMiss, 110);

        assert_eq!(cache.needs.len(), 1);
        assert_eq!(cache.needs[0].last_attempt_at, Some(110));
        assert_eq!(cache.source_availability[0].state, SOURCE_STATE_MISSING);
    }

    #[test]
    fn poisoned_hash_response_keeps_need_and_marks_source_poisoned() {
        let mut cache = SyncCache::empty();
        let expected = to_hex(&sha256(b"good bytes"));
        cache.record_content_need(&expected, Some("peer-a"), 10, 100);

        assert!(!cache.record_hash_response(&expected, "peer-a", b"bad bytes", 110));

        assert_eq!(cache.needs.len(), 1);
        assert_eq!(cache.needs[0].last_attempt_at, Some(110));
        assert_eq!(cache.source_availability[0].state, SOURCE_STATE_POISONED);

        assert!(cache.record_hash_response(&expected, "peer-b", b"good bytes", 120));

        assert!(cache.needs.is_empty());
        assert_eq!(
            cache
                .source_availability
                .iter()
                .find(|source| source.source_id == "peer-b")
                .unwrap()
                .state,
            SOURCE_STATE_AVAILABLE
        );
    }
}
