#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn cmd_stats(config_dir: &std::path::Path) -> Result<()> {
    let initialized = already_initialized(config_dir);
    let config = AppConfig::load_or_default(config_path_in(config_dir))
        .with_context(|| format!("reading config at {}", config_path_in(config_dir).display()))?;
    let daemon_status = load_daemon_status(config_dir);
    let blocks_dir = config_dir.join("blocks");
    let block_stats =
        collect_file_stats_with_entry_limit(&blocks_dir, Some(STATUS_BLOCK_STATS_ENTRY_LIMIT))
            .with_context(|| format!("reading block store stats at {}", blocks_dir.display()))?;
    let current_root_cid = current_primary_root_cid(&config);
    let snapshot_url = current_root_cid
        .as_deref()
        .and_then(drive_iris_to_snapshot_url_for_root);
    let drive_iris_to_url = current_root_cid
        .as_ref()
        .and_then(|_| drive_iris_to_url_for_primary_drive(&config));
    let root_file_stats = current_root_cid
        .as_deref()
        .and_then(|root| root_file_stats(config_dir, root));
    let merged_stats = primary_drive_stats(config_dir, &config);
    let files = merged_stats
        .as_ref()
        .map(|stats| stats.file_count)
        .or_else(|| root_file_stats.as_ref().map(|stats| stats.file_count))
        .unwrap_or(0);
    let top_level_entries = merged_stats
        .as_ref()
        .map(|stats| stats.top_level_entries)
        .or_else(|| {
            current_root_cid
                .as_deref()
                .and_then(|root| root_top_level_entries(config_dir, root))
        })
        .unwrap_or(0);
    let visible_file_bytes = merged_stats
        .as_ref()
        .map(|stats| stats.visible_file_bytes)
        .or_else(|| {
            root_file_stats
                .as_ref()
                .map(|stats| stats.visible_file_bytes)
        })
        .unwrap_or(0);
    let authorized_app_keys = config
        .profile
        .as_ref()
        .and_then(|state| state.app_keys.as_ref())
        .map_or(0, |snap| snap.app_actors.len());
    let published_app_key_roots = config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .map_or(0, |drive| drive.app_key_roots.len());
    let unresolved_conflicts = current_root_cid
        .as_deref()
        .and_then(|root| root_conflict_status(config_dir, root))
        .and_then(|status| status.get("unresolved_count").and_then(Value::as_u64))
        .unwrap_or(0);
    let daemon_running = daemon_status
        .as_ref()
        .and_then(|status| status.get("running"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let daemon_fresh = daemon_status
        .as_ref()
        .and_then(|status| status.get("fresh"))
        .and_then(Value::as_bool)
        .unwrap_or(false);

    println!(
        "{}",
        json!({
            "initialized": initialized,
            "files": files,
            "top_level_entries": top_level_entries,
            "visible_file_bytes": visible_file_bytes,
            "local_block_count": block_stats.file_count,
            "local_block_bytes": block_stats.total_bytes,
            "authorized_app_keys": authorized_app_keys,
            "published_app_key_roots": published_app_key_roots,
            "backup_targets": effective_backup_targets(&config).len(),
            "unresolved_conflicts": unresolved_conflicts,
            "daemon_running": daemon_running,
            "daemon_fresh": daemon_fresh,
            "snapshot_url": snapshot_url,
            "drive_url": drive_iris_to_url,
        })
    );
    Ok(())
}
