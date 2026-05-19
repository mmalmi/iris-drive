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

use crate::config::{AppConfig, ConfigError};
use crate::indexer::{IndexError, index_dir_with_history};
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

        let root_cid =
            index_dir_with_history(&self.tree, working_dir, previous_root.as_ref(), unix_now())
                .await?;

        // Live-file count excludes the .tombstones subtree from the
        // report (it's an internal-only annotation).
        let listing = self
            .tree
            .list_directory(&root_cid)
            .await
            .map_err(|e| DaemonError::Store(e.to_string()))?;
        let top_level_entries = listing
            .iter()
            .filter(|e| e.name != crate::merge::TOMBSTONE_PREFIX)
            .count();

        self.update_primary_drive(&root_cid, Some(working_dir))?;

        Ok(ImportReport {
            root_cid: root_cid.to_string(),
            working_dir: working_dir.to_path_buf(),
            top_level_entries,
        })
    }

    fn update_primary_drive(
        &mut self,
        root_cid: &Cid,
        working_dir: Option<&Path>,
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
            let now = unix_now();
            let dck_generation = account
                .app_keys
                .as_ref()
                .map_or(0, |snap| snap.dck_generation);
            updated.device_roots.insert(
                account.device_pubkey.clone(),
                crate::config::DeviceRootRef {
                    root_cid: root_cid.to_string(),
                    published_at: now,
                    dck_generation,
                },
            );
        }

        self.config.upsert_drive(updated);
        self.config.save(config_path_in(&self.config_dir))?;
        Ok(())
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
        let public_root = index_dir_with_history(&public_tree, work.path(), None, unix_now())
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
                crate::config::DeviceRootRef {
                    root_cid: public_root.to_string(),
                    published_at: unix_now(),
                    dck_generation: account.state.app_keys.as_ref().unwrap().dck_generation,
                },
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
}
