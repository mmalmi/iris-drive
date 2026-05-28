#[allow(clippy::wildcard_imports)]
use super::*;

#[allow(clippy::too_many_lines)]
pub(crate) fn cmd_status(config_dir: &std::path::Path) -> Result<()> {
    let initialized = already_initialized(config_dir);
    let config = AppConfig::load_or_default(config_path_in(config_dir))
        .with_context(|| format!("reading config at {}", config_path_in(config_dir).display()))?;
    let daemon_status = load_daemon_status(config_dir);
    let blocks_dir = config_dir.join("blocks");
    let block_stats =
        collect_file_stats_with_entry_limit(&blocks_dir, Some(STATUS_BLOCK_STATS_ENTRY_LIMIT))
            .with_context(|| format!("reading block store stats at {}", blocks_dir.display()))?;
    let current_root_cid = current_primary_root_cid(&config);
    let current_root_private = current_root_cid.as_deref().and_then(root_is_private);
    let drive_iris_to_url = current_root_cid
        .as_ref()
        .and_then(|_| drive_iris_to_url_for_primary_drive(&config));
    let snapshot_url = current_root_cid
        .as_deref()
        .and_then(drive_iris_to_snapshot_url_for_root);
    let browser_gateway_urls = local_gateway_urls_for_root(
        current_root_cid.as_deref(),
        DEFAULT_GATEWAY_PORT,
        config.local_nhash_resolver_enabled,
    );
    let merged_stats = primary_drive_stats(config_dir, &config);
    let root_file_stats = current_root_cid
        .as_deref()
        .and_then(|root| root_file_stats(config_dir, root));
    let top_level_entries = merged_stats
        .as_ref()
        .map(|stats| stats.top_level_entries)
        .or_else(|| {
            current_root_cid
                .as_deref()
                .and_then(|root| root_top_level_entries(config_dir, root))
        });
    let file_count = merged_stats
        .as_ref()
        .map(|stats| stats.file_count)
        .or_else(|| root_file_stats.as_ref().map(|stats| stats.file_count));
    let visible_file_bytes = merged_stats
        .as_ref()
        .map(|stats| stats.visible_file_bytes)
        .or_else(|| {
            root_file_stats
                .as_ref()
                .map(|stats| stats.visible_file_bytes)
        });
    let conflict_status = current_root_cid
        .as_deref()
        .and_then(|root| root_conflict_status(config_dir, root))
        .unwrap_or_else(|| conflict_status_payload(&[]));
    let peers = peer_statuses(config_dir, &config, daemon_status.as_ref());
    let authorized_device_count = peers.len();
    let published_device_roots = config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .map_or(0, |drive| drive.device_roots.len());
    let fips_diagnostics = fips_network_diagnostics(&config, daemon_status.as_ref());
    let backup_targets = backup_targets_status(&config);
    let backup_target_count = backup_targets.len();
    let account_block = status_account_block(&config);
    println!(
        "{}",
        json!({
            "initialized": initialized,
            "config_dir": config_dir.display().to_string(),
            "pubkey_npub": config.account.as_ref().map(|s| account_npub(&s.device_pubkey)),
            "account": account_block,
            "drives": config.drives.iter().map(|d| json!({
                "drive_id": d.drive_id,
                "display_name": d.display_name,
                "owner_pubkey": d.owner_pubkey,
                "role": drive_role_label(d.role),
                "last_root_cid": d.last_root_cid,
                "device_root_count": d.device_roots.len(),
            })).collect::<Vec<_>>(),
            "hashtree": {
                "blocks_dir": blocks_dir.display().to_string(),
                "local_block_count": block_stats.file_count,
                "local_block_bytes": block_stats.total_bytes,
                "local_block_stats_truncated": block_stats.truncated,
                "current_root_cid": current_root_cid,
                "current_root_private": current_root_private,
                "drive_iris_to_url": drive_iris_to_url,
                "files_iris_to_url": drive_iris_to_url,
                "snapshot_url": snapshot_url,
                "permalink_url": snapshot_url,
                "local_gateway": browser_gateway_urls,
                "file_count": file_count,
                "top_level_entries": top_level_entries,
                "visible_file_bytes": visible_file_bytes,
            },
            "network": {
                "relays": config.relays,
                "blossom_servers": config.blossom_servers,
                "backup_target_count": backup_target_count,
                "backup_targets": backup_targets,
                "authorized_device_count": authorized_device_count,
                "published_device_roots": published_device_roots,
                "relay_statuses": daemon_status
                    .as_ref()
                    .and_then(|status| status.get("relay_statuses"))
                    .cloned()
                    .unwrap_or_else(|| json!([])),
                "fips": fips_diagnostics,
            },
            "settings": settings_status(&config),
            "daemon": daemon_status,
            "conflicts": conflict_status,
            "peers": peers,
        })
    );
    Ok(())
}

pub(crate) fn cmd_nhash_resolver(
    config_dir: &std::path::Path,
    sub: Option<NhashResolverCmd>,
) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let mut changed = false;
    match sub.unwrap_or(NhashResolverCmd::Status) {
        NhashResolverCmd::Status => {}
        NhashResolverCmd::Enable => {
            changed = !config.local_nhash_resolver_enabled;
            config.local_nhash_resolver_enabled = true;
        }
        NhashResolverCmd::Disable => {
            changed = config.local_nhash_resolver_enabled;
            config.local_nhash_resolver_enabled = false;
        }
    }
    if changed {
        config.save(config_path_in(config_dir))?;
    }
    println!(
        "{}",
        local_nhash_resolver_status(&config, DEFAULT_GATEWAY_PORT, changed)
    );
    Ok(())
}

pub(crate) fn settings_status(config: &AppConfig) -> Value {
    json!({
        "local_nhash_resolver_enabled": config.local_nhash_resolver_enabled,
    })
}

pub(crate) fn local_nhash_resolver_status(
    config: &AppConfig,
    port: u16,
    restart_required: bool,
) -> Value {
    json!({
        "enabled": config.local_nhash_resolver_enabled,
        "host": iris_drive_core::gateway::LOCAL_NHASH_RESOLVER_HOST,
        "port": port,
        "base_url": format!(
            "http://{}:{port}/",
            iris_drive_core::gateway::LOCAL_NHASH_RESOLVER_HOST,
        ),
        "url_pattern": format!(
            "http://{}:{port}/<nhash>/<filename>",
            iris_drive_core::gateway::LOCAL_NHASH_RESOLVER_HOST,
        ),
        "restart_required": restart_required,
    })
}

pub(crate) fn status_account_block(config: &AppConfig) -> Option<Value> {
    config.account.as_ref().map(|state| {
        json!({
            "owner_npub": account_npub(&state.owner_pubkey),
            "device_npub": account_npub(&state.device_pubkey),
            "has_owner_signing_authority": state.has_owner_signing_authority,
            "authorization_state": authorization_state_label(state),
            "roster_size": state.app_keys.as_ref().map_or(0, |s| s.devices.len()),
            "profile": config.user_profile.as_ref().map(|profile| json!({
                "username": profile.username,
                "photo_path": profile.photo_path,
            })),
            "device_link_request": device_link_request_json(state),
            "device_link_invite": device_link_invite_json(state),
            "inbound_device_link_requests": inbound_device_link_requests_json(state),
        })
    })
}

pub(crate) fn current_primary_root_cid(config: &AppConfig) -> Option<String> {
    config
        .account
        .as_ref()
        .and_then(|state| {
            config
                .drive(iris_drive_core::PRIMARY_DRIVE_ID)
                .and_then(|drive| drive.device_roots.get(&state.device_pubkey))
                .map(|root| root.root_cid.clone())
        })
        .or_else(|| {
            config
                .drive(iris_drive_core::PRIMARY_DRIVE_ID)
                .and_then(|drive| drive.last_root_cid.clone())
        })
}

pub(crate) fn backup_targets_status(config: &AppConfig) -> Vec<Value> {
    effective_backup_targets(config)
        .iter()
        .map(backup_target_status)
        .collect()
}

pub(crate) fn backup_target_status(target: &BackupTarget) -> Value {
    json!({
        "id": target.id.as_str(),
        "kind": backup_target_kind_label(target.kind),
        "target": target.target.as_str(),
        "label": target.label.as_deref(),
        "enabled": target.enabled,
        "last_sync": target.last_sync.as_ref().map(backup_target_sync_status),
        "last_check": target.last_check.as_ref().map(backup_target_check_status),
    })
}

pub(crate) fn backup_target_sync_status(sync: &BackupTargetSync) -> Value {
    json!({
        "state": sync.state.as_str(),
        "root_cid": sync.root_cid.as_str(),
        "synced_at": sync.synced_at,
        "total_hashes": sync.total_hashes,
        "uploaded": sync.uploaded,
        "already_present": sync.already_present,
    })
}

pub(crate) fn backup_target_check_status(check: &BackupTargetCheck) -> Value {
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

const DAEMON_STATUS_SCHEMA: u32 = 1;
const DAEMON_STATUS_FRESH_SECS: i64 = 15;

pub(crate) fn daemon_status_path(config_dir: &Path) -> PathBuf {
    config_dir.join("daemon-status.json")
}

pub(crate) fn load_daemon_status(config_dir: &Path) -> Option<Value> {
    let pid = daemon_lock_pid(config_dir);
    let running = pid.is_some_and(process_is_running);
    let now = unix_now();
    let raw = std::fs::read_to_string(daemon_status_path(config_dir)).ok();
    let mut value = raw
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or_else(|| json!({}));
    let object = value.as_object_mut()?;
    let updated_at = object
        .get("updated_at")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let fresh = running && now.saturating_sub(updated_at) <= DAEMON_STATUS_FRESH_SECS;
    object.insert("schema".to_string(), json!(DAEMON_STATUS_SCHEMA));
    object.insert("running".to_string(), json!(running));
    object.insert("pid".to_string(), json!(pid));
    object.insert("fresh".to_string(), json!(fresh));
    if !fresh
        && let Some(fips) = object
            .get_mut("fips_block_sync")
            .and_then(Value::as_object_mut)
    {
        fips.insert("connected_peers".to_string(), json!([]));
        fips.insert("mesh_peers".to_string(), json!([]));
    }
    Some(value)
}

pub(crate) fn write_daemon_status(config_dir: &Path, mut payload: Value) {
    let now = unix_now();
    if let Some(payload_object) = payload.as_object_mut()
        && let Ok(raw) = std::fs::read_to_string(daemon_status_path(config_dir))
        && let Ok(existing) = serde_json::from_str::<Value>(&raw)
        && let Some(existing_object) = existing.as_object()
    {
        for key in [
            "last_block_sync",
            "block_sync_by_root",
            "relays",
            "owner_npub",
            "provider_update_mode",
            "watch_debounce_ms",
            "mount",
            "relay_statuses",
            "embedded_hashtree",
            "browser_gateway",
            "fips_block_sync",
            "fips_block_sync_error",
        ] {
            if !payload_object.contains_key(key)
                && let Some(value) = existing_object.get(key)
            {
                payload_object.insert(key.to_string(), value.clone());
            }
        }
    }
    if let Some(object) = payload.as_object_mut() {
        object.insert("schema".to_string(), json!(DAEMON_STATUS_SCHEMA));
        object.insert("pid".to_string(), json!(std::process::id()));
        object.insert("running".to_string(), json!(true));
        object.insert("fresh".to_string(), json!(true));
        object.insert("updated_at".to_string(), json!(now));
    }
    if let Some(parent) = daemon_status_path(config_dir).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&payload) {
        let _ = std::fs::write(daemon_status_path(config_dir), bytes);
    }
}

pub(crate) fn merge_daemon_status(
    config_dir: &Path,
    update: impl FnOnce(&mut serde_json::Map<String, Value>),
) {
    let mut value = std::fs::read_to_string(daemon_status_path(config_dir))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or_else(|| json!({}));
    if !value.is_object() {
        value = json!({});
    }
    if let Some(object) = value.as_object_mut() {
        update(object);
    }
    write_daemon_status(config_dir, value);
}

pub(crate) fn daemon_lock_pid(config_dir: &Path) -> Option<u32> {
    std::fs::read_to_string(config_dir.join("daemon.lock"))
        .ok()
        .and_then(|contents| contents.trim().parse::<u32>().ok())
}

pub(crate) fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

pub(crate) fn cmd_conflicts(config_dir: &std::path::Path, command: ConflictsCmd) -> Result<()> {
    match command {
        ConflictsCmd::Resolve { conflict_id } => cmd_conflict_resolve(config_dir, &conflict_id),
    }
}

pub(crate) fn cmd_conflict_resolve(config_dir: &std::path::Path, conflict_id: &str) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let mut daemon = Daemon::open(config_dir)
            .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
        let report = daemon
            .resolve_conflict_record(conflict_id)
            .await
            .with_context(|| format!("resolving conflict {conflict_id}"))?;
        println!(
            "{}",
            json!({
                "conflict_id": report.conflict_id,
                "previous_root_cid": report.previous_root_cid,
                "root_cid": report.root_cid,
                "changed": report.changed,
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}

#[derive(Debug, Default)]
pub(crate) struct FileStats {
    pub(crate) file_count: u64,
    pub(crate) total_bytes: u64,
    pub(crate) truncated: bool,
}

fn retry_interrupted_io<T>(mut op: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    loop {
        match op() {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            result => return result,
        }
    }
}

pub(crate) fn collect_file_stats_with_entry_limit(
    path: &Path,
    entry_limit: Option<usize>,
) -> Result<FileStats> {
    let metadata = match retry_interrupted_io(|| std::fs::metadata(path)) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileStats::default());
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.is_file() {
        return Ok(FileStats {
            file_count: 1,
            total_bytes: metadata.len(),
            truncated: false,
        });
    }
    if !metadata.is_dir() {
        return Ok(FileStats::default());
    }

    let mut stats = FileStats::default();
    let mut stack = vec![path.to_path_buf()];
    let mut visited_entries = 0usize;
    while let Some(dir) = stack.pop() {
        if entry_limit.is_some_and(|limit| visited_entries >= limit) {
            stats.truncated = true;
            break;
        }
        let mut entries = retry_interrupted_io(|| std::fs::read_dir(&dir))?;
        loop {
            if entry_limit.is_some_and(|limit| visited_entries >= limit) {
                stats.truncated = true;
                break;
            }
            let Some(entry) = retry_interrupted_io(|| match entries.next() {
                Some(entry) => entry.map(Some),
                None => Ok(None),
            })?
            else {
                break;
            };
            visited_entries += 1;
            let path = entry.path();
            let metadata = retry_interrupted_io(|| entry.metadata())?;
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() {
                stats.file_count += 1;
                stats.total_bytes += metadata.len();
            }
        }
    }
    Ok(stats)
}

pub(crate) fn root_is_private(root_cid: &str) -> Option<bool> {
    Cid::parse(root_cid).ok().map(|cid| cid.key.is_some())
}

const DRIVE_IRIS_TO_ORIGIN: &str = "https://drive.iris.to";

pub(crate) fn drive_iris_to_url_for_primary_drive(config: &AppConfig) -> Option<String> {
    let drive = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)?;
    Some(drive_iris_to_url_for_drive(
        &drive.owner_pubkey,
        &drive.drive_id,
    ))
}

pub(crate) fn drive_iris_to_url_for_drive(owner_pubkey_hex: &str, drive_id: &str) -> String {
    format!(
        "{DRIVE_IRIS_TO_ORIGIN}/#/{}/{}",
        account_npub(owner_pubkey_hex),
        percent_encode_path_segment(drive_id)
    )
}

pub(crate) fn drive_iris_to_snapshot_url_for_root(root_cid: &str) -> Option<String> {
    let cid = Cid::parse(root_cid).ok()?;
    let nhash = nhash_encode_full(&NHashData {
        hash: cid.hash,
        decrypt_key: cid.key,
    })
    .ok()?;
    Some(format!("{DRIVE_IRIS_TO_ORIGIN}/#/{nhash}"))
}

pub(crate) fn local_gateway_urls_for_root(
    root_cid: Option<&str>,
    port: u16,
    enabled: bool,
) -> serde_json::Value {
    if !enabled {
        return json!({
            "enabled": false,
            "host": iris_drive_core::gateway::LOCAL_NHASH_RESOLVER_HOST,
            "port": port,
        });
    }
    let immutable_url = root_cid
        .and_then(|root| Cid::parse(root).ok())
        .map(|cid| iris_drive_core::gateway::local_immutable_url(port, &cid));
    let nhash_url = root_cid
        .and_then(|root| Cid::parse(root).ok())
        .and_then(|cid| {
            nhash_encode_full(&NHashData {
                hash: cid.hash,
                decrypt_key: cid.key,
            })
            .ok()
        })
        .map(|nhash| iris_drive_core::gateway::local_nhash_url(port, &nhash, None));
    json!({
        "enabled": true,
        "portal_url": format!("http://sites.iris.localhost:{port}/"),
        "primary_drive_url": iris_drive_core::gateway::local_drive_url(
            port,
            iris_drive_core::PRIMARY_DRIVE_ID,
        ),
        "nhash_resolver_url": format!(
            "http://{}:{port}/",
            iris_drive_core::gateway::LOCAL_NHASH_RESOLVER_HOST,
        ),
        "nhash_url": nhash_url,
        "immutable_url": immutable_url,
    })
}

pub(crate) fn percent_encode_path_segment(segment: &str) -> String {
    let mut encoded = String::new();
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(&mut encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

pub(crate) const STATUS_BLOCK_STATS_ENTRY_LIMIT: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PrimaryDriveStatusStats {
    pub(crate) file_count: usize,
    pub(crate) top_level_entries: usize,
    pub(crate) visible_file_bytes: u64,
}

pub(crate) fn root_top_level_entries(config_dir: &Path, root_cid: &str) -> Option<usize> {
    let cid = Cid::parse(root_cid).ok()?;
    let daemon = Daemon::open(config_dir).ok()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    runtime.block_on(async {
        daemon
            .tree()
            .list_directory(&cid)
            .await
            .ok()
            .map(|entries| {
                entries
                    .iter()
                    .filter(|entry| entry.name != iris_drive_core::META_DIR)
                    .count()
            })
    })
}

pub(crate) fn root_file_stats(
    config_dir: &Path,
    root_cid: &str,
) -> Option<PrimaryDriveStatusStats> {
    let cid = Cid::parse(root_cid).ok()?;
    let daemon = Daemon::open(config_dir).ok()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let (files, _tombstones) =
        runtime.block_on(async { walk_device_tree(daemon.tree(), &cid).await.ok() })?;
    let top_level_entries = files
        .iter()
        .filter_map(|entry| entry.path.split('/').next())
        .filter(|segment| !segment.is_empty())
        .collect::<BTreeSet<_>>()
        .len();
    let visible_file_bytes = files.iter().map(|entry| entry.size).sum();
    Some(PrimaryDriveStatusStats {
        file_count: files.len(),
        top_level_entries,
        visible_file_bytes,
    })
}

pub(crate) fn root_file_count(config_dir: &Path, root_cid: &str) -> Option<usize> {
    root_file_stats(config_dir, root_cid).map(|stats| stats.file_count)
}

pub(crate) fn primary_drive_stats(
    config_dir: &Path,
    config: &AppConfig,
) -> Option<PrimaryDriveStatusStats> {
    let daemon = Daemon::open(config_dir).ok()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    runtime
        .block_on(async {
            iris_drive_core::primary_merged_view(daemon.tree(), config)
                .await
                .ok()
        })
        .map(|merged| PrimaryDriveStatusStats {
            file_count: merged.file_count(),
            top_level_entries: merged.top_level_entries(),
            visible_file_bytes: merged.view.files.iter().map(|entry| entry.size).sum(),
        })
}

pub(crate) fn root_conflict_status(config_dir: &Path, root_cid: &str) -> Option<serde_json::Value> {
    let cid = Cid::parse(root_cid).ok()?;
    let daemon = Daemon::open(config_dir).ok()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let records = runtime.block_on(async {
        iris_drive_core::read_conflict_records(daemon.tree(), &cid)
            .await
            .ok()
    })?;
    Some(conflict_status_payload(&records))
}

pub(crate) fn conflict_status_payload(
    records: &[iris_drive_core::ConflictRecord],
) -> serde_json::Value {
    let unresolved_records: Vec<_> = records
        .iter()
        .filter(|record| record.state == iris_drive_core::ConflictState::Unresolved)
        .collect();
    let unresolved: Vec<_> = unresolved_records
        .iter()
        .map(|record| conflict_record_status_payload(record))
        .collect();
    let overflow_paths = conflict_overflow_payload(&unresolved_records);
    let resolved_count = records.len().saturating_sub(unresolved.len());

    json!({
        "total_count": records.len(),
        "unresolved_count": unresolved.len(),
        "resolved_count": resolved_count,
        "per_path_cap": CONFLICT_STATUS_PATH_CAP,
        "overflow_count": overflow_paths.len(),
        "overflow_paths": overflow_paths,
        "unresolved": unresolved,
    })
}

pub(crate) fn conflict_overflow_payload(
    records: &[&iris_drive_core::ConflictRecord],
) -> Vec<serde_json::Value> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for record in records {
        *counts.entry(record.path.as_str()).or_default() += 1;
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > CONFLICT_STATUS_PATH_CAP)
        .map(|(path, count)| {
            json!({
                "path": path,
                "unresolved_count": count,
                "cap": CONFLICT_STATUS_PATH_CAP,
            })
        })
        .collect()
}

pub(crate) fn conflict_record_status_payload(
    record: &iris_drive_core::ConflictRecord,
) -> serde_json::Value {
    json!({
        "conflict_id": record.conflict_id.as_str(),
        "path": record.path.as_str(),
        "visible_conflict_path": record.visible_conflict_path.as_str(),
        "created_at": record.created_at,
        "state": conflict_state_label(&record.state),
    })
}

pub(crate) fn conflict_state_label(state: &iris_drive_core::ConflictState) -> &'static str {
    match state {
        iris_drive_core::ConflictState::Unresolved => "unresolved",
        iris_drive_core::ConflictState::Resolved => "resolved",
    }
}

pub(crate) fn peer_statuses(
    config_dir: &Path,
    config: &AppConfig,
    daemon_status: Option<&Value>,
) -> Vec<serde_json::Value> {
    let Some(account) = config.account.as_ref() else {
        return Vec::new();
    };
    let Some(snapshot) = account.app_keys.as_ref() else {
        return Vec::new();
    };
    let primary_drive = config.drive(iris_drive_core::PRIMARY_DRIVE_ID);

    let daemon_running = daemon_status
        .and_then(|status| status.get("running"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fips_status = daemon_status
        .and_then(|status| status.get("fips_block_sync"))
        .filter(|value| value.is_object());
    let connected_fips =
        string_set_from_json_array(fips_status.and_then(|status| status.get("connected_peers")));
    let mesh_fips =
        string_set_from_json_array(fips_status.and_then(|status| status.get("mesh_peers")));
    let authorized_fips =
        string_set_from_json_array(fips_status.and_then(|status| status.get("authorized_peers")));
    let block_sync_by_root = daemon_status
        .and_then(|status| status.get("block_sync_by_root"))
        .filter(|value| value.is_object());

    snapshot
        .devices
        .iter()
        .map(|device| {
            let root = primary_drive.and_then(|drive| drive.device_roots.get(&device.pubkey));
            let root_cid = root.map(|root| root.root_cid.clone());
            let root_private = root_cid.as_deref().and_then(root_is_private);
            let root_available = root_cid
                .as_deref()
                .map(|root| root_file_count(config_dir, root).is_some());
            let device_npub = account_npub(&device.pubkey);
            let is_current_device = device.pubkey == account.device_pubkey;
            let fips_direct_online = connected_fips.contains(&device_npub);
            let fips_mesh_online = mesh_fips.contains(&device_npub);
            let fips_online = if is_current_device {
                daemon_running
            } else {
                fips_direct_online || fips_mesh_online
            };
            let fips_online_via = if is_current_device && fips_online {
                Some("local")
            } else if fips_direct_online {
                Some("direct")
            } else if fips_mesh_online {
                Some("mesh")
            } else {
                None
            };
            let sync_state = device_sync_state(is_current_device, root.is_some(), root_available);
            let last_block_sync = root_cid
                .as_ref()
                .and_then(|root| block_sync_by_root.and_then(|map| map.get(root)).cloned());
            json!({
                "device_pubkey": device.pubkey,
                "device_npub": device_npub,
                "label": device.label,
                "role": device_role_label(device.role),
                "authorized": true,
                "is_current_device": is_current_device,
                "added_at": device.added_at,
                "fips_authorized": authorized_fips.contains(&device_npub),
                "fips_online": fips_online,
                "fips_direct_online": fips_direct_online,
                "fips_mesh_online": fips_mesh_online,
                "fips_online_via": fips_online_via,
                "has_root": root.is_some(),
                "root_cid": root_cid,
                "root_private": root_private,
                "root_available": root_available,
                "sync_state": sync_state,
                "last_block_sync": last_block_sync,
                "published_at": root.map(|root| root.published_at),
                "dck_generation": root.map(|root| root.dck_generation),
                "device_seq": root.map(|root| root.device_seq),
            })
        })
        .collect()
}

pub(crate) fn fips_network_diagnostics(config: &AppConfig, daemon_status: Option<&Value>) -> Value {
    let running = daemon_status
        .and_then(|status| status.get("running"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fresh = daemon_status
        .and_then(|status| status.get("fresh"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fips_status = daemon_status
        .and_then(|status| status.get("fips_block_sync"))
        .filter(|value| value.is_object());
    let mut authorized_peers =
        string_vec_from_json_array(fips_status.and_then(|status| status.get("authorized_peers")));
    if authorized_peers.is_empty() {
        authorized_peers = configured_fips_authorized_peer_npubs(config);
    }
    let connected_peers =
        string_vec_from_json_array(fips_status.and_then(|status| status.get("connected_peers")));
    let mesh_peers =
        string_vec_from_json_array(fips_status.and_then(|status| status.get("mesh_peers")));
    let authorized_set = authorized_peers.iter().cloned().collect::<BTreeSet<_>>();
    let connected_set = connected_peers.iter().cloned().collect::<BTreeSet<_>>();
    let roster_connected_peer_count = connected_set.intersection(&authorized_set).count();
    let other_peer_count = connected_set.difference(&authorized_set).count();
    let error = daemon_status
        .and_then(|status| status.get("fips_block_sync_error"))
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or(Value::Null);
    let nostr_discovery_app = fips_status
        .and_then(|status| status.get("nostr_discovery_app"))
        .and_then(Value::as_str)
        .or_else(|| {
            fips_status
                .and_then(|status| status.get("discovery_scope"))
                .and_then(Value::as_str)
        });

    json!({
        "enabled": fips_status.is_some(),
        "running": running,
        "fresh": fresh,
        "endpoint_npub": fips_status
            .and_then(|status| status.get("endpoint_npub"))
            .and_then(Value::as_str),
        "discovery_scope": fips_status
            .and_then(|status| status.get("discovery_scope"))
            .and_then(Value::as_str),
        "nostr_discovery_app": nostr_discovery_app,
        "udp_enabled": fips_status
            .and_then(|status| status.get("udp_enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "udp_bind_addr": fips_status
            .and_then(|status| status.get("udp_bind_addr"))
            .and_then(Value::as_str),
        "udp_public": fips_status
            .and_then(|status| status.get("udp_public"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "udp_external_addr": fips_status
            .and_then(|status| status.get("udp_external_addr"))
            .and_then(Value::as_str),
        "webrtc_enabled": fips_status
            .and_then(|status| status.get("webrtc_enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        "webrtc_max_connections": fips_status
            .and_then(|status| status.get("webrtc_max_connections"))
            .and_then(Value::as_u64),
        "open_discovery_max_pending": fips_status
            .and_then(|status| status.get("open_discovery_max_pending"))
            .and_then(Value::as_u64),
        "mesh_peer_count": fips_status
            .and_then(|status| status.get("mesh_peer_count"))
            .and_then(Value::as_u64)
            .unwrap_or(mesh_peers.len() as u64),
        "roster_peer_count": authorized_peers.len(),
        "roster_connected_peer_count": roster_connected_peer_count,
        "authorized_peer_count": authorized_peers.len(),
        "connected_peer_count": connected_peers.len(),
        "other_peer_count": other_peer_count,
        "authorized_peers": authorized_peers,
        "connected_peers": connected_peers,
        "mesh_peers": mesh_peers,
        "relay_statuses": fips_status
            .and_then(|status| status.get("relay_statuses"))
            .cloned()
            .unwrap_or_else(|| json!([])),
        "error": error,
    })
}

pub(crate) fn configured_fips_authorized_peer_npubs(config: &AppConfig) -> Vec<String> {
    let Some(account) = config.account.as_ref() else {
        return Vec::new();
    };
    let Some(snapshot) = account.app_keys.as_ref() else {
        return Vec::new();
    };

    snapshot
        .devices
        .iter()
        .filter(|device| device.pubkey != account.device_pubkey)
        .map(|device| account_npub(&device.pubkey))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn string_vec_from_json_array(value: Option<&Value>) -> Vec<String> {
    string_set_from_json_array(value).into_iter().collect()
}

pub(crate) fn string_set_from_json_array(value: Option<&Value>) -> BTreeSet<String> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

pub(crate) fn device_sync_state(
    is_current_device: bool,
    has_root: bool,
    root_available: Option<bool>,
) -> &'static str {
    if is_current_device {
        return if has_root { "local" } else { "not imported" };
    }
    match (has_root, root_available) {
        (false, _) => "waiting for root",
        (true, Some(true)) => "synced",
        (true, Some(false)) => "blocks pending",
        (true, None) => "metadata only",
    }
}

#[cfg(test)]
mod tests;
