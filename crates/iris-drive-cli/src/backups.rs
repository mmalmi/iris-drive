#[allow(clippy::wildcard_imports)]
use super::*;
use hashtree_core::diff::collect_hashes;

pub(crate) fn cmd_blossom_servers(
    config_dir: &std::path::Path,
    sub: BlossomServersCmd,
) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    match sub {
        BlossomServersCmd::List => {}
        BlossomServersCmd::Add { url } => {
            if !config.blossom_servers.contains(&url) {
                config.blossom_servers.push(url);
                config.save(config_path_in(config_dir))?;
            }
        }
        BlossomServersCmd::Remove { url } => {
            let before = config.blossom_servers.len();
            config.blossom_servers.retain(|s| s != &url);
            if config.blossom_servers.len() != before {
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

    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    match sub {
        BackupsCmd::List | BackupsCmd::Sync { .. } => {}
        BackupsCmd::Add { target, label } => {
            let target = parse_backup_target(&target, label).context("parsing backup target")?;
            config.upsert_backup_target(target);
            config.save(config_path_in(config_dir))?;
        }
        BackupsCmd::Remove { target } => {
            let target_id = parse_backup_target(&target, None)
                .context("parsing backup target")?
                .id;
            if config.remove_backup_target(&target_id).is_some() {
                config.save(config_path_in(config_dir))?;
            }
        }
    }
    println!(
        "{}",
        json!({
            "backup_targets": backup_targets_status(&config),
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
