//! Durable root handoff for newly approved `AppKey` installs.
//!
//! Approval metadata must not arrive before the newly authorized install can
//! decrypt and fetch the current Drive. This module prepares one publishable,
//! causally complete root and uploads all of its live blocks before callers
//! announce the root envelope and, finally, the approval receipt.

use std::path::Path;

use anyhow::{Context, Result};
use hashtree_core::Cid;

use crate::{AppConfig, AppKey, AppKeyRootRef, Daemon, PRIMARY_DRIVE_ID};

#[derive(Debug, Clone)]
pub struct DeviceApprovalRootHandoff {
    pub root_scope_id: String,
    pub drive_id: String,
    pub root: AppKeyRootRef,
    pub authorized_app_keys: Vec<String>,
    pub blossom_upload: crate::blossom_sync::UploadReport,
}

struct DeviceApprovalRootSnapshot {
    root_scope_id: String,
    drive_id: String,
    root: AppKeyRootRef,
    authorized_app_keys: Vec<String>,
    blossom_servers: Vec<String>,
}

/// Build and upload the root that must be published before an approval
/// receipt. An otherwise-empty profile gets an explicit encrypted empty root,
/// so successful approval always leaves the new `AppKey` with a resolvable
/// `main` tree.
pub async fn prepare_device_approval_root_handoff(
    config_dir: &Path,
) -> Result<DeviceApprovalRootHandoff> {
    let snapshot = prepare_device_approval_root_snapshot(config_dir).await?;
    let root_cid = Cid::parse(&snapshot.root.root_cid)
        .with_context(|| format!("parsing device approval root {}", snapshot.root.root_cid))?;
    let app_key = AppKey::load(crate::paths::key_path_in(config_dir))
        .context("loading AppKey for device approval")?;
    let blossom = crate::blossom_sync_client(app_key.keys().clone(), &snapshot.blossom_servers);
    let daemon = Daemon::open(config_dir).context("opening approval block store")?;
    let blossom_upload = crate::blossom_sync::upload_tree(daemon.tree(), &root_cid, &blossom)
        .await
        .context("uploading device approval Drive root to Blossom")?;

    Ok(DeviceApprovalRootHandoff {
        root_scope_id: snapshot.root_scope_id,
        drive_id: snapshot.drive_id,
        root: snapshot.root,
        authorized_app_keys: snapshot.authorized_app_keys,
        blossom_upload,
    })
}

async fn prepare_device_approval_root_snapshot(
    config_dir: &Path,
) -> Result<DeviceApprovalRootSnapshot> {
    // FileProvider can persist a new root from another process. Serialize the
    // causal snapshot with that local transaction, then release the lock before
    // the Blossom upload performed by the public handoff function.
    let _config_mutation = crate::config_lock::ConfigMutationLock::acquire(config_dir)
        .await
        .context("locking Drive for device approval")?;
    let mut daemon = Daemon::open(config_dir).context("opening Drive for device approval")?;
    daemon
        .materialize_primary_merged_root_for_device_approval()
        .await
        .context("materializing current Drive for device approval")?;
    drop(daemon);

    let config = AppConfig::load_or_default(crate::paths::config_path_in(config_dir))
        .context("reloading materialized approval root")?;
    let state = config
        .profile
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("profile disappeared while preparing device approval"))?;
    let drive = config
        .drive(PRIMARY_DRIVE_ID)
        .ok_or_else(|| anyhow::anyhow!("primary Drive disappeared during device approval"))?;
    let root = drive
        .app_key_roots
        .get(&state.app_key_pubkey)
        .cloned()
        .ok_or_else(|| {
            anyhow::anyhow!("approving AppKey has no materialized primary Drive root")
        })?;
    if root.local_only {
        anyhow::bail!("device approval root is still local-only");
    }
    if config.blossom_servers.is_empty() {
        anyhow::bail!(
            "cannot publish device approval before Drive blocks are available: no Blossom servers configured"
        );
    }

    let authorized_app_keys = crate::drive_root_recipient_app_key_pubkeys(state, drive);
    if authorized_app_keys.is_empty() {
        anyhow::bail!("device approval Drive root has no authorized recipients");
    }

    Ok(DeviceApprovalRootSnapshot {
        root_scope_id: state.root_scope_id(),
        drive_id: drive.drive_id.clone(),
        root,
        authorized_app_keys,
        blossom_servers: config.blossom_servers.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::prepare_device_approval_root_snapshot;
    use std::future::Future;
    use std::task::{Context, Poll, Waker};

    #[tokio::test]
    async fn approval_snapshot_waits_for_file_provider_config_transaction() {
        let dir = tempfile::tempdir().unwrap();
        let profile = crate::Profile::create(dir.path(), Some("owner".to_owned())).unwrap();
        let mut config = crate::AppConfig {
            profile: Some(profile.state.clone()),
            ..crate::AppConfig::default()
        };
        config.upsert_drive(crate::Drive::primary(profile.state.root_scope_id()));
        config
            .save(crate::paths::config_path_in(dir.path()))
            .unwrap();
        let file_provider_transaction = crate::config_lock::ConfigMutationLock::acquire(dir.path())
            .await
            .unwrap();
        let mut snapshot = Box::pin(prepare_device_approval_root_snapshot(dir.path()));

        let first_poll = Future::poll(snapshot.as_mut(), &mut Context::from_waker(Waker::noop()));
        assert!(
            matches!(first_poll, Poll::Pending),
            "approval snapshot ignored the FileProvider config transaction"
        );
        drop(file_provider_transaction);

        let snapshot = snapshot.await.unwrap();
        assert!(!snapshot.root.root_cid.is_empty());
        assert_eq!(snapshot.drive_id, crate::PRIMARY_DRIVE_ID);
    }
}
