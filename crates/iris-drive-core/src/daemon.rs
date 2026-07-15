//! Persistent daemon supervisor.
//!
//! Owns:
//! - A filesystem-backed hashtree store at `<config_dir>/blocks/`.
//! - The user's `AppConfig` (drives, schema, identity reference).
//! - Virtual mount/provider roots for the primary drive.
//!
//! Stays minimal for v1: one-shot import + status. Long-running sync is
//! handled by the CLI daemon over virtual roots/providers.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hashtree_core::{Cid, HashTree, HashTreeConfig};
use hashtree_fs::FsBlobStore;
use thiserror::Error;

use crate::calendar::CALENDAR_TREE_NAME;
use crate::config::{AppConfig, AppKeyRootRef, ConfigError, Drive, DriveRole};
use crate::conflict::ConflictState;
use crate::indexer::{
    IndexError, index_dir_with_history_and_meta, layer_conflict_records,
    layer_history_and_meta_on_root, layer_history_and_meta_on_root_with_tombstone_base_and_paths,
    layer_prev_link, layer_root_meta, local_visible_root_for_mount_import, read_conflict_records,
    read_root_meta,
};
use crate::paths::{config_path_in, key_path_in, sync_cache_path_in};
use crate::root_meta::{DriveRootMeta, RootObservation, RootParent};
use crate::sync_cache::{SyncCache, SyncCacheError};

pub const PRIMARY_DRIVE_ID: &str = "main";
const LOCAL_ONLY_PARENT_WALK_LIMIT: usize = 1024;

mod embedded;
mod local_only;

pub use embedded::{EmbeddedHashtreeHost, embedded_hashtree_state_root_in};
#[cfg(test)]
pub(crate) use embedded::{embedded_browser_nostr_relays, embedded_browser_settings, same_relay};

#[derive(Debug, Error)]
pub enum DaemonError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("config: {0}")]
    Config(#[from] ConfigError),
    #[error("identity not initialized; run `idrive init` first")]
    Uninitialized,
    #[error("index: {0}")]
    Index(#[from] IndexError),
    #[error("projection: {0}")]
    Projection(#[from] crate::ProjectionError),
    #[error("sync cache: {0}")]
    SyncCache(#[from] SyncCacheError),
    #[error("store: {0}")]
    Store(String),
    #[error("primary drive missing from config (expected drive_id={PRIMARY_DRIVE_ID})")]
    PrimaryDriveMissing,
    #[error("drive missing from config: {0}")]
    DriveMissing(String),
    #[error("primary drive has no recorded root")]
    PrimaryRootMissing,
    #[error("conflict record not found: {0}")]
    ConflictRecordNotFound(String),
}

/// Snapshot of an import operation, suitable for serializing to JSON.
#[derive(Debug, Clone)]
pub struct ImportReport {
    pub root_cid: String,
    pub source_dir: Option<PathBuf>,
    pub file_count: usize,
    pub top_level_entries: usize,
}

/// Snapshot of a conflict-resolution marker update.
#[derive(Debug, Clone)]
pub struct ConflictResolveReport {
    pub conflict_id: String,
    pub previous_root_cid: String,
    pub root_cid: String,
    pub changed: bool,
}

pub struct Daemon {
    config_dir: PathBuf,
    blocks_dir: PathBuf,
    tree: Arc<HashTree<FsBlobStore>>,
    config: AppConfig,
}

impl Daemon {
    /// Open a daemon rooted at `config_dir`. The block store lives at
    /// `<config_dir>/blocks/`; the config file lives at
    /// `<config_dir>/config.toml`. Returns `Uninitialized` if no
    /// identity has been generated yet — callers should run
    /// `Identity::load_or_generate(key_path_in(config_dir))` first
    /// (i.e. `idrive init`).
    pub fn open(config_dir: impl Into<PathBuf>) -> Result<Self, DaemonError> {
        let config_dir = config_dir.into();
        if !key_path_in(&config_dir).exists() {
            return Err(DaemonError::Uninitialized);
        }
        let config = AppConfig::load_or_default_cached_profile(config_path_in(&config_dir))?;
        Self::open_with_config(config_dir, config)
    }

    pub fn open_with_config(
        config_dir: impl Into<PathBuf>,
        config: AppConfig,
    ) -> Result<Self, DaemonError> {
        let config_dir = config_dir.into();
        if !key_path_in(&config_dir).exists() {
            return Err(DaemonError::Uninitialized);
        }
        std::fs::create_dir_all(&config_dir)?;
        let blocks_dir = config_dir.join("blocks");
        std::fs::create_dir_all(&blocks_dir)?;
        let store = FsBlobStore::new(&blocks_dir).map_err(|e| DaemonError::Store(e.to_string()))?;
        let tree = Arc::new(HashTree::new(HashTreeConfig::new(Arc::new(store))));
        Ok(Self {
            config_dir,
            blocks_dir,
            tree,
            config,
        })
    }

    #[must_use]
    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    #[must_use]
    pub fn blocks_dir(&self) -> &Path {
        &self.blocks_dir
    }

    #[must_use]
    pub fn tree(&self) -> &HashTree<FsBlobStore> {
        &self.tree
    }

    #[must_use]
    pub fn tree_handle(&self) -> Arc<HashTree<FsBlobStore>> {
        self.tree.clone()
    }

    /// CID currently recorded for the primary drive, if any.
    #[must_use]
    pub fn primary_root(&self) -> Option<&str> {
        self.config
            .drive(PRIMARY_DRIVE_ID)
            .and_then(|d| d.last_root_cid.as_deref())
    }

    /// Bulk-index `source_dir` into the daemon's persistent store and
    /// stamp the resulting root CID onto the primary drive. The previous
    /// root remains addressable in the store; nothing is GC'd.
    pub async fn import_source_dir(
        &mut self,
        source_dir: impl AsRef<Path>,
    ) -> Result<ImportReport, DaemonError> {
        self.load_full_profile_for_mutation()?;
        let source_dir = source_dir.as_ref();
        // Look up this device's previous root, if any, so the indexer
        // can diff against it and emit tombstones for removed files.
        let previous_root_cid = self
            .config
            .profile
            .as_ref()
            .and_then(|account| {
                self.config
                    .drive(PRIMARY_DRIVE_ID)
                    .and_then(|d| d.app_key_roots.get(&account.app_key_pubkey))
            })
            .map(|entry| entry.root_cid.clone());
        let previous_root = match previous_root_cid.as_ref() {
            Some(s) => Some(Cid::parse(s).map_err(|e| DaemonError::Store(e.to_string()))?),
            None => None,
        };

        let now = self.next_import_timestamp();
        let root_meta = self.root_meta_for_import(now);
        let root_cid = index_dir_with_history_and_meta(
            &self.tree,
            source_dir,
            previous_root.as_ref(),
            now,
            root_meta.as_ref(),
        )
        .await?;

        self.report_and_record_root(
            root_cid,
            Some(source_dir.to_path_buf()),
            root_meta.as_ref(),
            now,
        )
        .await
    }

    /// Persist a user-visible hashtree root produced by a virtual mount or
    /// platform file-provider adapter as this device's latest contribution.
    ///
    /// The input root should contain only visible user files/directories. This
    /// method layers Iris Drive history, tombstones, and causal metadata before
    /// recording the root in config, so mount-backed edits sync with the same
    /// merge semantics as folder imports.
    pub async fn import_visible_root(&mut self, root: Cid) -> Result<ImportReport, DaemonError> {
        self.import_visible_root_for_drive(PRIMARY_DRIVE_ID, root)
            .await
    }

    /// Persist a visible root for a specific configured tree/drive.
    ///
    /// This is used by app-specific web-compatible roots such as the Iris
    /// Calendar `calendar` tree. The primary Drive import helpers above remain
    /// the usual path for provider/mount edits.
    pub async fn import_visible_root_for_drive(
        &mut self,
        drive_id: &str,
        root: Cid,
    ) -> Result<ImportReport, DaemonError> {
        self.import_visible_root_with_tombstone_base_for_drive(drive_id, root, None)
            .await
    }

    /// Publish this `AppKey`'s causal root for the currently merged drive view.
    ///
    /// Remote `AppKey` roots are inputs. Once their blocks are local, this folds
    /// the merged visible view into this `AppKey`'s own root and records the
    /// observed source roots in `.hashtree/root.json`, so later roster changes
    /// do not make already-accepted files depend on a removed `AppKey` root.
    pub async fn materialize_primary_merged_root(
        &mut self,
    ) -> Result<Option<ImportReport>, DaemonError> {
        self.load_full_profile_for_mutation()?;
        let Some(account) = self.config.profile.clone() else {
            return Ok(None);
        };
        let drive = self
            .config
            .drive(PRIMARY_DRIVE_ID)
            .ok_or(DaemonError::PrimaryDriveMissing)?
            .clone();
        if drive.app_key_roots.is_empty() {
            return Ok(None);
        }

        let merged = crate::primary_merged_root(&self.tree, &self.config).await?;
        if let Some(current) = drive.app_key_roots.get(&account.app_key_pubkey) {
            let current_cid =
                Cid::parse(&current.root_cid).map_err(|e| DaemonError::Store(e.to_string()))?;
            let current_visible =
                crate::indexer::filter_ignored_entries_from_root(&self.tree, &current_cid).await?;
            if current_visible == merged.root_cid
                && current_root_observes_drive_roots(&self.tree, &current_cid, &account, &drive)
                    .await?
            {
                return Ok(None);
            }
        }

        self.import_visible_root_local_only(merged.root_cid)
            .await
            .map(Some)
    }

    pub async fn import_visible_root_with_tombstone_base(
        &mut self,
        root: Cid,
        tombstone_base_root: Option<Cid>,
    ) -> Result<ImportReport, DaemonError> {
        self.import_visible_root_with_tombstone_base_for_drive(
            PRIMARY_DRIVE_ID,
            root,
            tombstone_base_root,
        )
        .await
    }

    pub async fn import_visible_root_with_tombstone_base_for_drive(
        &mut self,
        drive_id: &str,
        root: Cid,
        tombstone_base_root: Option<Cid>,
    ) -> Result<ImportReport, DaemonError> {
        self.import_visible_root_with_tombstone_base_paths_and_local_only_for_drive(
            drive_id,
            root,
            tombstone_base_root,
            None,
            false,
        )
        .await
    }

    pub async fn import_visible_root_with_tombstone_base_and_paths(
        &mut self,
        root: Cid,
        tombstone_base_root: Option<Cid>,
        tombstone_paths: Option<&std::collections::BTreeSet<String>>,
    ) -> Result<ImportReport, DaemonError> {
        self.import_visible_root_with_tombstone_base_paths_and_local_only_for_drive(
            PRIMARY_DRIVE_ID,
            root,
            tombstone_base_root,
            tombstone_paths,
            false,
        )
        .await
    }

    async fn import_visible_root_local_only(
        &mut self,
        root: Cid,
    ) -> Result<ImportReport, DaemonError> {
        self.import_visible_root_with_tombstone_base_paths_and_local_only_for_drive(
            PRIMARY_DRIVE_ID,
            root,
            None,
            None,
            true,
        )
        .await
    }

    async fn import_visible_root_with_tombstone_base_paths_and_local_only_for_drive(
        &mut self,
        drive_id: &str,
        root: Cid,
        tombstone_base_root: Option<Cid>,
        tombstone_paths: Option<&std::collections::BTreeSet<String>>,
        local_only: bool,
    ) -> Result<ImportReport, DaemonError> {
        self.load_full_profile_for_mutation()?;
        self.ensure_drive_for_import(drive_id)?;
        let previous_root_ref = self.config.profile.as_ref().and_then(|account| {
            self.config
                .drive(drive_id)
                .and_then(|d| d.app_key_roots.get(&account.app_key_pubkey))
        });
        let previous_root_cid = previous_root_ref.map(|entry| entry.root_cid.clone());
        let previous_root = match previous_root_cid.as_ref() {
            Some(s) => Some(Cid::parse(s).map_err(|e| DaemonError::Store(e.to_string()))?),
            None => None,
        };
        let local_only_tombstone_mask = if !local_only
            && drive_id == PRIMARY_DRIVE_ID
            && tombstone_base_root.is_some()
            && previous_root_ref.is_some_and(|root| root.local_only)
        {
            self.local_only_tombstone_mask(previous_root.as_ref())
                .await?
        } else {
            None
        };
        let previous_publishable_root_ref = if drive_id == PRIMARY_DRIVE_ID {
            match (self.config.profile.as_ref(), previous_root_ref) {
                (Some(account), Some(previous_root_ref)) => {
                    self.previous_publishable_root_ref_for_import(
                        &account.app_key_pubkey,
                        previous_root_ref,
                    )
                    .await?
                }
                _ => None,
            }
        } else {
            previous_root_ref.cloned()
        };
        let previous_publishable_root = previous_publishable_root_ref
            .as_ref()
            .map(|root| Cid::parse(&root.root_cid))
            .transpose()
            .map_err(|e| DaemonError::Store(e.to_string()))?;
        let previous_delta_root = if local_only {
            None
        } else if tombstone_base_root.is_some() && drive_id == PRIMARY_DRIVE_ID {
            previous_publishable_root.clone()
        } else {
            previous_root.clone()
        };
        let previous_history_root = if local_only {
            None
        } else if drive_id == PRIMARY_DRIVE_ID {
            previous_publishable_root
        } else {
            previous_root.clone()
        };

        let now = self.next_import_timestamp_for_drive(drive_id);
        let mut root_meta = self.root_meta_for_import_for_drive(drive_id, now);
        if let Some(meta) = root_meta.as_mut() {
            meta.local_only = local_only;
            if local_only
                && drive_id == PRIMARY_DRIVE_ID
                && let (Some(account), Some(previous_publishable_root_ref)) = (
                    self.config.profile.as_ref(),
                    previous_publishable_root_ref.as_ref(),
                )
            {
                meta.parents = vec![RootParent {
                    app_key_pubkey: account.app_key_pubkey.clone(),
                    app_key_seq: previous_publishable_root_ref.app_key_seq,
                    root_cid: previous_publishable_root_ref.root_cid.clone(),
                }];
            }
        }
        let mut scoped_tombstone_paths = None;
        let projection_root = if tombstone_base_root.is_some() && drive_id == PRIMARY_DRIVE_ID {
            Some(
                crate::primary_merged_root(&self.tree, &self.config)
                    .await?
                    .root_cid,
            )
        } else {
            tombstone_base_root.clone()
        };
        let import_root = if let Some(tombstone_base_root) = tombstone_base_root.as_ref() {
            let phase = std::time::Instant::now();
            let delta = local_visible_root_for_mount_import(
                &self.tree,
                &root,
                previous_delta_root.as_ref(),
                tombstone_base_root,
                projection_root.as_ref(),
                tombstone_paths,
            )
            .await?;
            let mut import_root = delta.root;
            if let Some(mask) = local_only_tombstone_mask.as_ref() {
                import_root = self
                    .remove_legacy_local_only_tombstoned_paths(import_root, &root, mask)
                    .await?;
            }
            tracing::debug!(
                elapsed_ms = phase.elapsed().as_millis(),
                "visible root import built local delta"
            );
            scoped_tombstone_paths = Some(delta.tombstone_paths);
            import_root
        } else {
            root
        };
        let tombstone_paths = scoped_tombstone_paths
            .as_ref()
            .map_or(tombstone_paths, Some);
        let root_cid = if let Some(tombstone_base_root) = tombstone_base_root.as_ref() {
            let phase = std::time::Instant::now();
            let root_cid = layer_history_and_meta_on_root_with_tombstone_base_and_paths(
                &self.tree,
                import_root,
                previous_history_root.as_ref(),
                Some(tombstone_base_root),
                now,
                root_meta.as_ref(),
                tombstone_paths,
            )
            .await?;
            tracing::debug!(
                elapsed_ms = phase.elapsed().as_millis(),
                "visible root import layered metadata"
            );
            root_cid
        } else {
            let phase = std::time::Instant::now();
            let root_cid = layer_history_and_meta_on_root(
                &self.tree,
                import_root,
                previous_history_root.as_ref(),
                now,
                root_meta.as_ref(),
            )
            .await?;
            tracing::debug!(
                elapsed_ms = phase.elapsed().as_millis(),
                "visible root import layered metadata"
            );
            root_cid
        };

        self.report_and_record_root_for_drive(drive_id, root_cid, None, root_meta.as_ref(), now)
            .await
    }

    async fn previous_publishable_root_ref_for_import(
        &self,
        app_key_pubkey: &str,
        previous_root: &AppKeyRootRef,
    ) -> Result<Option<AppKeyRootRef>, DaemonError> {
        let mut current = previous_root.clone();
        for _ in 0..LOCAL_ONLY_PARENT_WALK_LIMIT {
            if !current.local_only {
                return Ok(Some(current));
            }
            let Some(parent) = current
                .parents
                .iter()
                .rev()
                .find(|parent| parent.app_key_pubkey == app_key_pubkey)
            else {
                return Ok(None);
            };
            let parent_cid =
                Cid::parse(&parent.root_cid).map_err(|e| DaemonError::Store(e.to_string()))?;
            let Some(meta) = read_root_meta(&self.tree, &parent_cid).await? else {
                let mut legacy = AppKeyRootRef::legacy(
                    parent.root_cid.clone(),
                    current.published_at,
                    current.dck_generation,
                );
                legacy.app_key_seq = parent.app_key_seq;
                return Ok(Some(legacy));
            };
            current = AppKeyRootRef::from_meta(parent.root_cid.clone(), meta.created_at, &meta);
        }
        Ok(None)
    }

    async fn report_and_record_root(
        &mut self,
        root_cid: Cid,
        source_dir: Option<PathBuf>,
        root_meta: Option<&DriveRootMeta>,
        published_at: i64,
    ) -> Result<ImportReport, DaemonError> {
        self.report_and_record_root_for_drive(
            PRIMARY_DRIVE_ID,
            root_cid,
            source_dir,
            root_meta,
            published_at,
        )
        .await
    }

    async fn report_and_record_root_for_drive(
        &mut self,
        drive_id: &str,
        root_cid: Cid,
        source_dir: Option<PathBuf>,
        root_meta: Option<&DriveRootMeta>,
        published_at: i64,
    ) -> Result<ImportReport, DaemonError> {
        // Live-file count excludes the internal metadata directory from the
        // report.
        let phase = std::time::Instant::now();
        let listing = self
            .tree
            .list_directory(&root_cid)
            .await
            .map_err(|e| DaemonError::Store(e.to_string()))?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            "record root listed top-level entries"
        );
        let top_level_entries = listing
            .iter()
            .filter(|e| e.name != crate::merge::META_DIR)
            .count();
        let phase = std::time::Instant::now();
        let (files, _tombstones) = crate::merge::walk_app_key_tree(&self.tree, &root_cid)
            .await
            .map_err(|e| DaemonError::Store(e.to_string()))?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            "record root walked device tree"
        );

        let phase = std::time::Instant::now();
        self.update_drive(drive_id, &root_cid, root_meta, published_at)?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            "record root updated config"
        );
        let phase = std::time::Instant::now();
        self.persist_sync_cache_with_current_base_for_drive(drive_id)
            .await?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            "record root persisted sync cache"
        );

        Ok(ImportReport {
            root_cid: root_cid.to_string(),
            source_dir,
            file_count: files.len(),
            top_level_entries,
        })
    }

    /// Mark a durable conflict record resolved and publish a new root
    /// that preserves the rest of the snapshot unchanged.
    pub async fn resolve_conflict_record(
        &mut self,
        conflict_id: &str,
    ) -> Result<ConflictResolveReport, DaemonError> {
        self.load_full_profile_for_mutation()?;
        let previous_root_cid = self.current_app_key_root_cid()?.to_string();
        let previous_root =
            Cid::parse(&previous_root_cid).map_err(|e| DaemonError::Store(e.to_string()))?;
        let mut records = read_conflict_records(&self.tree, &previous_root).await?;
        let Some(record) = records
            .iter_mut()
            .find(|record| record.conflict_id == conflict_id)
        else {
            return Err(DaemonError::ConflictRecordNotFound(conflict_id.to_string()));
        };

        if record.state == ConflictState::Resolved {
            return Ok(ConflictResolveReport {
                conflict_id: conflict_id.to_string(),
                previous_root_cid: previous_root_cid.clone(),
                root_cid: previous_root_cid,
                changed: false,
            });
        }

        record.state = ConflictState::Resolved;
        let now = self.next_import_timestamp();
        let root_meta = self.root_meta_for_import(now);
        let mut root = layer_conflict_records(
            &self.tree,
            previous_root.clone(),
            std::slice::from_ref(record),
        )
        .await?;
        root = layer_prev_link(&self.tree, root, &previous_root).await?;
        if let Some(meta) = root_meta.as_ref() {
            root = layer_root_meta(&self.tree, root, meta).await?;
        }
        self.update_primary_drive(&root, root_meta.as_ref(), now)?;
        self.persist_sync_cache_with_current_base_for_drive(PRIMARY_DRIVE_ID)
            .await?;

        Ok(ConflictResolveReport {
            conflict_id: conflict_id.to_string(),
            previous_root_cid,
            root_cid: root.to_string(),
            changed: true,
        })
    }

    fn current_app_key_root_cid(&self) -> Result<&str, DaemonError> {
        let drive = self
            .config
            .drive(PRIMARY_DRIVE_ID)
            .ok_or(DaemonError::PrimaryDriveMissing)?;
        if let Some(account) = self.config.profile.as_ref()
            && let Some(root) = drive.app_key_roots.get(&account.app_key_pubkey)
        {
            return Ok(&root.root_cid);
        }
        drive
            .last_root_cid
            .as_deref()
            .ok_or(DaemonError::PrimaryRootMissing)
    }

    pub async fn rebuild_sync_cache(&self) -> Result<SyncCache, DaemonError> {
        let cache = SyncCache::rebuild_from_config(&self.tree, &self.config, unix_now()).await?;
        cache.save(sync_cache_path_in(&self.config_dir))?;
        Ok(cache)
    }

    pub async fn load_or_rebuild_sync_cache(&self) -> Result<SyncCache, DaemonError> {
        let path = sync_cache_path_in(&self.config_dir);
        match SyncCache::load(&path) {
            Ok(cache) => Ok(cache),
            Err(SyncCacheError::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                self.rebuild_sync_cache().await
            }
            Err(SyncCacheError::Json(_) | SyncCacheError::SchemaMismatch { .. }) => {
                self.rebuild_sync_cache().await
            }
            Err(error) => Err(error.into()),
        }
    }

    async fn persist_sync_cache_with_current_base_for_drive(
        &self,
        drive_id: &str,
    ) -> Result<(), DaemonError> {
        let Some(account) = self.config.profile.as_ref() else {
            let cache =
                SyncCache::rebuild_from_config(&self.tree, &self.config, unix_now()).await?;
            cache.save(sync_cache_path_in(&self.config_dir))?;
            return Ok(());
        };

        let path = sync_cache_path_in(&self.config_dir);
        let mut cache = SyncCache::load(&path).unwrap_or_else(|_| SyncCache::empty());
        if cache
            .replace_app_key_root_from_config(
                &self.tree,
                &self.config,
                drive_id,
                &account.app_key_pubkey,
                unix_now(),
            )
            .await
            .is_err()
        {
            cache = SyncCache::rebuild_from_config(&self.tree, &self.config, unix_now()).await?;
            cache.set_current_device_base(drive_id, &account.app_key_pubkey);
        }
        cache.save(sync_cache_path_in(&self.config_dir))?;
        Ok(())
    }

    fn update_primary_drive(
        &mut self,
        root_cid: &Cid,
        root_meta: Option<&DriveRootMeta>,
        published_at: i64,
    ) -> Result<(), DaemonError> {
        self.update_drive(PRIMARY_DRIVE_ID, root_cid, root_meta, published_at)
    }

    fn update_drive(
        &mut self,
        drive_id: &str,
        root_cid: &Cid,
        root_meta: Option<&DriveRootMeta>,
        published_at: i64,
    ) -> Result<(), DaemonError> {
        let drive = match self.config.drive(PRIMARY_DRIVE_ID) {
            Some(d) if drive_id == PRIMARY_DRIVE_ID => d.clone(),
            _ if drive_id == PRIMARY_DRIVE_ID => return Err(DaemonError::PrimaryDriveMissing),
            _ => self
                .config
                .drive(drive_id)
                .cloned()
                .ok_or_else(|| DaemonError::DriveMissing(drive_id.to_string()))?,
        };
        let mut updated = drive;
        updated.last_root_cid = Some(root_cid.to_string());

        // Per-AppKey root entry, keyed by this install's AppKey pubkey.
        // Falls back to no-op when there is no profile yet.
        if let Some(account) = self.config.profile.as_ref() {
            let dck_generation = account
                .current_app_keys_projection()
                .map_or(0, |snap| snap.dck_generation);
            let app_key_root = root_meta.map_or_else(
                || AppKeyRootRef::legacy(root_cid.to_string(), published_at, dck_generation),
                |meta| AppKeyRootRef::from_meta(root_cid.to_string(), published_at, meta),
            );
            updated
                .app_key_roots
                .insert(account.app_key_pubkey.clone(), app_key_root);
        }

        self.config.upsert_drive(updated);
        self.config.save(config_path_in(&self.config_dir))?;
        Ok(())
    }

    fn load_full_profile_for_mutation(&mut self) -> Result<(), DaemonError> {
        // Daemon::open keeps startup/status cheap with cached profile state,
        // but root writes can benefit from sidecar roster ops for AppKey
        // metadata. Linked devices may have already learned peer roots before
        // their local sidecar roster is a complete authority for every writer,
        // so only let the full profile filter root writers when it covers every
        // known root writer or explicitly tombstones it.
        let cached_profile = self.config.profile.clone();
        let config = AppConfig::load_or_default(config_path_in(&self.config_dir))?;
        self.config.profile = match config.profile {
            Some(profile) if full_profile_covers_known_app_key_roots(&profile, &self.config) => {
                Some(profile)
            }
            Some(profile) => cached_profile.or(Some(profile)),
            None => None,
        };
        Ok(())
    }

    fn next_import_timestamp(&self) -> i64 {
        self.next_import_timestamp_for_drive(PRIMARY_DRIVE_ID)
    }

    fn next_import_timestamp_for_drive(&self, drive_id: &str) -> i64 {
        let now = unix_now();
        let previous = self.config.profile.as_ref().and_then(|account| {
            self.config
                .drive(drive_id)
                .and_then(|drive| drive.app_key_roots.get(&account.app_key_pubkey))
        });
        previous.map_or(now, |root| now.max(root.published_at.saturating_add(1)))
    }

    fn root_meta_for_import(&self, created_at: i64) -> Option<DriveRootMeta> {
        self.root_meta_for_import_for_drive(PRIMARY_DRIVE_ID, created_at)
    }

    fn root_meta_for_import_for_drive(
        &self,
        drive_id: &str,
        created_at: i64,
    ) -> Option<DriveRootMeta> {
        let account = self.config.profile.as_ref()?;
        let drive = self.config.drive(drive_id)?;
        let previous = drive.app_key_roots.get(&account.app_key_pubkey);
        let app_key_seq = previous.map_or(1, |root| root.app_key_seq.saturating_add(1).max(1));
        let parents = previous
            .map(|root| RootParent {
                app_key_pubkey: account.app_key_pubkey.clone(),
                app_key_seq: root.app_key_seq,
                root_cid: root.root_cid.clone(),
            })
            .into_iter()
            .collect();
        let observed = drive
            .active_app_key_roots(Some(account))
            .into_iter()
            .filter(|(_, root)| root.app_key_seq > 0)
            .map(|(app_key_pubkey, root)| {
                (
                    app_key_pubkey.clone(),
                    RootObservation {
                        app_key_seq: root.app_key_seq,
                        root_cid: root.root_cid.clone(),
                    },
                )
            })
            .collect();
        let dck_generation = account
            .current_app_keys_projection()
            .map_or(0, |snap| snap.dck_generation);
        Some(DriveRootMeta {
            schema: DriveRootMeta::SCHEMA,
            drive_id: drive.drive_id.clone(),
            app_key_pubkey: account.app_key_pubkey.clone(),
            app_key_seq,
            dck_generation,
            local_only: false,
            parents,
            observed,
            created_at,
        })
    }

    fn ensure_drive_for_import(&mut self, drive_id: &str) -> Result<(), DaemonError> {
        if self.config.drive(drive_id).is_some() {
            return Ok(());
        }
        if drive_id == PRIMARY_DRIVE_ID {
            return Err(DaemonError::PrimaryDriveMissing);
        }
        if drive_id != CALENDAR_TREE_NAME {
            return Err(DaemonError::DriveMissing(drive_id.to_string()));
        }
        let root_scope_id = self
            .config
            .profile
            .as_ref()
            .map(super::profile::ProfileState::root_scope_id)
            .ok_or_else(|| DaemonError::DriveMissing(drive_id.to_string()))?;
        self.config.upsert_drive(Drive {
            root_scope_id,
            drive_id: CALENDAR_TREE_NAME.into(),
            display_name: "Calendar".into(),
            role: DriveRole::Owner,
            app_key_roots: std::collections::BTreeMap::new(),
            last_root_cid: None,
            key_hex: None,
        });
        Ok(())
    }
}

fn full_profile_covers_known_app_key_roots(
    profile: &crate::profile::ProfileState,
    config: &AppConfig,
) -> bool {
    if !profile.has_profile_roster_evidence() {
        return true;
    }
    let active_app_keys = profile
        .active_root_writer_app_key_pubkeys()
        .into_iter()
        .collect::<BTreeSet<_>>();
    let projection = profile.profile_projection();
    config.drives.iter().all(|drive| {
        drive.app_key_roots.keys().all(|app_key_pubkey| {
            active_app_keys.contains(app_key_pubkey)
                || projection.tombstones.contains_key(app_key_pubkey)
        })
    })
}

async fn current_root_observes_drive_roots(
    tree: &HashTree<FsBlobStore>,
    current_root: &Cid,
    account: &crate::profile::ProfileState,
    drive: &crate::config::Drive,
) -> Result<bool, DaemonError> {
    let Some(meta) = read_root_meta(tree, current_root).await? else {
        return Ok(false);
    };
    Ok(drive
        .active_app_key_roots(Some(account))
        .into_iter()
        .all(|(app_key_pubkey, root)| {
            app_key_pubkey == &account.app_key_pubkey
                || root.local_only
                || root.app_key_seq == 0
                || meta
                    .observed
                    .get(app_key_pubkey)
                    .is_some_and(|observed| root_observation_covers(observed, root))
        }))
}

fn root_observation_covers(observed: &RootObservation, root: &AppKeyRootRef) -> bool {
    observed.root_cid == root.root_cid
        || (root.app_key_seq > 0 && observed.app_key_seq >= root.app_key_seq)
}

fn unix_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests;
