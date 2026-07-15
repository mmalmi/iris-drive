#[allow(clippy::wildcard_imports)]
use super::*;
use iris_drive_core::backup_summary::{backup_target_summary, is_default_blossom_server};

pub(crate) fn backup_targets_status(config: &AppConfig) -> Vec<Value> {
    visible_backup_targets(config)
        .iter()
        .map(backup_target_status)
        .collect()
}

pub(crate) fn configured_backup_targets_status(config: &AppConfig) -> Vec<Value> {
    effective_backup_targets(config)
        .iter()
        .map(backup_target_status)
        .collect()
}

fn visible_backup_targets(config: &AppConfig) -> Vec<BackupTarget> {
    let mut targets = effective_backup_targets(config);
    for target in visible_default_blossom_backup_targets(config) {
        if !targets.iter().any(|existing| existing.id == target.id) {
            targets.push(target);
        }
    }
    targets
}

fn visible_default_blossom_backup_targets(config: &AppConfig) -> Vec<BackupTarget> {
    config
        .blossom_servers
        .iter()
        .filter(|server| is_default_blossom_server(server))
        .filter_map(|server| parse_backup_target(server, None).ok())
        .collect()
}

pub(crate) fn backup_target_status(target: &BackupTarget) -> Value {
    let summary = backup_target_summary(target);
    json!({
        "id": summary.id,
        "kind": summary.kind,
        "target": summary.target,
        "label": summary.label,
        "title": summary.title,
        "state": summary.state,
        "detail": summary.detail,
        "enabled": summary.enabled,
        "last_sync": target.last_sync.as_ref().map(backup_target_sync_status),
        "last_check": target.last_check.as_ref().map(backup_target_check_status),
    })
}

fn backup_target_sync_status(sync: &BackupTargetSync) -> Value {
    json!({
        "state": sync.state.as_str(),
        "root_cid": sync.root_cid.as_str(),
        "synced_at": sync.synced_at,
        "total_hashes": sync.total_hashes,
        "uploaded": sync.uploaded,
        "already_present": sync.already_present,
    })
}

fn backup_target_check_status(check: &BackupTargetCheck) -> Value {
    json!({
        "state": check.state.as_str(),
        "root_cid": check.root_cid.as_str(),
        "checked_at": check.checked_at,
        "total_hashes": check.total_hashes,
        "sample_size": check.sample_size,
        "sampled_hashes": check.sampled_hashes,
        "present": check.present,
        "missing": check.missing,
        "unknown": check.unknown,
        "latency_ms": check.latency_ms,
        "download_bytes": check.download_bytes,
        "download_ms": check.download_ms,
        "download_bytes_per_second": check.download_bytes_per_second,
        "error": check.error.as_deref(),
    })
}
