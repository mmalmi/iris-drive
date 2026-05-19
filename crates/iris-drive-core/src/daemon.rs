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

use hashtree_core::{Cid, HashTree, HashTreeConfig};
use hashtree_fs::FsBlobStore;
use thiserror::Error;

use crate::config::{AppConfig, ConfigError, Drive};
use crate::indexer::{index_dir, IndexError};
use crate::paths::{config_path_in, key_path_in};

pub const PRIMARY_DRIVE_ID: &str = "main";

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
    #[error("store: {0}")]
    Store(String),
    #[error("primary drive missing from config (expected drive_id={PRIMARY_DRIVE_ID})")]
    PrimaryDriveMissing,
}

/// Snapshot of an import operation, suitable for serializing to JSON.
#[derive(Debug, Clone)]
pub struct ImportReport {
    pub root_cid: String,
    pub working_dir: PathBuf,
    pub top_level_entries: usize,
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
        let store =
            FsBlobStore::new(&blocks_dir).map_err(|e| DaemonError::Store(e.to_string()))?;
        let tree = Arc::new(HashTree::new(
            HashTreeConfig::new(Arc::new(store)).public(),
        ));
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

    /// CID currently recorded for the primary drive, if any.
    #[must_use]
    pub fn primary_root(&self) -> Option<&str> {
        self.config
            .drive(PRIMARY_DRIVE_ID)
            .and_then(|d| d.last_root_cid.as_deref())
    }

    /// Bulk-index `working_dir` into the daemon's persistent store and
    /// stamp the resulting root CID onto the primary drive. The previous
    /// root remains addressable in the store; nothing is GC'd.
    pub async fn import_working_dir(
        &mut self,
        working_dir: impl AsRef<Path>,
    ) -> Result<ImportReport, DaemonError> {
        let working_dir = working_dir.as_ref();
        let root_cid = index_dir(&self.tree, working_dir).await?;

        let listing = self
            .tree
            .list_directory(&root_cid)
            .await
            .map_err(|e| DaemonError::Store(e.to_string()))?;

        self.update_primary_drive(&root_cid)?;

        Ok(ImportReport {
            root_cid: root_cid.to_string(),
            working_dir: working_dir.to_path_buf(),
            top_level_entries: listing.len(),
        })
    }

    fn update_primary_drive(&mut self, root_cid: &Cid) -> Result<(), DaemonError> {
        let drive = match self.config.drive(PRIMARY_DRIVE_ID) {
            Some(d) => d.clone(),
            None => return Err(DaemonError::PrimaryDriveMissing),
        };
        let updated = Drive {
            last_root_cid: Some(root_cid.to_string()),
            ..drive
        };
        self.config.upsert_drive(updated);
        self.config.save(config_path_in(&self.config_dir))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Drive;
    use crate::identity::Identity;
    use tempfile::tempdir;

    fn init_config(dir: &Path) -> Identity {
        let identity = Identity::generate(key_path_in(dir));
        identity.save().unwrap();
        let mut cfg = AppConfig::default();
        cfg.upsert_drive(Drive::primary(identity.pubkey_hex()));
        cfg.save(config_path_in(dir)).unwrap();
        identity
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
    async fn blocks_dir_is_under_config_dir() {
        let cfg_dir = tempdir().unwrap();
        init_config(cfg_dir.path());
        let daemon = Daemon::open(cfg_dir.path()).unwrap();
        assert!(daemon.blocks_dir().starts_with(cfg_dir.path()));
        assert!(daemon.blocks_dir().ends_with("blocks"));
        // Directory exists on disk.
        assert!(daemon.blocks_dir().is_dir());
    }
}
