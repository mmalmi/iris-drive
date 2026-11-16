//! A local Drive commit must survive the app or share sheet exiting mid-upload.
use anyhow::{Context, Result, bail, ensure};
use iris_drive_core::{Daemon, config_lock::ConfigMutationLock};
use serde_json::{Value, json};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};
pub(super) enum UploadState {
    Complete,
    Pending,
    Paused,
}

pub(super) enum PendingUpload {
    Photos,
    SharedFiles,
}

impl PendingUpload {
    pub(super) fn queue(&self, dir: &Path, profile: &str, generation: &[u8]) -> Result<()> {
        iris_drive_core::atomic_write(&self.path(dir, profile), generation)?;
        Ok(())
    }

    fn path(&self, dir: &Path, profile: &str) -> PathBuf {
        let id = hashtree_core::to_hex(&hashtree_core::sha256(profile.as_bytes()));
        let prefix = match self {
            Self::Photos => "photo-backup",
            Self::SharedFiles => "shared-files",
        };
        dir.join(format!("{prefix}-{id}.pending"))
    }

    pub(super) fn publish_json(
        &self,
        dir: &str,
        profile: Option<&str>,
        allow_network: bool,
    ) -> Value {
        let result = super::native_provider_runtime().and_then(|runtime| {
            runtime.block_on(self.publish(Path::new(dir), profile, allow_network))
        });
        result_json(result)
    }

    pub(super) async fn publish(
        &self,
        dir: &Path,
        profile: Option<&str>,
        allow_network: bool,
    ) -> Result<UploadState> {
        let lock = ConfigMutationLock::acquire(dir).await?;
        let daemon = Daemon::open(dir)?;
        let Some(state) = daemon
            .config()
            .profile
            .as_ref()
            .filter(|state| state.is_authorized())
        else {
            if matches!(self, Self::SharedFiles) {
                return Ok(UploadState::Complete);
            }
            bail!("Device is not authorized");
        };
        let current = state.profile_id.to_string();
        if let Some(expected) = profile {
            iris_drive_core::photo_backup::check_profile(daemon.config(), expected)?;
        }
        let path = self.path(dir, &current);
        let generation = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(UploadState::Complete),
            Err(error) => return Err(error.into()),
        };
        if matches!(self, Self::SharedFiles) && !daemon.config().sync_enabled {
            return Ok(UploadState::Paused);
        }
        if !allow_network {
            return Ok(UploadState::Pending);
        }
        let prepared = match super::prepare_current_app_key_root_publish(&daemon).await? {
            super::PreparedProviderRootPublish::Ready(prepared) => prepared,
            super::PreparedProviderRootPublish::Unavailable(reason) => {
                bail!(
                    "{}",
                    reason["error"]
                        .as_str()
                        .unwrap_or("Drive is waiting to sync")
                )
            }
        };
        drop(lock);
        // Large videos legitimately take longer than the foreground File Provider deadline.
        // Each Blossom transfer uses the client's request timeout. Never clear the queue
        // until the blocks and the signed root have both been accepted.
        let result = super::publish_prepared_app_key_root(dir, *prepared).await?;
        ensure!(
            result["published_drive_root"] == true,
            "Drive is waiting to sync"
        );
        let _lock = ConfigMutationLock::acquire(dir).await?;
        let current = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(UploadState::Complete),
            Err(error) => return Err(error.into()),
        };
        if current != generation {
            return Ok(UploadState::Pending);
        }
        fs::remove_file(path).context("finishing Drive upload")?;
        Ok(UploadState::Complete)
    }
}

pub(super) fn result_json(result: Result<UploadState>) -> Value {
    match result {
        Ok(UploadState::Complete) => json!({"ok": true, "pending": false}),
        Ok(UploadState::Pending) => json!({"ok": false, "pending": true}),
        Ok(UploadState::Paused) => json!({"ok": false, "pending": true, "paused": true}),
        Err(error) => json!({"ok": false, "pending": true, "error": format!("{error:#}")}),
    }
}
