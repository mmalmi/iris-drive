//! Persistent daemon supervisor.
//!
//! Owns:
//! - A filesystem-backed hashtree store at `<config_dir>/blocks/`.
//! - The user's `AppConfig` (drives, schema, identity reference).
//! - Virtual mount/provider roots for the primary drive.
//!
//! Stays minimal for v1: one-shot import + status. Long-running sync is
//! handled by the CLI daemon over virtual roots/providers.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use hashtree_core::{Cid, HashTree, HashTreeConfig};
use hashtree_fs::FsBlobStore;
use serde_json::json;
use thiserror::Error;

use crate::config::{AppConfig, ConfigError, DeviceRootRef};
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

pub struct EmbeddedHashtreeHost {
    runtime: hashtree_embedded::HostDaemonRuntime,
    status: hashtree_embedded::HostDaemonStatus,
}

impl EmbeddedHashtreeHost {
    pub fn start(config_dir: &Path, config: &AppConfig) -> Result<Self> {
        let state_root = embedded_hashtree_state_root_in(config_dir);
        let embedded_config_dir = state_root.join("config");
        std::fs::create_dir_all(&embedded_config_dir)
            .with_context(|| format!("creating {}", embedded_config_dir.display()))?;
        let settings = json!({
            "nostrRelays": config.relays.clone(),
            "blossomReadServers": config.blossom_servers.clone(),
            "blossomWriteServers": config.blossom_servers.clone(),
            "enableWebrtc": false,
            "enableMulticast": false,
            "enableFips": false,
            "enableFipsUdp": false,
            "enableFipsWebrtc": false,
            "fetchFromFipsPeers": false,
            "socialGraphCrawlDepth": 0,
            "syncEnabled": false,
            "syncOwn": false,
            "syncFollowed": false,
            "publicWrites": false,
        });
        let settings_path = embedded_config_dir.join("browser_settings.json");
        std::fs::write(&settings_path, serde_json::to_vec_pretty(&settings)?)
            .with_context(|| format!("writing {}", settings_path.display()))?;
        let device_key_path = key_path_in(config_dir);
        if device_key_path.exists() {
            std::fs::copy(&device_key_path, embedded_config_dir.join("keys")).with_context(
                || {
                    format!(
                        "copying Iris Drive device key from {}",
                        device_key_path.display()
                    )
                },
            )?;
        }

        let runtime = hashtree_embedded::HostDaemonRuntime::start(
            hashtree_embedded::HostDaemonOptions::new(state_root),
        )?;
        let status = runtime.status();
        Ok(Self { runtime, status })
    }

    #[must_use]
    pub fn status(&self) -> &hashtree_embedded::HostDaemonStatus {
        &self.status
    }

    #[must_use]
    pub fn status_payload(&self) -> serde_json::Value {
        json!({
            "base_url": self.status.base_url.clone(),
            "self_npub": self.status.self_npub.clone(),
            "config_dir": self.status.config_dir.display().to_string(),
            "data_dir": self.status.data_dir.display().to_string(),
        })
    }
}

impl Drop for EmbeddedHashtreeHost {
    fn drop(&mut self) {
        self.runtime.shutdown();
    }
}

#[must_use]
pub fn embedded_hashtree_state_root_in(config_dir: &Path) -> PathBuf {
    if config_dir.file_name().and_then(|name| name.to_str()) == Some("Config")
        && let Some(app_data_dir) = config_dir.parent()
    {
        return app_data_dir.join("Hashtree");
    }
    config_dir.join("Hashtree")
}

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
        std::fs::create_dir_all(&config_dir)?;
        let blocks_dir = config_dir.join("blocks");
        std::fs::create_dir_all(&blocks_dir)?;
        let store = FsBlobStore::new(&blocks_dir).map_err(|e| DaemonError::Store(e.to_string()))?;
        let tree = Arc::new(HashTree::new(HashTreeConfig::new(Arc::new(store))));
        let config = AppConfig::load_or_default(config_path_in(&config_dir))?;
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
                    .and_then(|d| d.device_roots.get(&account.device_pubkey))
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
        self.import_visible_root_with_tombstone_base(root, None)
            .await
    }

    /// Publish this device's causal root for the currently merged drive view.
    ///
    /// Remote device roots are inputs. Once their blocks are local, this folds
    /// the merged visible view into this device's own root and records the
    /// observed source roots in `.hashtree/root.json`, so later roster changes
    /// do not make already-accepted files depend on a removed device root.
    pub async fn materialize_primary_merged_root(
        &mut self,
    ) -> Result<Option<ImportReport>, DaemonError> {
        let Some(account) = self.config.profile.clone() else {
            return Ok(None);
        };
        let drive = self
            .config
            .drive(PRIMARY_DRIVE_ID)
            .ok_or(DaemonError::PrimaryDriveMissing)?
            .clone();
        if drive.device_roots.is_empty() {
            return Ok(None);
        }

        let merged = crate::primary_merged_root(&self.tree, &self.config).await?;
        if let Some(current) = drive.device_roots.get(&account.device_pubkey) {
            let current_cid =
                Cid::parse(&current.root_cid).map_err(|e| DaemonError::Store(e.to_string()))?;
            let current_visible =
                crate::indexer::filter_ignored_entries_from_root(&self.tree, &current_cid).await?;
            if current_visible == merged.root_cid
                && current_root_observes_drive_roots(
                    &self.tree,
                    &current_cid,
                    &account.device_pubkey,
                    &drive,
                )
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
        self.import_visible_root_with_tombstone_base_and_paths(root, tombstone_base_root, None)
            .await
    }

    pub async fn import_visible_root_with_tombstone_base_and_paths(
        &mut self,
        root: Cid,
        tombstone_base_root: Option<Cid>,
        tombstone_paths: Option<&std::collections::BTreeSet<String>>,
    ) -> Result<ImportReport, DaemonError> {
        self.import_visible_root_with_tombstone_base_paths_and_local_only(
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
        self.import_visible_root_with_tombstone_base_paths_and_local_only(root, None, None, true)
            .await
    }

    async fn import_visible_root_with_tombstone_base_paths_and_local_only(
        &mut self,
        root: Cid,
        tombstone_base_root: Option<Cid>,
        tombstone_paths: Option<&std::collections::BTreeSet<String>>,
        local_only: bool,
    ) -> Result<ImportReport, DaemonError> {
        let previous_root_cid = self
            .config
            .profile
            .as_ref()
            .and_then(|account| {
                self.config
                    .drive(PRIMARY_DRIVE_ID)
                    .and_then(|d| d.device_roots.get(&account.device_pubkey))
            })
            .map(|entry| entry.root_cid.clone());
        let previous_root = match previous_root_cid.as_ref() {
            Some(s) => Some(Cid::parse(s).map_err(|e| DaemonError::Store(e.to_string()))?),
            None => None,
        };

        let now = self.next_import_timestamp();
        let mut root_meta = self.root_meta_for_import(now);
        if let Some(meta) = root_meta.as_mut() {
            meta.local_only = local_only;
        }
        let mut scoped_tombstone_paths = None;
        let import_root = if let Some(tombstone_base_root) = tombstone_base_root.as_ref() {
            let phase = std::time::Instant::now();
            let projection_root = crate::primary_merged_root(&self.tree, &self.config)
                .await?
                .root_cid;
            let delta = local_visible_root_for_mount_import(
                &self.tree,
                &root,
                previous_root.as_ref(),
                tombstone_base_root,
                Some(&projection_root),
                tombstone_paths,
            )
            .await?;
            tracing::debug!(
                elapsed_ms = phase.elapsed().as_millis(),
                "visible root import built local delta"
            );
            scoped_tombstone_paths = Some(delta.tombstone_paths);
            delta.root
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
                previous_root.as_ref(),
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
                previous_root.as_ref(),
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

        self.report_and_record_root(root_cid, None, root_meta.as_ref(), now)
            .await
    }

    async fn report_and_record_root(
        &mut self,
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
        let (files, _tombstones) = crate::merge::walk_device_tree(&self.tree, &root_cid)
            .await
            .map_err(|e| DaemonError::Store(e.to_string()))?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            "record root walked device tree"
        );

        let phase = std::time::Instant::now();
        self.update_primary_drive(&root_cid, root_meta, published_at)?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            "record root updated config"
        );
        let phase = std::time::Instant::now();
        self.persist_sync_cache_with_current_base().await?;
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
        let previous_root_cid = self.current_device_root_cid()?.to_string();
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
        self.persist_sync_cache_with_current_base().await?;

        Ok(ConflictResolveReport {
            conflict_id: conflict_id.to_string(),
            previous_root_cid,
            root_cid: root.to_string(),
            changed: true,
        })
    }

    fn current_device_root_cid(&self) -> Result<&str, DaemonError> {
        let drive = self
            .config
            .drive(PRIMARY_DRIVE_ID)
            .ok_or(DaemonError::PrimaryDriveMissing)?;
        if let Some(account) = self.config.profile.as_ref()
            && let Some(root) = drive.device_roots.get(&account.device_pubkey)
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

    async fn persist_sync_cache_with_current_base(&self) -> Result<(), DaemonError> {
        let Some(account) = self.config.profile.as_ref() else {
            let cache =
                SyncCache::rebuild_from_config(&self.tree, &self.config, unix_now()).await?;
            cache.save(sync_cache_path_in(&self.config_dir))?;
            return Ok(());
        };

        let path = sync_cache_path_in(&self.config_dir);
        let mut cache = SyncCache::load(&path).unwrap_or_else(|_| SyncCache::empty());
        if cache
            .replace_device_root_from_config(
                &self.tree,
                &self.config,
                PRIMARY_DRIVE_ID,
                &account.device_pubkey,
                unix_now(),
            )
            .await
            .is_err()
        {
            cache = SyncCache::rebuild_from_config(&self.tree, &self.config, unix_now()).await?;
            cache.set_current_device_base(PRIMARY_DRIVE_ID, &account.device_pubkey);
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
        let drive = match self.config.drive(PRIMARY_DRIVE_ID) {
            Some(d) => d.clone(),
            None => return Err(DaemonError::PrimaryDriveMissing),
        };
        let mut updated = drive;
        updated.last_root_cid = Some(root_cid.to_string());

        // Per-device root entry, keyed by this device's pubkey.
        // Falls back to no-op when there is no account yet (legacy
        // installs from before the multi-device split).
        if let Some(account) = self.config.profile.as_ref() {
            let dck_generation = account
                .app_keys
                .as_ref()
                .map_or(0, |snap| snap.dck_generation);
            let device_root = root_meta.map_or_else(
                || DeviceRootRef::legacy(root_cid.to_string(), published_at, dck_generation),
                |meta| DeviceRootRef::from_meta(root_cid.to_string(), published_at, meta),
            );
            updated
                .device_roots
                .insert(account.device_pubkey.clone(), device_root);
        }

        self.config.upsert_drive(updated);
        self.config.save(config_path_in(&self.config_dir))?;
        Ok(())
    }

    fn next_import_timestamp(&self) -> i64 {
        let now = unix_now();
        let previous = self.config.profile.as_ref().and_then(|account| {
            self.config
                .drive(PRIMARY_DRIVE_ID)
                .and_then(|drive| drive.device_roots.get(&account.device_pubkey))
        });
        previous.map_or(now, |root| now.max(root.published_at.saturating_add(1)))
    }

    fn root_meta_for_import(&self, created_at: i64) -> Option<DriveRootMeta> {
        let account = self.config.profile.as_ref()?;
        let drive = self.config.drive(PRIMARY_DRIVE_ID)?;
        let previous = drive.device_roots.get(&account.device_pubkey);
        let device_seq = previous.map_or(1, |root| root.device_seq.saturating_add(1).max(1));
        let parents = previous
            .map(|root| RootParent {
                device_id: account.device_pubkey.clone(),
                device_seq: root.device_seq,
                root_cid: root.root_cid.clone(),
            })
            .into_iter()
            .collect();
        let observed = drive
            .device_roots
            .iter()
            .filter(|(_, root)| root.device_seq > 0)
            .map(|(device_id, root)| {
                (
                    device_id.clone(),
                    RootObservation {
                        device_seq: root.device_seq,
                        root_cid: root.root_cid.clone(),
                    },
                )
            })
            .collect();
        let dck_generation = account
            .app_keys
            .as_ref()
            .map_or(0, |snap| snap.dck_generation);
        Some(DriveRootMeta {
            schema: DriveRootMeta::SCHEMA,
            drive_id: drive.drive_id.clone(),
            device_id: account.device_pubkey.clone(),
            device_seq,
            dck_generation,
            local_only: false,
            parents,
            observed,
            created_at,
        })
    }
}

async fn current_root_observes_drive_roots(
    tree: &HashTree<FsBlobStore>,
    current_root: &Cid,
    current_device: &str,
    drive: &crate::config::Drive,
) -> Result<bool, DaemonError> {
    let Some(meta) = read_root_meta(tree, current_root).await? else {
        return Ok(false);
    };
    Ok(drive.device_roots.iter().all(|(device_id, root)| {
        device_id == current_device
            || root.local_only
            || root.device_seq == 0
            || meta
                .observed
                .get(device_id)
                .is_some_and(|observed| root_observation_covers(observed, root))
    }))
}

fn root_observation_covers(observed: &RootObservation, root: &DeviceRootRef) -> bool {
    observed.root_cid == root.root_cid
        || (root.device_seq > 0 && observed.device_seq >= root.device_seq)
}

fn unix_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests;
