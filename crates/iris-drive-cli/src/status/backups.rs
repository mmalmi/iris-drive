#[allow(clippy::wildcard_imports)]
use super::*;

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
    let label = backup_target_label(target);
    json!({
        "id": target.id.as_str(),
        "kind": backup_target_kind_label(target.kind),
        "target": target.target.as_str(),
        "label": label,
        "title": backup_target_title(target, label),
        "state": backup_target_state(target),
        "detail": backup_target_detail(target),
        "enabled": target.enabled,
        "last_sync": target.last_sync.as_ref().map(backup_target_sync_status),
        "last_check": target.last_check.as_ref().map(backup_target_check_status),
    })
}

fn backup_target_label(target: &BackupTarget) -> Option<&str> {
    target
        .label
        .as_deref()
        .filter(|label| !label.trim().is_empty())
        .or_else(|| {
            (target.kind == BackupTargetKind::Blossom && is_default_blossom_server(&target.target))
                .then_some("Blossom remote")
        })
}

fn backup_target_title(target: &BackupTarget, label: Option<&str>) -> String {
    label
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| backup_target_display_target(target))
}

fn backup_target_state(target: &BackupTarget) -> &str {
    target
        .last_sync
        .as_ref()
        .map(|sync| sync.state.as_str())
        .unwrap_or(match target.kind {
            BackupTargetKind::Fips => "pending",
            BackupTargetKind::Blossom | BackupTargetKind::Filesystem | BackupTargetKind::Lmdb => {
                "ready"
            }
        })
}

fn backup_target_detail(target: &BackupTarget) -> String {
    let mut parts = vec![backup_target_display_target(target)];
    if let Some(sync) = target.last_sync.as_ref() {
        parts.push(format!("{}/{}", sync.uploaded, sync.total_hashes));
    }
    if let Some(check) = target.last_check.as_ref() {
        if !check.state.trim().is_empty() {
            parts.push(format!("check {}", check.state));
        }
        if let Some(latency_ms) = check.latency_ms {
            parts.push(format!("{latency_ms} ms"));
        }
        if let Some(bytes_per_second) = check.download_bytes_per_second {
            parts.push(format!(
                "{}/s",
                backup_target_format_bytes(bytes_per_second)
            ));
        }
    }
    parts.join(" | ")
}

fn backup_target_display_target(target: &BackupTarget) -> String {
    if target.kind == BackupTargetKind::Fips {
        short_status_value(&target.target)
    } else {
        target.target.clone()
    }
}

fn short_status_value(value: &str) -> String {
    if value.chars().count() <= 32 {
        return value.to_owned();
    }
    let start = value.chars().take(14).collect::<String>();
    let end = value
        .chars()
        .rev()
        .take(10)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{start}...{end}")
}

fn backup_target_format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
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

pub(crate) fn backup_target_kind_label(kind: BackupTargetKind) -> &'static str {
    match kind {
        BackupTargetKind::Blossom => "blossom",
        BackupTargetKind::Fips => "fips",
        BackupTargetKind::Filesystem => "filesystem",
        BackupTargetKind::Lmdb => "lmdb",
    }
}
