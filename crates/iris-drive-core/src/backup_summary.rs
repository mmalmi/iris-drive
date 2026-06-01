use crate::config::{BackupTarget, BackupTargetKind, DEFAULT_BLOSSOM_SERVERS};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackupTargetSummary {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub label: Option<String>,
    pub title: String,
    pub state: String,
    pub detail: String,
    pub enabled: bool,
}

#[must_use]
pub fn backup_target_summary(target: &BackupTarget) -> BackupTargetSummary {
    let label = backup_target_label(target).map(ToOwned::to_owned);
    BackupTargetSummary {
        id: target.id.clone(),
        kind: backup_target_kind_label(target.kind).to_owned(),
        target: target.target.clone(),
        title: backup_target_title(target, label.as_deref()),
        state: backup_target_state(target).to_owned(),
        detail: backup_target_detail(target),
        enabled: target.enabled,
        label,
    }
}

#[must_use]
pub fn blossom_backup_target(server: &str) -> Option<BackupTarget> {
    let target = normalize_blossom_url_for_comparison(server)?;
    Some(BackupTarget {
        id: format!("blossom:{target}"),
        kind: BackupTargetKind::Blossom,
        target,
        label: None,
        enabled: true,
        last_sync: None,
        last_check: None,
    })
}

#[must_use]
pub fn is_default_blossom_server(server: &str) -> bool {
    DEFAULT_BLOSSOM_SERVERS
        .iter()
        .filter_map(|default| normalize_blossom_url_for_comparison(default))
        .any(|default| {
            normalize_blossom_url_for_comparison(server).is_some_and(|server| server == default)
        })
}

#[must_use]
pub fn backup_target_kind_label(kind: BackupTargetKind) -> &'static str {
    match kind {
        BackupTargetKind::Blossom => "blossom",
        BackupTargetKind::Fips => "fips",
        BackupTargetKind::Filesystem => "filesystem",
        BackupTargetKind::Lmdb => "lmdb",
    }
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
            parts.push(format!("{}/s", format_backup_bytes(bytes_per_second)));
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

fn format_backup_bytes(bytes: u64) -> String {
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

fn normalize_blossom_url_for_comparison(value: &str) -> Option<String> {
    let trimmed = value.trim().trim_end_matches('/');
    (trimmed.starts_with("http://") || trimmed.starts_with("https://")).then(|| trimmed.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{BackupTargetCheck, BackupTargetSync};

    #[test]
    fn backup_target_summary_emits_shared_row_fields() {
        let summary = backup_target_summary(&BackupTarget {
            id: "backup-1".to_owned(),
            kind: BackupTargetKind::Blossom,
            target: "https://backup.example".to_owned(),
            label: Some("Archive".to_owned()),
            enabled: true,
            last_sync: Some(BackupTargetSync {
                state: "uploading".to_owned(),
                root_cid: "root".to_owned(),
                synced_at: 1_700_000_000,
                total_hashes: 5,
                uploaded: 2,
                already_present: 1,
            }),
            last_check: Some(BackupTargetCheck {
                state: "verified".to_owned(),
                root_cid: "root".to_owned(),
                checked_at: 1_700_000_100,
                total_hashes: 5,
                sample_size: 5,
                sampled_hashes: 5,
                present: 5,
                missing: 0,
                unknown: 0,
                latency_ms: Some(35),
                download_bytes: Some(2048),
                download_ms: Some(1000),
                download_bytes_per_second: Some(2048),
                error: None,
            }),
        });

        assert_eq!(summary.title, "Archive");
        assert_eq!(summary.state, "uploading");
        assert_eq!(
            summary.detail,
            "https://backup.example | 2/5 | check verified | 35 ms | 2.0 KB/s"
        );
    }

    #[test]
    fn default_blossom_and_fips_targets_get_friendly_summaries() {
        let default = blossom_backup_target(" https://upload.iris.to/ ").unwrap();
        let summary = backup_target_summary(&default);
        assert_eq!(summary.label.as_deref(), Some("Blossom remote"));
        assert_eq!(summary.title, "Blossom remote");
        assert_eq!(summary.state, "ready");

        let fips_summary = backup_target_summary(&BackupTarget {
            id: "fips-1".to_owned(),
            kind: BackupTargetKind::Fips,
            target: "abcdefghijklmnopqrstuvwxyz0123456789".to_owned(),
            label: None,
            enabled: true,
            last_sync: None,
            last_check: None,
        });

        assert_eq!(fips_summary.title, "abcdefghijklmn...0123456789");
        assert_eq!(fips_summary.state, "pending");
        assert_eq!(fips_summary.detail, "abcdefghijklmn...0123456789");
    }
}
