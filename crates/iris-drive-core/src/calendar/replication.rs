use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use hashtree_core::Cid;
use nostr_sdk::{Client, Event};

use super::{CALENDAR_TREE_NAME, local_replica_tree, replica_writer};
use crate::config_lock::ConfigMutationLock;
use crate::paths::{config_path_in, key_path_in};
use crate::{AppConfig, AppKey, Daemon, blossom_sync, blossom_sync_client, relay_sync};

/// Flush the persisted local Calendar writer root during a one-shot native
/// sync. Ordinary Drive-only profiles never open a block store for this step.
pub(crate) async fn publish_pending_replica(
    client: &Client,
    config_dir: &Path,
    timeout: Duration,
) -> Result<()> {
    let config = AppConfig::load_or_default_cached_profile(config_path_in(config_dir))?;
    let Some(state) = config
        .profile
        .as_ref()
        .filter(|state| state.can_write_roots())
    else {
        return Ok(());
    };
    let Some(tree_name) = local_replica_tree(config_dir, state)? else {
        return Ok(());
    };
    let Some(root) = config
        .drive(&tree_name)
        .filter(|drive| drive.root_scope_id == state.root_scope_id())
        .and_then(|drive| drive.app_key_roots.get(&state.app_key_pubkey))
    else {
        return Ok(());
    };
    let receipt = config_dir.join(format!("{tree_name}.published"));
    match std::fs::read_to_string(&receipt) {
        Ok(cid) if cid == root.root_cid => return Ok(()),
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }

    let cid = Cid::parse(&root.root_cid)?;
    let daemon = Daemon::open(config_dir)?;
    let device = AppKey::load(key_path_in(config_dir))?;
    ensure!(
        device.keys().public_key().to_hex() == state.app_key_pubkey,
        "Calendar signer changed"
    );
    let storage = blossom_sync_client(device.keys().clone(), &config.blossom_servers);
    tokio::time::timeout(timeout, async {
        blossom_sync::upload_tree(daemon.tree(), &cid, &storage).await?;
        if !root_is_current(
            config_dir,
            &tree_name,
            &state.app_key_pubkey,
            &root.root_cid,
        )? {
            return Ok::<_, anyhow::Error>(());
        }
        relay_sync::publish_files_root(client, device.keys(), &tree_name, root).await?;
        let _lock = ConfigMutationLock::acquire(config_dir).await?;
        if root_is_current(
            config_dir,
            &tree_name,
            &state.app_key_pubkey,
            &root.root_cid,
        )? {
            crate::atomic_file::atomic_write_private(&receipt, root.root_cid.as_bytes())?;
        }
        Ok(())
    })
    .await
    .context("publishing Calendar changes timed out")??;
    Ok(())
}

fn root_is_current(config_dir: &Path, tree: &str, signer: &str, root: &str) -> Result<bool> {
    let config = AppConfig::load_or_default_cached_profile(config_path_in(config_dir))?;
    let Some(state) = config
        .profile
        .as_ref()
        .filter(|state| state.app_key_pubkey == signer && state.can_write_roots())
    else {
        return Ok(false);
    };
    Ok(config
        .drive(tree)
        .filter(|drive| drive.root_scope_id == state.root_scope_id())
        .and_then(|drive| drive.app_key_roots.get(signer))
        .is_some_and(|latest| latest.root_cid == root))
}

pub(crate) fn apply_discovered_roots(
    config: &mut AppConfig,
    account: &crate::Profile,
    events: Result<Vec<Event>, relay_sync::RelayError>,
) -> bool {
    let mut changed = false;
    match events {
        Ok(events) => {
            for event in events {
                match relay_sync::apply_remote_files_root_event(
                    config,
                    &event,
                    Some(account.app_key.keys()),
                ) {
                    Ok(relay_sync::FilesRootApply::Applied) => changed = true,
                    Ok(_) => {}
                    Err(error) => tracing::warn!(%error, "rejecting Calendar root"),
                }
            }
        }
        Err(error) => tracing::warn!(%error, "fetching Calendar roots"),
    }
    changed
}

pub(crate) async fn append_missing_roots(
    config_dir: &Path,
    config: &AppConfig,
    account: &crate::Profile,
    downloads: &mut Vec<String>,
) -> Result<()> {
    let scope = account.state.root_scope_id();
    let daemon = Daemon::open_with_config(config_dir, config.clone())?;
    for drive in config.drives.iter().filter(|drive| {
        drive.root_scope_id == scope
            && (drive.drive_id == CALENDAR_TREE_NAME || replica_writer(&drive.drive_id).is_some())
    }) {
        let Some(root) = drive.app_key_roots.get(&account.state.app_key_pubkey) else {
            continue;
        };
        let cid = Cid::parse(&root.root_cid)?;
        let filename = if drive.drive_id == CALENDAR_TREE_NAME {
            super::CALENDAR_DATA_FILE
        } else {
            super::replica::CHANGES_FILE
        };
        if !matches!(
            super::replica_store::read_tree_file(daemon.tree(), &cid, filename).await,
            Ok(Some(_))
        ) && !downloads.contains(&root.root_cid)
        {
            downloads.push(root.root_cid.clone());
        }
    }
    Ok(())
}
