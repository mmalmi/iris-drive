//! Persistent daemon supervisor.
//!
//! Owns:
//! - A filesystem-backed hashtree store at `<config_dir>/blocks/`.
//! - The user's `AppConfig` (drives, schema, identity reference).
//! - A working-directory location for the primary drive.
//!
//! Stays minimal for v1: one-shot import + status. Long-running watchers
//! and live Nostr publish/subscribe land in a follow-up phase.

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
    IndexError, index_dir_with_history_and_meta, layer_conflict_records, layer_prev_link,
    layer_root_meta, read_conflict_records,
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
    #[error("sync cache: {0}")]
    SyncCache(#[from] SyncCacheError),
    #[error("materialize: {0}")]
    Materialize(#[from] crate::MaterializeError),
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
    pub working_dir: PathBuf,
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

/// Snapshot of applying the merged drive view to the working directory.
#[derive(Debug, Clone)]
pub struct MaterializeWorkingDirReport {
    pub materialize: crate::MaterializeReport,
    pub import: Option<ImportReport>,
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

    /// If the primary drive has a `working_dir` configured but this
    /// device hasn't yet recorded a private root, run the initial import.
    /// This also migrates legacy public roots created before private
    /// htree storage became the default. Creates the working dir on disk
    /// if it doesn't exist. Returns `Some(report)` if an import ran,
    /// `None` otherwise.
    pub async fn ensure_initial_import(&mut self) -> Result<Option<ImportReport>, DaemonError> {
        let Some(account) = self.config.account.clone() else {
            return Ok(None);
        };
        let Some(drive) = self.config.drive(PRIMARY_DRIVE_ID).cloned() else {
            return Ok(None);
        };
        let has_private_root = match drive.device_roots.get(&account.device_pubkey) {
            Some(root) => Cid::parse(&root.root_cid)
                .map_err(|e| DaemonError::Store(e.to_string()))?
                .key
                .is_some(),
            None => false,
        };
        if has_private_root {
            return Ok(None);
        }
        let Some(working_dir) = drive.working_dir.clone() else {
            return Ok(None);
        };
        std::fs::create_dir_all(&working_dir)?;
        let report = self.import_working_dir(&working_dir).await?;
        Ok(Some(report))
    }

    /// Bulk-index `working_dir` into the daemon's persistent store and
    /// stamp the resulting root CID onto the primary drive. The previous
    /// root remains addressable in the store; nothing is GC'd.
    pub async fn import_working_dir(
        &mut self,
        working_dir: impl AsRef<Path>,
    ) -> Result<ImportReport, DaemonError> {
        self.import_working_dir_inner(working_dir, false).await
    }

    async fn import_materialized_working_dir(
        &mut self,
        working_dir: impl AsRef<Path>,
    ) -> Result<ImportReport, DaemonError> {
        self.import_working_dir_inner(working_dir, true).await
    }

    async fn import_working_dir_inner(
        &mut self,
        working_dir: impl AsRef<Path>,
        materialized_only: bool,
    ) -> Result<ImportReport, DaemonError> {
        let working_dir = working_dir.as_ref();
        // Look up this device's previous root, if any, so the indexer
        // can diff against it and emit tombstones for removed files.
        let previous_root_cid = self
            .config
            .account
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

        let now = unix_now();
        let root_meta = self.root_meta_for_import(now);
        let root_cid = index_dir_with_history_and_meta(
            &self.tree,
            working_dir,
            previous_root.as_ref(),
            now,
            root_meta.as_ref(),
        )
        .await?;

        // Live-file count excludes the internal metadata directory from the
        // report.
        let listing = self
            .tree
            .list_directory(&root_cid)
            .await
            .map_err(|e| DaemonError::Store(e.to_string()))?;
        let top_level_entries = listing
            .iter()
            .filter(|e| e.name != crate::merge::META_DIR)
            .count();
        let (files, _tombstones) = crate::merge::walk_device_tree(&self.tree, &root_cid)
            .await
            .map_err(|e| DaemonError::Store(e.to_string()))?;

        self.update_primary_drive(
            &root_cid,
            Some(working_dir),
            root_meta.as_ref(),
            now,
            materialized_only,
        )?;
        self.persist_sync_cache_with_current_base().await?;

        Ok(ImportReport {
            root_cid: root_cid.to_string(),
            working_dir: working_dir.to_path_buf(),
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
        let now = unix_now();
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
        self.update_primary_drive(&root, None, root_meta.as_ref(), now, false)?;
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
        if let Some(account) = self.config.account.as_ref()
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

    /// Apply the merged primary drive view to this device's working directory.
    /// If files changed on disk, re-import and persist a new local root.
    pub async fn materialize_primary_drive(
        &mut self,
    ) -> Result<Option<MaterializeWorkingDirReport>, DaemonError> {
        let Some(working_dir) = self
            .config
            .drive(PRIMARY_DRIVE_ID)
            .and_then(|drive| drive.working_dir.clone())
        else {
            return Ok(None);
        };

        let materialize = crate::materialize_primary_drive(
            self.tree_handle(),
            &self.config,
            working_dir.as_path(),
        )
        .await?;
        let import = if materialize.changed() {
            Some(self.import_materialized_working_dir(&working_dir).await?)
        } else {
            None
        };
        Ok(Some(MaterializeWorkingDirReport {
            materialize,
            import,
        }))
    }

    async fn persist_sync_cache_with_current_base(&self) -> Result<(), DaemonError> {
        let mut cache =
            SyncCache::rebuild_from_config(&self.tree, &self.config, unix_now()).await?;
        if let Some(account) = self.config.account.as_ref() {
            cache.set_current_device_base(PRIMARY_DRIVE_ID, &account.device_pubkey);
        }
        cache.save(sync_cache_path_in(&self.config_dir))?;
        Ok(())
    }

    fn update_primary_drive(
        &mut self,
        root_cid: &Cid,
        working_dir: Option<&Path>,
        root_meta: Option<&DriveRootMeta>,
        published_at: i64,
        materialized_only: bool,
    ) -> Result<(), DaemonError> {
        let drive = match self.config.drive(PRIMARY_DRIVE_ID) {
            Some(d) => d.clone(),
            None => return Err(DaemonError::PrimaryDriveMissing),
        };
        let mut updated = drive;
        updated.last_root_cid = Some(root_cid.to_string());
        if let Some(wd) = working_dir {
            updated.working_dir = Some(wd.to_path_buf());
        }

        // Per-device root entry, keyed by this device's pubkey.
        // Falls back to no-op when there is no account yet (legacy
        // installs from before the multi-device split).
        if let Some(account) = self.config.account.as_ref() {
            let dck_generation = account
                .app_keys
                .as_ref()
                .map_or(0, |snap| snap.dck_generation);
            let mut device_root = root_meta.map_or_else(
                || DeviceRootRef::legacy(root_cid.to_string(), published_at, dck_generation),
                |meta| DeviceRootRef::from_meta(root_cid.to_string(), published_at, meta),
            );
            device_root.materialized_only = materialized_only;
            updated
                .device_roots
                .insert(account.device_pubkey.clone(), device_root);
        }

        self.config.upsert_drive(updated);
        self.config.save(config_path_in(&self.config_dir))?;
        Ok(())
    }

    fn root_meta_for_import(&self, created_at: i64) -> Option<DriveRootMeta> {
        let account = self.config.account.as_ref()?;
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
            parents,
            observed,
            created_at,
        })
    }
}

fn unix_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Drive;
    use crate::conflict::{ConflictRecord, ConflictSide, ConflictState};
    use crate::identity::Identity;
    use std::sync::Arc;
    use tempfile::tempdir;

    fn init_config(dir: &Path) -> Identity {
        let identity = Identity::generate(key_path_in(dir));
        identity.save().unwrap();
        let mut cfg = AppConfig::default();
        cfg.upsert_drive(Drive::primary(identity.pubkey_hex()));
        cfg.save(config_path_in(dir)).unwrap();
        identity
    }

    /// Spin up a real `Account` via the create flow, then save the
    /// `AccountState` into `AppConfig`. Used to exercise the per-device
    /// root code path.
    fn init_config_with_account(dir: &Path) -> crate::account::Account {
        let account = crate::account::Account::create(dir, Some("test-device".into())).unwrap();
        let mut cfg = AppConfig {
            account: Some(account.state.clone()),
            ..AppConfig::default()
        };
        cfg.upsert_drive(Drive::primary(account.state.owner_pubkey.clone()));
        cfg.save(config_path_in(dir)).unwrap();
        account
    }

    #[tokio::test]
    async fn import_persists_working_dir_on_primary_drive() {
        let cfg_dir = tempdir().unwrap();
        init_config_with_account(cfg_dir.path());
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

        let work = tempdir().unwrap();
        std::fs::write(work.path().join("a.txt"), b"a").unwrap();
        daemon.import_working_dir(work.path()).await.unwrap();

        let drive = daemon.config().drive(PRIMARY_DRIVE_ID).unwrap();
        assert_eq!(drive.working_dir.as_deref(), Some(work.path()));

        // Survives reopen.
        drop(daemon);
        let reopened = Daemon::open(cfg_dir.path()).unwrap();
        assert_eq!(
            reopened
                .config()
                .drive(PRIMARY_DRIVE_ID)
                .unwrap()
                .working_dir
                .as_deref(),
            Some(work.path())
        );
    }

    #[tokio::test]
    async fn materialized_import_is_local_only_until_user_imports() {
        let cfg_dir = tempdir().unwrap();
        let account = init_config_with_account(cfg_dir.path());
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

        let work = tempdir().unwrap();
        std::fs::write(work.path().join("note.txt"), b"from peer").unwrap();
        daemon
            .import_materialized_working_dir(work.path())
            .await
            .unwrap();
        let root = daemon
            .config()
            .drive(PRIMARY_DRIVE_ID)
            .unwrap()
            .device_roots
            .get(&account.state.device_pubkey)
            .unwrap();
        assert!(root.materialized_only);

        std::fs::write(work.path().join("note.txt"), b"local edit").unwrap();
        daemon.import_working_dir(work.path()).await.unwrap();
        let root = daemon
            .config()
            .drive(PRIMARY_DRIVE_ID)
            .unwrap()
            .device_roots
            .get(&account.state.device_pubkey)
            .unwrap();
        assert!(!root.materialized_only);
    }

    #[tokio::test]
    async fn import_persists_rebuildable_sync_cache_with_base_state() {
        let cfg_dir = tempdir().unwrap();
        let account = init_config_with_account(cfg_dir.path());
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

        let work = tempdir().unwrap();
        std::fs::write(work.path().join("note.txt"), b"hello cache").unwrap();
        let report = daemon.import_working_dir(work.path()).await.unwrap();

        let cache =
            crate::sync_cache::SyncCache::load(crate::paths::sync_cache_path_in(cfg_dir.path()))
                .unwrap();
        assert_eq!(cache.schema, crate::sync_cache::SyncCache::SCHEMA);
        assert_eq!(cache.roots.len(), 1);
        assert_eq!(cache.roots[0].device_id, account.state.device_pubkey);
        assert_eq!(cache.roots[0].root_cid, report.root_cid);
        assert_eq!(cache.path_state.len(), 1);
        assert_eq!(cache.path_state[0].path, "note.txt");
        assert_eq!(cache.path_state[0].root_cid, report.root_cid);
        assert!(cache.path_state[0].whole_file_hash.is_some());
        assert_eq!(cache.base_state.len(), 1);
        assert_eq!(cache.base_state[0].path, "note.txt");
        assert_eq!(cache.base_state[0].base_root_cid, report.root_cid);
        assert_eq!(
            cache.base_anchor_for_drive(PRIMARY_DRIVE_ID),
            Some(report.root_cid.as_str())
        );

        std::fs::remove_file(crate::paths::sync_cache_path_in(cfg_dir.path())).unwrap();
        let rebuilt = daemon.rebuild_sync_cache().await.unwrap();
        assert_eq!(rebuilt.roots.len(), 1);
        assert_eq!(rebuilt.path_state.len(), 1);
        assert_eq!(rebuilt.path_state[0].path, "note.txt");
        assert!(
            rebuilt.base_state.is_empty(),
            "rebuilds restore current state but not historical base quality"
        );
    }

    #[tokio::test]
    async fn corrupt_sync_cache_rebuilds_from_signed_roots() {
        let cfg_dir = tempdir().unwrap();
        init_config_with_account(cfg_dir.path());
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

        let work = tempdir().unwrap();
        std::fs::write(work.path().join("note.txt"), b"hello cache").unwrap();
        let report = daemon.import_working_dir(work.path()).await.unwrap();

        let cache_path = crate::paths::sync_cache_path_in(cfg_dir.path());
        std::fs::write(&cache_path, b"{ definitely not json").unwrap();
        let rebuilt = daemon.load_or_rebuild_sync_cache().await.unwrap();

        assert_eq!(rebuilt.roots.len(), 1);
        assert_eq!(rebuilt.path_state.len(), 1);
        assert_eq!(rebuilt.path_state[0].path, "note.txt");
        assert_eq!(rebuilt.path_state[0].root_cid, report.root_cid);
        assert!(rebuilt.base_state.is_empty());

        let loaded = crate::sync_cache::SyncCache::load(cache_path).unwrap();
        assert_eq!(loaded.path_state, rebuilt.path_state);
    }

    #[tokio::test]
    async fn import_records_per_device_root_when_account_present() {
        let cfg_dir = tempdir().unwrap();
        let account = init_config_with_account(cfg_dir.path());
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

        let work = tempdir().unwrap();
        std::fs::write(work.path().join("hello.txt"), b"hi").unwrap();
        let report = daemon.import_working_dir(work.path()).await.unwrap();

        let drive = daemon.config().drive(PRIMARY_DRIVE_ID).unwrap();
        assert_eq!(drive.device_roots.len(), 1);
        let entry = drive
            .device_roots
            .get(&account.state.device_pubkey)
            .expect("per-device root for this device");
        assert_eq!(entry.root_cid, report.root_cid);
        assert!(entry.published_at > 0);
        assert_eq!(entry.dck_generation, 1); // create-flow seeds DCK gen 1
        assert_eq!(entry.device_seq, 1);
        assert!(entry.parents.is_empty());
    }

    #[tokio::test]
    async fn import_embeds_root_meta_and_advances_device_sequence() {
        let cfg_dir = tempdir().unwrap();
        let account = init_config_with_account(cfg_dir.path());
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

        let work = tempdir().unwrap();
        std::fs::write(work.path().join("note.txt"), b"one").unwrap();
        let first = daemon.import_working_dir(work.path()).await.unwrap();
        let first_cid = Cid::parse(&first.root_cid).unwrap();
        let first_meta = crate::indexer::read_root_meta(daemon.tree(), &first_cid)
            .await
            .unwrap()
            .expect("first root metadata");
        assert_eq!(first_meta.schema, crate::DriveRootMeta::SCHEMA);
        assert_eq!(first_meta.drive_id, PRIMARY_DRIVE_ID);
        assert_eq!(first_meta.device_id, account.state.device_pubkey);
        assert_eq!(first_meta.device_seq, 1);
        assert!(first_meta.parents.is_empty());

        std::fs::write(work.path().join("note.txt"), b"two").unwrap();
        let second = daemon.import_working_dir(work.path()).await.unwrap();
        let second_cid = Cid::parse(&second.root_cid).unwrap();
        let second_meta = crate::indexer::read_root_meta(daemon.tree(), &second_cid)
            .await
            .unwrap()
            .expect("second root metadata");
        assert_eq!(second_meta.device_seq, 2);
        assert_eq!(second_meta.parents.len(), 1);
        assert_eq!(second_meta.parents[0].device_seq, 1);
        assert_eq!(second_meta.parents[0].root_cid, first.root_cid);

        let entry = daemon
            .config()
            .drive(PRIMARY_DRIVE_ID)
            .unwrap()
            .device_roots
            .get(&account.state.device_pubkey)
            .unwrap();
        assert_eq!(entry.root_cid, second.root_cid);
        assert_eq!(entry.device_seq, 2);
        assert_eq!(entry.parents, second_meta.parents);
    }

    #[tokio::test]
    async fn import_uses_encrypted_private_hashtree_blocks() {
        let cfg_dir = tempdir().unwrap();
        init_config_with_account(cfg_dir.path());
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

        let work = tempdir().unwrap();
        let secret = b"secret contents that must not appear as plaintext in blobs";
        std::fs::write(work.path().join("secret.txt"), secret).unwrap();
        let report = daemon.import_working_dir(work.path()).await.unwrap();

        let cid = Cid::parse(&report.root_cid).unwrap();
        assert!(
            cid.key.is_some(),
            "persistent drive roots must carry a CHK key"
        );

        let mut stack = vec![daemon.blocks_dir().to_path_buf()];
        let mut saw_blob = false;
        while let Some(path) = stack.pop() {
            for entry in std::fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    saw_blob = true;
                    let bytes = std::fs::read(&path).unwrap();
                    assert!(
                        !bytes.windows(secret.len()).any(|window| window == secret),
                        "stored blob {} leaked plaintext",
                        path.display()
                    );
                }
            }
        }
        assert!(saw_blob, "import should write blobs");
    }

    #[tokio::test]
    async fn resolve_conflict_record_marks_record_resolved_and_advances_root() {
        let cfg_dir = tempdir().unwrap();
        init_config_with_account(cfg_dir.path());
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();

        let work = tempdir().unwrap();
        std::fs::write(work.path().join("report.pdf"), b"chosen").unwrap();
        let imported = daemon.import_working_dir(work.path()).await.unwrap();
        let imported_root = Cid::parse(&imported.root_cid).unwrap();
        let record = conflict_record("conflict-a");

        let mut root_with_conflict =
            crate::indexer::layer_conflict_records(daemon.tree(), imported_root.clone(), &[record])
                .await
                .unwrap();
        root_with_conflict =
            crate::indexer::layer_prev_link(daemon.tree(), root_with_conflict, &imported_root)
                .await
                .unwrap();
        let now = unix_now();
        let root_meta = daemon.root_meta_for_import(now).unwrap();
        root_with_conflict =
            crate::indexer::layer_root_meta(daemon.tree(), root_with_conflict, &root_meta)
                .await
                .unwrap();
        daemon
            .update_primary_drive(&root_with_conflict, None, Some(&root_meta), now, false)
            .unwrap();

        let report = daemon.resolve_conflict_record("conflict-a").await.unwrap();

        assert!(report.changed);
        assert_eq!(report.previous_root_cid, root_with_conflict.to_string());
        assert_ne!(report.root_cid, report.previous_root_cid);
        let resolved_root = Cid::parse(&report.root_cid).unwrap();
        let records = crate::indexer::read_conflict_records(daemon.tree(), &resolved_root)
            .await
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].state, ConflictState::Resolved);
        let resolved_meta = crate::indexer::read_root_meta(daemon.tree(), &resolved_root)
            .await
            .unwrap()
            .expect("resolved root metadata");
        assert_eq!(
            resolved_meta.parents[0].root_cid,
            root_with_conflict.to_string()
        );
    }

    fn conflict_record(conflict_id: &str) -> ConflictRecord {
        ConflictRecord {
            schema: ConflictRecord::SCHEMA,
            conflict_id: conflict_id.into(),
            path: "report.pdf".into(),
            visible_conflict_path: "report (conflict from phone).pdf".into(),
            local: ConflictSide {
                device_id: "laptop".into(),
                device_seq: 2,
                root_cid: "cid-local".into(),
                whole_file_hash: "hash-local".into(),
            },
            remote: Some(ConflictSide {
                device_id: "phone".into(),
                device_seq: 7,
                root_cid: "cid-remote".into(),
                whole_file_hash: "hash-remote".into(),
            }),
            deleted: None,
            state: ConflictState::Unresolved,
            created_at: 1234,
        }
    }

    #[tokio::test]
    async fn open_uninitialized_errors() {
        let dir = tempdir().unwrap();
        match Daemon::open(dir.path()) {
            Err(DaemonError::Uninitialized) => {}
            Err(other) => panic!("expected Uninitialized, got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[tokio::test]
    async fn import_persists_root_cid_to_config() {
        let cfg_dir = tempdir().unwrap();
        init_config(cfg_dir.path());
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        assert!(daemon.primary_root().is_none());

        let work = tempdir().unwrap();
        std::fs::write(work.path().join("hello.txt"), b"hi there").unwrap();
        let report = daemon.import_working_dir(work.path()).await.unwrap();
        assert_eq!(report.top_level_entries, 1);
        assert!(!report.root_cid.is_empty());

        // primary drive's last_root_cid is set.
        let recorded = daemon.primary_root().unwrap();
        assert_eq!(recorded, report.root_cid);

        // a fresh open sees the same state.
        let reopened = Daemon::open(cfg_dir.path()).unwrap();
        assert_eq!(reopened.primary_root(), Some(report.root_cid.as_str()));
    }

    #[tokio::test]
    async fn import_survives_across_daemon_restarts() {
        let cfg_dir = tempdir().unwrap();
        init_config(cfg_dir.path());

        let work = tempdir().unwrap();
        std::fs::write(work.path().join("a.txt"), b"alpha").unwrap();

        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        let report = daemon.import_working_dir(work.path()).await.unwrap();
        let root_cid = report.root_cid.clone();
        drop(daemon);

        // Re-open and confirm we can still list the persisted root.
        let daemon = Daemon::open(cfg_dir.path()).unwrap();
        let cid = Cid::parse(&root_cid).unwrap();
        let listing = daemon.tree().list_directory(&cid).await.unwrap();
        assert_eq!(listing.len(), 1);
        assert_eq!(listing[0].name, "a.txt");
    }

    #[tokio::test]
    async fn re_import_records_new_root() {
        let cfg_dir = tempdir().unwrap();
        init_config(cfg_dir.path());
        let work = tempdir().unwrap();

        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        std::fs::write(work.path().join("a.txt"), b"first").unwrap();
        let first = daemon.import_working_dir(work.path()).await.unwrap();

        std::fs::write(work.path().join("b.txt"), b"second").unwrap();
        let second = daemon.import_working_dir(work.path()).await.unwrap();

        assert_ne!(first.root_cid, second.root_cid);
        assert_eq!(daemon.primary_root().unwrap(), second.root_cid);
    }

    #[tokio::test]
    async fn import_without_primary_drive_errors() {
        let cfg_dir = tempdir().unwrap();
        // identity present but no drives in config
        let identity = Identity::generate(key_path_in(cfg_dir.path()));
        identity.save().unwrap();
        AppConfig::default()
            .save(config_path_in(cfg_dir.path()))
            .unwrap();

        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        let work = tempdir().unwrap();
        match daemon.import_working_dir(work.path()).await {
            Err(DaemonError::PrimaryDriveMissing) => {}
            other => panic!("expected PrimaryDriveMissing, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn ensure_initial_import_runs_when_working_dir_set_but_no_root() {
        let cfg_dir = tempdir().unwrap();
        let account = init_config_with_account(cfg_dir.path());

        // Pre-stage a working_dir on the primary drive (mimics the
        // tray-app bootstrap), but no device_roots entry yet.
        let work = tempdir().unwrap();
        std::fs::write(work.path().join("note.txt"), b"hello").unwrap();
        {
            let mut cfg = AppConfig::load_or_default(config_path_in(cfg_dir.path())).unwrap();
            let drive = cfg
                .drives
                .iter_mut()
                .find(|d| d.drive_id == PRIMARY_DRIVE_ID)
                .unwrap();
            drive.working_dir = Some(work.path().to_path_buf());
            cfg.save(config_path_in(cfg_dir.path())).unwrap();
        }

        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        let report = daemon.ensure_initial_import().await.unwrap();
        let report = report.expect("initial import should run");
        assert_eq!(report.working_dir, work.path());

        let drive = daemon.config().drive(PRIMARY_DRIVE_ID).unwrap();
        assert!(
            drive
                .device_roots
                .contains_key(&account.state.device_pubkey)
        );

        // Second call is a no-op.
        let again = daemon.ensure_initial_import().await.unwrap();
        assert!(again.is_none());
    }

    #[tokio::test]
    async fn ensure_initial_import_creates_working_dir_if_missing() {
        let cfg_dir = tempdir().unwrap();
        init_config_with_account(cfg_dir.path());

        // Point working_dir at a path that does NOT exist yet.
        let work_parent = tempdir().unwrap();
        let missing = work_parent.path().join("Iris Drive");
        assert!(!missing.exists());
        {
            let mut cfg = AppConfig::load_or_default(config_path_in(cfg_dir.path())).unwrap();
            let drive = cfg
                .drives
                .iter_mut()
                .find(|d| d.drive_id == PRIMARY_DRIVE_ID)
                .unwrap();
            drive.working_dir = Some(missing.clone());
            cfg.save(config_path_in(cfg_dir.path())).unwrap();
        }

        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        daemon.ensure_initial_import().await.unwrap();
        assert!(
            missing.is_dir(),
            "ensure_initial_import should create the working dir"
        );
    }

    #[tokio::test]
    async fn ensure_initial_import_reencrypts_legacy_public_root() {
        let cfg_dir = tempdir().unwrap();
        let account = init_config_with_account(cfg_dir.path());
        let work = tempdir().unwrap();
        std::fs::write(work.path().join("secret.txt"), b"legacy plaintext").unwrap();

        let blocks_dir = cfg_dir.path().join("blocks");
        std::fs::create_dir_all(&blocks_dir).unwrap();
        let store = FsBlobStore::new(&blocks_dir).unwrap();
        let public_tree = HashTree::new(HashTreeConfig::new(Arc::new(store)).public());
        let public_root =
            crate::indexer::index_dir_with_history(&public_tree, work.path(), None, unix_now())
                .await
                .unwrap();
        assert!(public_root.key.is_none());

        {
            let mut cfg = AppConfig::load_or_default(config_path_in(cfg_dir.path())).unwrap();
            let drive = cfg
                .drives
                .iter_mut()
                .find(|d| d.drive_id == PRIMARY_DRIVE_ID)
                .unwrap();
            drive.working_dir = Some(work.path().to_path_buf());
            drive.device_roots.insert(
                account.state.device_pubkey.clone(),
                DeviceRootRef::legacy(
                    public_root.to_string(),
                    unix_now(),
                    account.state.app_keys.as_ref().unwrap().dck_generation,
                ),
            );
            cfg.save(config_path_in(cfg_dir.path())).unwrap();
        }

        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        let report = daemon
            .ensure_initial_import()
            .await
            .unwrap()
            .expect("legacy public root should be re-imported");

        assert_ne!(report.root_cid, public_root.to_string());
        let private_root = Cid::parse(&report.root_cid).unwrap();
        assert!(private_root.key.is_some());
    }

    #[tokio::test]
    async fn ensure_initial_import_noop_without_working_dir() {
        let cfg_dir = tempdir().unwrap();
        init_config_with_account(cfg_dir.path());
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        let report = daemon.ensure_initial_import().await.unwrap();
        assert!(report.is_none());
    }

    #[tokio::test]
    async fn blocks_dir_is_under_config_dir() {
        let cfg_dir = tempdir().unwrap();
        init_config(cfg_dir.path());
        let daemon = Daemon::open(cfg_dir.path()).unwrap();
        assert!(daemon.blocks_dir().starts_with(cfg_dir.path()));
        assert!(daemon.blocks_dir().ends_with("blocks"));
        // Directory exists on disk.
        assert!(daemon.blocks_dir().is_dir());
    }

    #[test]
    fn embedded_hashtree_state_uses_app_data_sibling_for_native_layout() {
        assert_eq!(
            embedded_hashtree_state_root_in(Path::new("/tmp/IrisDrive/AppData/Config")),
            PathBuf::from("/tmp/IrisDrive/AppData/Hashtree")
        );
    }

    #[test]
    fn embedded_hashtree_state_uses_config_child_for_plain_cli_layout() {
        assert_eq!(
            embedded_hashtree_state_root_in(Path::new("/tmp/iris-drive")),
            PathBuf::from("/tmp/iris-drive/Hashtree")
        );
    }
}
