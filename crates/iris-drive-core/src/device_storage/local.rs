//! Local storage can be measured from an already-open provider root even when
//! the network service is stopped or runs an older version.

use anyhow::{Context, Result};
use hashtree_core::Cid;

use super::measure::{HoldingCache, measure_holding, root_hex};
use super::report::{DeviceStorageFiles, DeviceStorageReport, current_app};
use super::reporter::{FOLDER_MAX_AGE_MS, seen_versions};
use super::state::{DeviceStorageState, profile_devices, update_device_storage_state};
use super::{disk, unix_now};
use crate::{Daemon, ProfileState};

pub async fn refresh_local_report(daemon: &Daemon, root: &Cid) -> Result<()> {
    let Some(profile) = daemon
        .config()
        .profile
        .as_ref()
        .filter(|profile| profile.is_authorized())
    else {
        return Ok(());
    };
    let dir = daemon.config_dir();
    let scope = profile.root_scope_id();
    let known = DeviceStorageState::load(dir, &scope).own;
    if let Some(report) = &known {
        let age = unix_now().saturating_sub(report.measured_at);
        if age < 3600 || (report.root == root_hex(root) && age < FOLDER_MAX_AGE_MS / 1000) {
            return Ok(());
        }
    }
    let devices = profile_devices(daemon.config(), profile);
    let report = measure_root(
        daemon,
        root,
        profile,
        &devices,
        &mut HoldingCache::default(),
    )
    .await?;
    // A refresh can finish after a roster mutation or logout. Only save for
    // the same profile and only if a concurrent measurement hasn't won.
    let _lock = crate::config_lock::ConfigMutationLock::acquire(dir).await?;
    let current =
        crate::AppConfig::load_or_default_cached_profile(crate::paths::config_path_in(dir))?;
    if current.profile.as_ref().is_some_and(|p| {
        p.profile_id == profile.profile_id
            && p.app_key_pubkey == profile.app_key_pubkey
            && p.is_authorized()
    }) {
        update_device_storage_state(dir, &scope, |stored| {
            if stored.own == known {
                stored.own = Some(report);
            }
        })?;
    }
    Ok(())
}

pub(super) async fn measure_root(
    daemon: &Daemon,
    root: &Cid,
    state: &ProfileState,
    devices: &[String],
    cache: &mut HoldingCache,
) -> Result<DeviceStorageReport> {
    let now_ms = unix_now().saturating_mul(1000);
    let holding =
        measure_holding(daemon.tree(), root, cache, Some(FOLDER_MAX_AGE_MS), now_ms).await;
    let blocks_dir = daemon.blocks_dir().to_path_buf();
    let (stored_bytes, free) = tokio::task::spawn_blocking(move || {
        (
            disk::stored_bytes(&blocks_dir),
            disk::free_bytes(&blocks_dir),
        )
    })
    .await
    .context("counting local storage")?;
    Ok(DeviceStorageReport {
        v: 1,
        measured_at: unix_now(),
        app: current_app().to_owned(),
        root: root_hex(root),
        files: holding.total.files,
        file_bytes: holding.total.bytes,
        held_files: holding.total.held_files,
        held_bytes: holding.total.held_bytes,
        stored_bytes,
        capacity_bytes: free.map(|free| stored_bytes.saturating_add(free)),
        unread_folders: (holding.unread_folders > 0).then_some(holding.unread_folders),
        seen: Some(seen_versions(daemon.config(), state, devices)),
        loose: Some(DeviceStorageFiles {
            files: holding.loose.files,
            bytes: holding.loose.bytes,
            held: holding.loose.held_bytes,
        }),
        folders: Some(holding.folders),
    })
}
