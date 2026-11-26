#[allow(clippy::wildcard_imports)]
use super::*;
pub(crate) use iris_drive_core::backup_ops::{
    backup_target_check_from_report, error_backup_target_check, normalize_blossom_url,
    parse_backup_target, upload_tree_to_filesystem_replica, upload_tree_to_replica,
};
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
    if let BackupsCmd::Friends(command) = sub {
        return friend_backups::run(config_dir, command);
    }
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
        BackupsCmd::List
        | BackupsCmd::Sync { .. }
        | BackupsCmd::Check { .. }
        | BackupsCmd::Friends(_) => {}
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
        let device =
            iris_drive_core::AppKey::load(key_path_in(config_dir)).context("loading app key")?;
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
        let device =
            iris_drive_core::AppKey::load(key_path_in(config_dir)).context("loading app key")?;
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
