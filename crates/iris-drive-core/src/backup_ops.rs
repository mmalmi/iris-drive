use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use hashtree_core::{Cid, HashTree, Store, diff::collect_hashes, to_hex};
use hashtree_fs::FsBlobStore;
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::{FromBech32, ToBech32};
use serde::{Deserialize, Serialize};

use crate::backup_summary::backup_target_kind_label;
use crate::blossom_sync::{BackupCheckReport, UploadReport};
use crate::config::{
    AppConfig, BackupTarget, BackupTargetCheck, BackupTargetKind, BackupTargetSync,
};
use crate::daemon::Daemon;
use crate::paths::{config_path_in, key_path_in};

const DEFAULT_BACKUP_CHECK_SAMPLE_SIZE: usize = 16;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupOperationReport {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub label: Option<String>,
    pub state: String,
    pub root_cid: String,
    pub upload: Option<BackupUploadSummary>,
    pub check: Option<BackupCheckSummary>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupUploadSummary {
    pub total_hashes: usize,
    pub uploaded: usize,
    pub already_present: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackupCheckSummary {
    pub total_hashes: usize,
    pub sample_size: usize,
    pub sampled_hashes: usize,
    pub present: usize,
    pub missing: usize,
    pub unknown: usize,
    pub latency_ms: Option<u64>,
    pub download_bytes: Option<usize>,
    pub download_ms: Option<u64>,
    pub download_bytes_per_second: Option<u64>,
    pub error: Option<String>,
}

pub fn add_blossom_server(config_dir: &Path, url: &str) -> Result<()> {
    let target = parse_backup_target(url, None)?;
    ensure_blossom_target(&target)?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    upsert_blossom_target(&mut config, target);
    config.save(config_path_in(config_dir))?;
    Ok(())
}

pub fn remove_blossom_server(config_dir: &Path, url: &str) -> Result<()> {
    let target = parse_backup_target(url, None)?;
    ensure_blossom_target(&target)?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    config
        .blossom_servers
        .retain(|server| server != &target.target);
    config.remove_backup_target(&target.id);
    config.save(config_path_in(config_dir))?;
    Ok(())
}

pub fn add_backup_target(config_dir: &Path, target: &str, label: Option<String>) -> Result<()> {
    let target = parse_backup_target(target, label)?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    if target.kind == BackupTargetKind::Blossom {
        upsert_blossom_target(&mut config, target);
    } else {
        config.upsert_backup_target(target);
    }
    config.save(config_path_in(config_dir))?;
    Ok(())
}

pub fn remove_backup_target(config_dir: &Path, target: &str) -> Result<()> {
    let target = parse_backup_target(target, None)?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    if target.kind == BackupTargetKind::Blossom {
        config
            .blossom_servers
            .retain(|server| server != &target.target);
    }
    config.remove_backup_target(&target.id);
    config.save(config_path_in(config_dir))?;
    Ok(())
}

pub async fn sync_backups(
    config_dir: &Path,
    target: Option<&str>,
) -> Result<Vec<BackupOperationReport>> {
    let target_id = target
        .map(|target| parse_backup_target(target, None).map(|target| target.id))
        .transpose()
        .context("parsing backup target")?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    ensure_configured_blossom_backup_targets(&mut config);
    let root_cid_str = current_primary_root_cid(&config)
        .ok_or_else(|| anyhow::anyhow!("no current drive root; import files first"))?;
    let root_cid = Cid::parse(&root_cid_str).context("parsing current root cid")?;
    let device =
        crate::DeviceIdentity::load(key_path_in(config_dir)).context("loading device key")?;
    let daemon = Daemon::open(config_dir).context("opening daemon for backup upload")?;
    let mut reports = Vec::new();

    for index in 0..config.backup_targets.len() {
        let target = config.backup_targets[index].clone();
        if !target.enabled {
            continue;
        }
        if let Some(target_id) = target_id.as_deref()
            && target.id != target_id
        {
            continue;
        }

        match target.kind {
            BackupTargetKind::Blossom => {
                let servers = vec![target.target.clone()];
                let client = crate::blossom_sync_client(device.keys().clone(), &servers);
                match crate::blossom_sync::upload_tree(daemon.tree(), &root_cid, &client).await {
                    Ok(upload) => {
                        config.backup_targets[index].last_sync = Some(BackupTargetSync {
                            state: "synced".to_string(),
                            root_cid: root_cid_str.clone(),
                            synced_at: unix_now(),
                            total_hashes: upload.total_hashes,
                            uploaded: upload.uploaded,
                            already_present: upload.already_present,
                        });
                        reports.push(sync_report(&target, "synced", &root_cid_str, upload));
                    }
                    Err(error) => reports.push(error_report(
                        &target,
                        "error",
                        &root_cid_str,
                        error.to_string(),
                    )),
                }
            }
            BackupTargetKind::Filesystem => {
                match upload_tree_to_filesystem_replica(
                    daemon.tree(),
                    &root_cid,
                    Path::new(&target.target),
                )
                .await
                {
                    Ok(upload) => {
                        config.backup_targets[index].last_sync = Some(BackupTargetSync {
                            state: "synced".to_string(),
                            root_cid: root_cid_str.clone(),
                            synced_at: unix_now(),
                            total_hashes: upload.total_hashes,
                            uploaded: upload.uploaded,
                            already_present: upload.already_present,
                        });
                        reports.push(sync_report(&target, "synced", &root_cid_str, upload));
                    }
                    Err(error) => reports.push(error_report(
                        &target,
                        "error",
                        &root_cid_str,
                        error.to_string(),
                    )),
                }
            }
            BackupTargetKind::Fips | BackupTargetKind::Lmdb => reports.push(error_report(
                &target,
                "pending",
                &root_cid_str,
                format!(
                    "{} backup transport pending",
                    backup_target_kind_label(target.kind)
                ),
            )),
        }
    }

    config.save(config_path_in(config_dir))?;
    Ok(reports)
}

pub async fn check_backups(
    config_dir: &Path,
    target: Option<&str>,
    sample_size: usize,
) -> Result<Vec<BackupOperationReport>> {
    let target_id = target
        .map(|target| parse_backup_target(target, None).map(|target| target.id))
        .transpose()
        .context("parsing backup target")?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    ensure_configured_blossom_backup_targets(&mut config);
    let root_cid_str = current_primary_root_cid(&config)
        .ok_or_else(|| anyhow::anyhow!("no current drive root; import files first"))?;
    let root_cid = Cid::parse(&root_cid_str).context("parsing current root cid")?;
    let device =
        crate::DeviceIdentity::load(key_path_in(config_dir)).context("loading device key")?;
    let daemon = Daemon::open(config_dir).context("opening daemon for backup check")?;
    let mut reports = Vec::new();
    let sample_size = sample_size.max(1);

    for index in 0..config.backup_targets.len() {
        let target = config.backup_targets[index].clone();
        if !target.enabled {
            continue;
        }
        if let Some(target_id) = target_id.as_deref()
            && target.id != target_id
        {
            continue;
        }

        match target.kind {
            BackupTargetKind::Blossom => {
                let servers = vec![target.target.clone()];
                let client = crate::blossom_sync_client(device.keys().clone(), &servers)
                    .with_timeout(std::time::Duration::from_secs(5));
                match crate::blossom_sync::check_tree_on_server(
                    daemon.tree(),
                    &root_cid,
                    &client,
                    &target.target,
                    sample_size,
                )
                .await
                {
                    Ok(check) => {
                        let state = check.state().to_string();
                        config.backup_targets[index].last_check = Some(
                            backup_target_check_from_report(&check, &state, &root_cid_str, None),
                        );
                        reports.push(check_report(&target, &state, &root_cid_str, &check));
                    }
                    Err(error) => {
                        let error = error.to_string();
                        config.backup_targets[index].last_check = Some(error_backup_target_check(
                            &root_cid_str,
                            sample_size,
                            error.clone(),
                        ));
                        reports.push(error_report(&target, "error", &root_cid_str, error));
                    }
                }
            }
            BackupTargetKind::Fips | BackupTargetKind::Filesystem | BackupTargetKind::Lmdb => {
                reports.push(error_report(
                    &target,
                    "pending",
                    &root_cid_str,
                    format!(
                        "{} backup checks pending",
                        backup_target_kind_label(target.kind)
                    ),
                ));
            }
        }
    }

    config.save(config_path_in(config_dir))?;
    Ok(reports)
}

pub fn ensure_configured_blossom_backup_targets(config: &mut AppConfig) -> bool {
    let mut changed = false;
    for target in configured_blossom_backup_targets(config) {
        if !config
            .backup_targets
            .iter()
            .any(|existing| existing.id == target.id)
        {
            config.upsert_backup_target(target);
            changed = true;
        }
    }
    changed
}

#[must_use]
pub fn effective_backup_targets(config: &AppConfig) -> Vec<BackupTarget> {
    let mut targets = config.backup_targets.clone();
    for target in configured_blossom_backup_targets(config) {
        if !targets.iter().any(|existing| existing.id == target.id) {
            targets.push(target);
        }
    }
    targets
}

#[must_use]
pub fn current_primary_root_cid(config: &AppConfig) -> Option<String> {
    config
        .account
        .as_ref()
        .and_then(|state| {
            config
                .drive(crate::PRIMARY_DRIVE_ID)
                .and_then(|drive| drive.device_roots.get(&state.device_pubkey))
                .map(|root| root.root_cid.clone())
        })
        .or_else(|| {
            config
                .drive(crate::PRIMARY_DRIVE_ID)
                .and_then(|drive| drive.last_root_cid.clone())
        })
}

pub fn parse_backup_target(input: &str, label: Option<String>) -> Result<BackupTarget> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("backup target is required"));
    }

    let (kind_hint, value) = if let Some(rest) = trimmed.strip_prefix("blossom:") {
        (Some(BackupTargetKind::Blossom), rest)
    } else if let Some(rest) = trimmed.strip_prefix("fips://") {
        (Some(BackupTargetKind::Fips), rest)
    } else if let Some(rest) = trimmed.strip_prefix("fips:") {
        (Some(BackupTargetKind::Fips), rest)
    } else if let Some(rest) = trimmed.strip_prefix("filesystem://") {
        (Some(BackupTargetKind::Filesystem), rest)
    } else if let Some(rest) = trimmed.strip_prefix("filesystem:") {
        (Some(BackupTargetKind::Filesystem), rest)
    } else if let Some(rest) = trimmed.strip_prefix("file://") {
        (Some(BackupTargetKind::Filesystem), rest)
    } else if let Some(rest) = trimmed.strip_prefix("fs://") {
        (Some(BackupTargetKind::Filesystem), rest)
    } else if let Some(rest) = trimmed.strip_prefix("fs:") {
        (Some(BackupTargetKind::Filesystem), rest)
    } else if let Some(rest) = trimmed.strip_prefix("lmdb://") {
        (Some(BackupTargetKind::Lmdb), rest)
    } else if let Some(rest) = trimmed.strip_prefix("lmdb:") {
        (Some(BackupTargetKind::Lmdb), rest)
    } else {
        (None, trimmed)
    };

    let target_label = label
        .map(|label| label.trim().to_string())
        .filter(|label| !label.is_empty());
    let kind = kind_hint.unwrap_or_else(|| {
        if value.starts_with("http://") || value.starts_with("https://") {
            BackupTargetKind::Blossom
        } else if looks_like_local_backup_path(value) {
            BackupTargetKind::Filesystem
        } else {
            BackupTargetKind::Fips
        }
    });

    match kind {
        BackupTargetKind::Blossom => {
            let target = normalize_blossom_url(value)?;
            Ok(BackupTarget {
                id: format!("blossom:{target}"),
                kind: BackupTargetKind::Blossom,
                target,
                label: target_label,
                enabled: true,
                last_sync: None,
                last_check: None,
            })
        }
        BackupTargetKind::Fips => {
            let hex = normalize_pubkey(value)?;
            let target = account_npub(&hex);
            Ok(BackupTarget {
                id: format!("fips:{target}"),
                kind: BackupTargetKind::Fips,
                target,
                label: target_label,
                enabled: true,
                last_sync: None,
                last_check: None,
            })
        }
        BackupTargetKind::Filesystem => {
            let target = normalize_local_backup_path(value)?;
            Ok(BackupTarget {
                id: format!("filesystem:{target}"),
                kind: BackupTargetKind::Filesystem,
                target,
                label: target_label,
                enabled: true,
                last_sync: None,
                last_check: None,
            })
        }
        BackupTargetKind::Lmdb => {
            let target = normalize_local_backup_path(value)?;
            Ok(BackupTarget {
                id: format!("lmdb:{target}"),
                kind: BackupTargetKind::Lmdb,
                target,
                label: target_label,
                enabled: true,
                last_sync: None,
                last_check: None,
            })
        }
    }
}

pub fn normalize_blossom_url(value: &str) -> Result<String> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Ok(trimmed.to_string())
    } else {
        Err(anyhow::anyhow!(
            "expected Blossom target URL starting with http:// or https://"
        ))
    }
}

#[must_use]
pub fn default_backup_check_sample_size() -> usize {
    DEFAULT_BACKUP_CHECK_SAMPLE_SIZE
}

async fn upload_tree_to_filesystem_replica<S>(
    tree: &HashTree<S>,
    root: &Cid,
    path: &Path,
) -> Result<UploadReport>
where
    S: Store + Send + Sync + 'static,
{
    let replica = FsBlobStore::new(path)
        .with_context(|| format!("opening filesystem backup target at {}", path.display()))?;
    upload_tree_to_replica(tree, root, &replica).await
}

async fn upload_tree_to_replica<S, R>(
    tree: &HashTree<S>,
    root: &Cid,
    replica: &R,
) -> Result<UploadReport>
where
    S: Store + Send + Sync + 'static,
    R: Store + Send + Sync,
{
    let hashes = collect_hashes(tree, root, 4)
        .await
        .context("collecting encrypted backup blocks")?;
    let mut report = UploadReport {
        total_hashes: hashes.len(),
        ..Default::default()
    };
    let local = tree.get_store();

    for hash in hashes {
        if replica
            .has(&hash)
            .await
            .with_context(|| format!("checking backup blob {}", to_hex(&hash)))?
        {
            report.already_present += 1;
            continue;
        }

        let bytes = local
            .get(&hash)
            .await
            .with_context(|| format!("reading local backup blob {}", to_hex(&hash)))?
            .ok_or_else(|| anyhow::anyhow!("missing local backup blob {}", to_hex(&hash)))?;
        if replica
            .put(hash, bytes)
            .await
            .with_context(|| format!("writing backup blob {}", to_hex(&hash)))?
        {
            report.uploaded += 1;
        } else {
            report.already_present += 1;
        }
    }

    Ok(report)
}

fn configured_blossom_backup_targets(config: &AppConfig) -> Vec<BackupTarget> {
    config
        .blossom_servers
        .iter()
        .filter_map(|server| parse_backup_target(server, None).ok())
        .collect()
}

fn upsert_blossom_target(config: &mut AppConfig, target: BackupTarget) {
    if !config.blossom_servers.contains(&target.target) {
        config.blossom_servers.push(target.target.clone());
    }
    config.upsert_backup_target(target);
}

fn ensure_blossom_target(target: &BackupTarget) -> Result<()> {
    if target.kind == BackupTargetKind::Blossom {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "expected Blossom target URL starting with http:// or https://"
        ))
    }
}

fn looks_like_local_backup_path(value: &str) -> bool {
    let value = value.trim();
    value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with("\\\\")
        || value.as_bytes().get(1).is_some_and(|byte| *byte == b':')
}

fn normalize_local_backup_path(value: &str) -> Result<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("backup path is required"));
    }
    let trimmed = if cfg!(windows)
        && trimmed.len() > 3
        && trimmed.as_bytes()[0] == b'/'
        && trimmed.as_bytes()[2] == b':'
    {
        &trimmed[1..]
    } else {
        trimmed
    };

    let path = if let Some(rest) = trimmed.strip_prefix("~/") {
        dirs::home_dir()
            .map(|home| home.join(rest))
            .ok_or_else(|| anyhow::anyhow!("home directory is not available"))?
    } else {
        PathBuf::from(trimmed)
    };
    if path.as_os_str().is_empty() {
        return Err(anyhow::anyhow!("backup path is required"));
    }
    Ok(path.to_string_lossy().to_string())
}

fn normalize_pubkey(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("public key is required"));
    }
    if trimmed.starts_with("npub1") {
        return PublicKey::from_bech32(trimmed)
            .map(|pubkey| pubkey.to_hex())
            .map_err(|error| anyhow::anyhow!("parsing npub: {error}"));
    }
    if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(trimmed.to_owned());
    }
    Err(anyhow::anyhow!(
        "expected npub1... or 64-char hex pubkey, got {trimmed}"
    ))
}

fn account_npub(hex: &str) -> String {
    PublicKey::from_hex(hex)
        .ok()
        .and_then(|pubkey| pubkey.to_bech32().ok())
        .unwrap_or_else(|| hex.to_owned())
}

fn sync_report(
    target: &BackupTarget,
    state: &str,
    root_cid: &str,
    upload: UploadReport,
) -> BackupOperationReport {
    BackupOperationReport {
        id: target.id.clone(),
        kind: backup_target_kind_label(target.kind).to_owned(),
        target: target.target.clone(),
        label: target.label.clone(),
        state: state.to_owned(),
        root_cid: root_cid.to_owned(),
        upload: Some(upload_summary(&upload)),
        check: None,
        error: None,
    }
}

fn check_report(
    target: &BackupTarget,
    state: &str,
    root_cid: &str,
    check: &BackupCheckReport,
) -> BackupOperationReport {
    BackupOperationReport {
        id: target.id.clone(),
        kind: backup_target_kind_label(target.kind).to_owned(),
        target: target.target.clone(),
        label: target.label.clone(),
        state: state.to_owned(),
        root_cid: root_cid.to_owned(),
        upload: None,
        check: Some(check_summary(check)),
        error: None,
    }
}

fn error_report(
    target: &BackupTarget,
    state: &str,
    root_cid: &str,
    error: String,
) -> BackupOperationReport {
    BackupOperationReport {
        id: target.id.clone(),
        kind: backup_target_kind_label(target.kind).to_owned(),
        target: target.target.clone(),
        label: target.label.clone(),
        state: state.to_owned(),
        root_cid: root_cid.to_owned(),
        upload: None,
        check: None,
        error: Some(error),
    }
}

fn upload_summary(report: &UploadReport) -> BackupUploadSummary {
    BackupUploadSummary {
        total_hashes: report.total_hashes,
        uploaded: report.uploaded,
        already_present: report.already_present,
    }
}

fn check_summary(report: &BackupCheckReport) -> BackupCheckSummary {
    BackupCheckSummary {
        total_hashes: report.total_hashes,
        sample_size: report.sample_size,
        sampled_hashes: report.sampled_hashes,
        present: report.present,
        missing: report.missing,
        unknown: report.unknown,
        latency_ms: report.latency_ms,
        download_bytes: report.download_bytes,
        download_ms: report.download_ms,
        download_bytes_per_second: report.download_bytes_per_second,
        error: report.error.clone(),
    }
}

fn backup_target_check_from_report(
    report: &BackupCheckReport,
    state: &str,
    root_cid: &str,
    error: Option<String>,
) -> BackupTargetCheck {
    BackupTargetCheck {
        state: state.to_string(),
        root_cid: root_cid.to_string(),
        checked_at: unix_now(),
        total_hashes: report.total_hashes,
        sample_size: report.sample_size,
        sampled_hashes: report.sampled_hashes,
        present: report.present,
        missing: report.missing,
        unknown: report.unknown,
        latency_ms: report.latency_ms,
        download_bytes: report.download_bytes,
        download_ms: report.download_ms,
        download_bytes_per_second: report.download_bytes_per_second,
        error: error.or_else(|| report.error.clone()),
    }
}

fn error_backup_target_check(
    root_cid: &str,
    sample_size: usize,
    error: String,
) -> BackupTargetCheck {
    BackupTargetCheck {
        state: "error".to_string(),
        root_cid: root_cid.to_string(),
        checked_at: unix_now(),
        total_hashes: 0,
        sample_size,
        sampled_hashes: 0,
        present: 0,
        missing: 0,
        unknown: 0,
        latency_ms: None,
        download_bytes: None,
        download_ms: None,
        download_bytes_per_second: None,
        error: Some(error),
    }
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}
