#[allow(clippy::wildcard_imports)]
use super::*;
use hashtree_core::diff::collect_hashes;
use iris_drive_core::backup_summary::is_default_blossom_server;

pub(crate) fn cmd_blossom_servers(
    config_dir: &std::path::Path,
    sub: BlossomServersCmd,
) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    match sub {
        BlossomServersCmd::List => {}
        BlossomServersCmd::Add { url } => {
            let url = normalize_blossom_url(&url)?;
            if !config.blossom_servers.contains(&url) {
                config.blossom_servers.push(url.clone());
            }
            config.upsert_backup_target(parse_backup_target(&url, None)?);
            config.save(config_path_in(config_dir))?;
        }
        BlossomServersCmd::Remove { url } => {
            let url = normalize_blossom_url(&url)?;
            let before = config.blossom_servers.len();
            config.blossom_servers.retain(|s| s != &url);
            let target_id = parse_backup_target(&url, None)?.id;
            let removed_backup = config.remove_backup_target(&target_id).is_some();
            if config.blossom_servers.len() != before || removed_backup {
                config.save(config_path_in(config_dir))?;
            }
        }
    }
    println!("{}", serde_json::to_string_pretty(&config.blossom_servers)?);
    Ok(())
}

pub(crate) fn cmd_backups(config_dir: &std::path::Path, sub: BackupsCmd) -> Result<()> {
    if let BackupsCmd::Sync { target } = sub {
        return cmd_backups_sync(config_dir, target.as_deref());
    }
    if let BackupsCmd::Check {
        target,
        sample_size,
    } = sub
    {
        return cmd_backups_check(config_dir, target.as_deref(), sample_size);
    }

    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let mut changed = ensure_configured_blossom_backup_targets(&mut config);
    match sub {
        BackupsCmd::List | BackupsCmd::Sync { .. } | BackupsCmd::Check { .. } => {}
        BackupsCmd::Add { target, label } => {
            let target = parse_backup_target(&target, label).context("parsing backup target")?;
            if target.kind == BackupTargetKind::Blossom
                && !config.blossom_servers.contains(&target.target)
            {
                config.blossom_servers.push(target.target.clone());
            }
            config.upsert_backup_target(target);
            changed = true;
        }
        BackupsCmd::Remove { target } => {
            let target = parse_backup_target(&target, None).context("parsing backup target")?;
            let target_id = target.id;
            if target.kind == BackupTargetKind::Blossom {
                let before = config.blossom_servers.len();
                config
                    .blossom_servers
                    .retain(|server| server != &target.target);
                changed |= config.blossom_servers.len() != before;
            }
            if config.remove_backup_target(&target_id).is_some() {
                changed = true;
            }
        }
    }
    if changed {
        config.save(config_path_in(config_dir))?;
    }
    println!(
        "{}",
        json!({
            "backup_targets": configured_backup_targets_status(&config),
        })
    );
    Ok(())
}

#[allow(clippy::too_many_lines)]
pub(crate) fn cmd_backups_sync(config_dir: &std::path::Path, target: Option<&str>) -> Result<()> {
    let target_id = target
        .map(|target| parse_backup_target(target, None).map(|target| target.id))
        .transpose()
        .context("parsing backup target")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
        ensure_configured_blossom_backup_targets(&mut config);
        let root_cid_str = current_primary_root_cid(&config)
            .ok_or_else(|| anyhow::anyhow!("no current drive root; import files first"))?;
        let root_cid = Cid::parse(&root_cid_str).context("parsing current root cid")?;
        let device = iris_drive_core::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
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
                    let client =
                        iris_drive_core::blossom_sync_client(device.keys().clone(), &servers);
                    match iris_drive_core::blossom_sync::upload_tree(
                        daemon.tree(),
                        &root_cid,
                        &client,
                    )
                    .await
                    {
                        Ok(upload) => {
                            let sync = BackupTargetSync {
                                state: "synced".to_string(),
                                root_cid: root_cid_str.clone(),
                                synced_at: unix_now(),
                                total_hashes: upload.total_hashes,
                                uploaded: upload.uploaded,
                                already_present: upload.already_present,
                            };
                            config.backup_targets[index].last_sync = Some(sync);
                            reports.push(json!({
                                "id": target.id,
                                "kind": "blossom",
                                "target": target.target,
                                "label": target.label,
                                "state": "synced",
                                "root_cid": root_cid_str.as_str(),
                                "upload": upload_report_json(&upload),
                            }));
                        }
                        Err(error) => {
                            reports.push(json!({
                                "id": target.id,
                                "kind": "blossom",
                                "target": target.target,
                                "label": target.label,
                                "state": "error",
                                "root_cid": root_cid_str.as_str(),
                                "error": error.to_string(),
                            }));
                        }
                    }
                }
                BackupTargetKind::Fips => {
                    reports.push(json!({
                        "id": target.id,
                        "kind": "fips",
                        "target": target.target,
                        "label": target.label,
                        "state": "pending",
                        "root_cid": root_cid_str.as_str(),
                        "error": "direct FIPS backup transport pending",
                    }));
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
                            let sync = BackupTargetSync {
                                state: "synced".to_string(),
                                root_cid: root_cid_str.clone(),
                                synced_at: unix_now(),
                                total_hashes: upload.total_hashes,
                                uploaded: upload.uploaded,
                                already_present: upload.already_present,
                            };
                            config.backup_targets[index].last_sync = Some(sync);
                            reports.push(json!({
                                "id": target.id,
                                "kind": "filesystem",
                                "target": target.target,
                                "label": target.label,
                                "state": "synced",
                                "root_cid": root_cid_str.as_str(),
                                "upload": upload_report_json(&upload),
                            }));
                        }
                        Err(error) => {
                            reports.push(json!({
                                "id": target.id,
                                "kind": "filesystem",
                                "target": target.target,
                                "label": target.label,
                                "state": "error",
                                "root_cid": root_cid_str.as_str(),
                                "error": error.to_string(),
                            }));
                        }
                    }
                }
                BackupTargetKind::Lmdb => {
                    match upload_tree_to_lmdb_replica(
                        daemon.tree(),
                        &root_cid,
                        Path::new(&target.target),
                    )
                    .await
                    {
                        Ok(upload) => {
                            let sync = BackupTargetSync {
                                state: "synced".to_string(),
                                root_cid: root_cid_str.clone(),
                                synced_at: unix_now(),
                                total_hashes: upload.total_hashes,
                                uploaded: upload.uploaded,
                                already_present: upload.already_present,
                            };
                            config.backup_targets[index].last_sync = Some(sync);
                            reports.push(json!({
                                "id": target.id,
                                "kind": "lmdb",
                                "target": target.target,
                                "label": target.label,
                                "state": "synced",
                                "root_cid": root_cid_str.as_str(),
                                "upload": upload_report_json(&upload),
                            }));
                        }
                        Err(error) => {
                            reports.push(json!({
                                "id": target.id,
                                "kind": "lmdb",
                                "target": target.target,
                                "label": target.label,
                                "state": "error",
                                "root_cid": root_cid_str.as_str(),
                                "error": error.to_string(),
                            }));
                        }
                    }
                }
            }
        }

        config.save(config_path_in(config_dir))?;
        println!("{}", json!({ "reports": reports }));
        Ok::<_, anyhow::Error>(())
    })
}

#[allow(clippy::too_many_lines)]
pub(crate) fn cmd_backups_check(
    config_dir: &std::path::Path,
    target: Option<&str>,
    sample_size: usize,
) -> Result<()> {
    let target_id = target
        .map(|target| parse_backup_target(target, None).map(|target| target.id))
        .transpose()
        .context("parsing backup target")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
        ensure_configured_blossom_backup_targets(&mut config);
        let root_cid_str = current_primary_root_cid(&config)
            .ok_or_else(|| anyhow::anyhow!("no current drive root; import files first"))?;
        let root_cid = Cid::parse(&root_cid_str).context("parsing current root cid")?;
        let device = iris_drive_core::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
        let daemon = Daemon::open(config_dir).context("opening daemon for backup check")?;
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
                    let client =
                        iris_drive_core::blossom_sync_client(device.keys().clone(), &servers)
                            .with_timeout(std::time::Duration::from_secs(5));
                    match iris_drive_core::blossom_sync::check_tree_on_server(
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
                            config.backup_targets[index].last_check =
                                Some(backup_target_check_from_report(
                                    &check,
                                    &state,
                                    &root_cid_str,
                                    None,
                                ));
                            reports.push(json!({
                                "id": target.id,
                                "kind": "blossom",
                                "target": target.target,
                                "label": target.label,
                                "state": state,
                                "root_cid": root_cid_str.as_str(),
                                "check": check_report_json(&check),
                            }));
                        }
                        Err(error) => {
                            let error = error.to_string();
                            config.backup_targets[index].last_check =
                                Some(error_backup_target_check(
                                    &root_cid_str,
                                    sample_size,
                                    error.clone(),
                                ));
                            reports.push(json!({
                                "id": target.id,
                                "kind": "blossom",
                                "target": target.target,
                                "label": target.label,
                                "state": "error",
                                "root_cid": root_cid_str.as_str(),
                                "error": error,
                            }));
                        }
                    }
                }
                BackupTargetKind::Fips => {
                    reports.push(json!({
                        "id": target.id,
                        "kind": "fips",
                        "target": target.target,
                        "label": target.label,
                        "state": "pending",
                        "root_cid": root_cid_str.as_str(),
                        "error": "direct FIPS backup checks pending",
                    }));
                }
                BackupTargetKind::Filesystem | BackupTargetKind::Lmdb => {
                    reports.push(json!({
                        "id": target.id,
                        "kind": backup_target_kind_label(target.kind),
                        "target": target.target,
                        "label": target.label,
                        "state": "pending",
                        "root_cid": root_cid_str.as_str(),
                        "error": "local replica checks pending",
                    }));
                }
            }
        }

        config.save(config_path_in(config_dir))?;
        println!("{}", json!({ "reports": reports }));
        Ok::<_, anyhow::Error>(())
    })
}

pub(crate) async fn upload_tree_to_filesystem_replica<S>(
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

pub(crate) async fn upload_tree_to_lmdb_replica<S>(
    tree: &HashTree<S>,
    root: &Cid,
    path: &Path,
) -> Result<UploadReport>
where
    S: Store + Send + Sync + 'static,
{
    let replica = LmdbBlobStore::new(path)
        .with_context(|| format!("opening LMDB backup target at {}", path.display()))?;
    upload_tree_to_replica(tree, root, &replica).await
}

pub(crate) async fn upload_tree_to_replica<S, R>(
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

pub(crate) fn ensure_configured_blossom_backup_targets(config: &mut AppConfig) -> bool {
    let mut changed = false;
    for target in implicit_configured_blossom_backup_targets(config) {
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

pub(crate) fn effective_backup_targets(config: &AppConfig) -> Vec<BackupTarget> {
    let mut targets = config.backup_targets.clone();
    for target in implicit_configured_blossom_backup_targets(config) {
        if !targets.iter().any(|existing| existing.id == target.id) {
            targets.push(target);
        }
    }
    targets
}

fn implicit_configured_blossom_backup_targets(config: &AppConfig) -> Vec<BackupTarget> {
    config
        .blossom_servers
        .iter()
        .filter(|server| !is_default_blossom_server(server))
        .filter_map(|server| parse_backup_target(server, None).ok())
        .collect()
}

pub(crate) fn parse_backup_target(input: &str, label: Option<String>) -> Result<BackupTarget> {
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

pub(crate) fn normalize_blossom_url(value: &str) -> Result<String> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Ok(trimmed.to_string())
    } else {
        Err(anyhow::anyhow!(
            "expected Blossom target URL starting with http:// or https://"
        ))
    }
}

pub(crate) fn looks_like_local_backup_path(value: &str) -> bool {
    let value = value.trim();
    value.starts_with('/')
        || value.starts_with("~/")
        || value.starts_with("./")
        || value.starts_with("../")
        || value.starts_with("\\\\")
        || value.as_bytes().get(1).is_some_and(|byte| *byte == b':')
}

pub(crate) fn normalize_local_backup_path(value: &str) -> Result<String> {
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

pub(crate) fn upload_report_json(report: &UploadReport) -> Value {
    json!({
        "total_hashes": report.total_hashes,
        "uploaded": report.uploaded,
        "already_present": report.already_present,
    })
}

pub(crate) fn check_report_json(
    report: &iris_drive_core::blossom_sync::BackupCheckReport,
) -> Value {
    json!({
        "total_hashes": report.total_hashes,
        "sample_size": report.sample_size,
        "sampled_hashes": report.sampled_hashes,
        "present": report.present,
        "missing": report.missing,
        "unknown": report.unknown,
        "latency_ms": report.latency_ms,
        "download_bytes": report.download_bytes,
        "download_ms": report.download_ms,
        "download_bytes_per_second": report.download_bytes_per_second,
        "error": report.error,
    })
}

pub(crate) fn backup_target_check_from_report(
    report: &iris_drive_core::blossom_sync::BackupCheckReport,
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

pub(crate) fn error_backup_target_check(
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
