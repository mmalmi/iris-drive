//! Rebuildable local sync cache.
//!
//! Signed drive roots remain the authority. This cache only memoizes the
//! current roots, per-path file identity, and locally accepted base state so
//! sync can resume without replaying an operation log.

use std::path::Path;

use hashtree_core::{Cid, CidParseError, HashTree, HashTreeError, Store, to_hex};
use serde::{Deserialize, Serialize};
use serde_json::json;
use thiserror::Error;

use crate::config::{AppConfig, DeviceRootRef};
use crate::conflict::FileSnapshot;
use crate::merge::walk_device_tree;

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
        self.sort_rows();
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
pub struct ContentNeed {
    pub hash_or_cid: String,
    pub source_hint: Option<String>,
    pub priority: i32,
    pub first_seen_at: i64,
    pub last_attempt_at: Option<i64>,
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
}
