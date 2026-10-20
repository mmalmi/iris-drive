use std::sync::Arc;

use super::{AppConfig, Mutex, NATIVE_SYNC_RELAY_TIMEOUT_SECS, Path, config_path_in};

pub(super) fn schedule_drive_root_sync_after_durable_approval(
    config_dir: &Path,
    config_mutation: &Arc<Mutex<()>>,
    relay_client: &nostr_sdk::Client,
) {
    let config_dir = config_dir.to_path_buf();
    let config_mutation = config_mutation.clone();
    let relay_client = relay_client.clone();
    tokio::spawn(async move {
        if let Err(error) = sync_drive_roots(&config_dir, &config_mutation, &relay_client).await {
            tracing::warn!(
                error,
                "syncing drive roots after native device approval failed"
            );
        }
    });
}

async fn sync_drive_roots(
    config_dir: &Path,
    config_mutation: &Mutex<()>,
    relay_client: &nostr_sdk::Client,
) -> Result<(), String> {
    let (root_scope_id, authorized_app_keys) = {
        let _guard = config_mutation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let config = AppConfig::load_or_default(config_path_in(config_dir))
            .map_err(|error| format!("loading approved profile: {error}"))?;
        let state = config
            .profile
            .as_ref()
            .ok_or_else(|| "approved profile disappeared".to_string())?;
        (
            state.root_scope_id(),
            iris_drive_core::authorized_app_key_pubkeys(state),
        )
    };
    let events = iris_drive_core::relay_sync::fetch_drive_roots(
        relay_client,
        &root_scope_id,
        iris_drive_core::PRIMARY_DRIVE_ID,
        &authorized_app_keys,
        std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
    )
    .await
    .map_err(|error| format!("fetching newly linked drive roots: {error}"))?;
    let disk_mutation = iris_drive_core::config_lock::ConfigMutationLock::acquire(config_dir)
        .await
        .map_err(|error| format!("locking newly linked drive roots: {error}"))?;
    let roots = {
        let _guard = config_mutation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut config = AppConfig::load_or_default(config_path_in(config_dir))
            .map_err(|error| format!("reloading approved profile: {error}"))?;
        if config
            .profile
            .as_ref()
            .map(iris_drive_core::ProfileState::root_scope_id)
            != Some(root_scope_id)
        {
            return Err("profile changed while fetching newly linked drive roots".to_string());
        }
        let roots = iris_drive_core::apply_drive_root_events(config_dir, &mut config, &events)
            .map_err(|error| format!("applying newly linked drive roots: {error}"))?;
        if roots.applied > 0 {
            config
                .save(config_path_in(config_dir))
                .map_err(|error| format!("saving newly linked drive roots: {error}"))?;
        }
        roots
    };
    drop(disk_mutation);
    let mut report = iris_drive_core::download_applied_drive_roots(
        config_dir,
        &roots.root_cids_to_download,
        None,
    )
    .await
    .map_err(|error| format!("downloading newly linked drive roots: {error}"))?;
    report.drive_root_events_seen = roots.seen;
    report.drive_root_events_applied = roots.applied;
    report.drive_root_events_skipped = roots.skipped;
    if report.fips_download.is_some() || report.blossom_download.is_some() {
        iris_drive_core::paths::touch_provider_root_signal_in(config_dir)
            .map_err(|error| format!("signaling newly linked drive roots: {error}"))?;
    }
    tracing::debug!(
        drive_root_events_seen = report.drive_root_events_seen,
        drive_root_events_applied = report.drive_root_events_applied,
        fips_download = report.fips_download.is_some(),
        blossom_download = report.blossom_download.is_some(),
        "synced drive roots after durable native device approval"
    );
    Ok(())
}
