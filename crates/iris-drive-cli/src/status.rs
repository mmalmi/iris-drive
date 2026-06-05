#[allow(clippy::wildcard_imports)]
use super::*;

mod backups;
mod network;
mod peers;

#[cfg(test)]
pub(crate) use backups::backup_target_status;
pub(crate) use backups::{backup_targets_status, configured_backup_targets_status};
use iris_drive_core::app_key_summary::{
    AppKeyConnectivity, app_key_roster_rows, primary_status_for_setup_state, primary_status_label,
    setup_label_for_setup_state, setup_state_flags, sync_status_label,
};
pub(crate) use iris_drive_core::backup_summary::backup_target_kind_label;
pub(crate) use iris_drive_core::fips_status::{
    fips_direct_devices_from_status, fips_mesh_devices_from_status,
    fips_online_devices_from_status, string_set_from_json_array, string_vec_from_json_array,
};
use iris_drive_core::provider::provider_refresh_key;
use iris_drive_core::relay_status::normalized_relay_statuses_for_relays;
pub(crate) use network::fips_network_diagnostics;
use peers::peer_statuses;

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
    let provider_refresh_key = provider_refresh_key(current_root_cid.as_deref(), &peers);
    let authorized_device_count = peers
        .iter()
        .filter(|peer| {
            peer.get("authorized")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count();
    let online_device_count = peers
        .iter()
        .filter(|peer| {
            peer.get("fips_online")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        })
        .count();
    let published_device_roots = config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .map_or(0, |drive| drive.device_roots.len());
    let fips_diagnostics = fips_network_diagnostics(&config, daemon_status.as_ref());
    let backup_targets = backup_targets_status(&config);
    let backup_target_count = backup_targets.len();
    let profile_block = status_profile_block(&config);
    let sync_status = daemon_sync_status(daemon_status.as_ref());
    println!(
        "{}",
        json!({
            "initialized": initialized,
            "config_dir": config_dir.display().to_string(),
            "current_app_key_npub": config.profile.as_ref().map(|s| pubkey_npub(&s.app_key_pubkey)),
            "profile": profile_block,
            "summary": status_summary(
                initialized,
                profile_block.as_ref(),
                authorized_device_count,
                online_device_count,
                file_count,
                visible_file_bytes,
                &sync_status,
                &provider_refresh_key,
            ),
            "drives": config.drives.iter().map(|d| json!({
                "drive_id": d.drive_id,
                "display_name": d.display_name,
                "root_scope_id": d.root_scope_id,
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
                "relay_statuses": normalized_relay_statuses(&config, daemon_status.as_ref()),
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

#[allow(clippy::too_many_arguments)]
pub(crate) fn status_summary(
    initialized: bool,
    profile: Option<&Value>,
    authorized_device_count: usize,
    online_device_count: usize,
    file_count: Option<usize>,
    visible_file_bytes: Option<u64>,
    sync_status: &str,
    provider_refresh_key: &str,
) -> Value {
    let setup_state = if initialized {
        profile
            .and_then(|profile| profile.get("authorization_state"))
            .and_then(Value::as_str)
            .unwrap_or("not_configured")
    } else {
        "not_configured"
    };
    let primary_status = primary_status_for_setup_state(setup_state);
    let setup_flags = setup_state_flags(setup_state);
    json!({
        "setup_state": setup_state,
        "setup_complete": setup_flags.setup_complete,
        "awaiting_approval": setup_flags.awaiting_approval,
        "revoked": setup_flags.revoked,
        "setup_label": setup_label_for_setup_state(setup_state),
        "primary_status": primary_status,
        "primary_status_label": primary_status_label(primary_status),
        "sync_status": sync_status,
        "sync_status_label": sync_status_label(sync_status),
        "provider_refresh_key": provider_refresh_key,
        "authorized_device_count": authorized_device_count,
        "online_device_count": online_device_count,
        "file_count": file_count.unwrap_or_default(),
        "visible_file_bytes": visible_file_bytes.unwrap_or_default(),
    })
}

pub(crate) fn daemon_sync_status(daemon_status: Option<&Value>) -> String {
    let Some(status) = daemon_status else {
        return "paused".to_owned();
    };
    if !status
        .get("running")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return "paused".to_owned();
    }
    if status
        .get("blossom_upload")
        .and_then(Value::as_object)
        .is_some_and(|upload| {
            let uploaded = upload
                .get("uploaded")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let already_present = upload
                .get("already_present")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            let total = upload
                .get("total_hashes")
                .and_then(Value::as_u64)
                .unwrap_or_default();
            total > 0 && uploaded.saturating_add(already_present) < total
        })
    {
        return "syncing".to_owned();
    }
    match status
        .get("event")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "initial_publish_error" | "auto_publish_error" | "apply_error" => "sync error",
        "initial_publish" | "auto_published" | "blossom_downloaded" => "synced",
        "drive_root" => "root synced",
        "shutdown" => "paused",
        _ => "up to date",
    }
    .to_owned()
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

pub(crate) fn status_profile_block(config: &AppConfig) -> Option<Value> {
    config.profile.as_ref().map(|state| {
        let mut state = state.clone();
        state.recompute_authorization();
        let mut output = profile_identity_json_map(&state);
        output.insert(
            "roster_size".to_string(),
            json!(state.app_keys.as_ref().map_or(0, |s| s.app_actors.len())),
        );
        output.insert(
            "user_profile".to_string(),
            config.user_profile.as_ref().map_or(Value::Null, |profile| {
                json!({
                "username": profile.username,
                "photo_path": profile.photo_path,
                })
            }),
        );
        output.insert(
            "app_key_link_request".to_string(),
            app_key_link_request_json(&state),
        );
        output.insert(
            "app_key_link_invite".to_string(),
            app_key_link_invite_json(&state),
        );
        output.insert(
            "inbound_app_key_link_requests".to_string(),
            json!(inbound_app_key_link_requests_json(&state)),
        );
        Value::Object(output)
    })
}

pub(crate) fn current_primary_root_cid(config: &AppConfig) -> Option<String> {
    config
        .profile
        .as_ref()
        .and_then(|state| {
            config
                .drive(iris_drive_core::PRIMARY_DRIVE_ID)
                .and_then(|drive| drive.device_roots.get(&state.app_key_pubkey))
                .map(|root| root.root_cid.clone())
        })
        .or_else(|| {
            config
                .drive(iris_drive_core::PRIMARY_DRIVE_ID)
                .and_then(|drive| drive.last_root_cid.clone())
        })
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
        fips.insert("online_devices".to_string(), json!([]));
        fips.insert("online_peers".to_string(), json!([]));
        fips.insert("direct_devices".to_string(), json!([]));
        fips.insert("direct_peers".to_string(), json!([]));
        fips.insert("connected_peers".to_string(), json!([]));
        fips.insert("mesh_devices".to_string(), json!([]));
        fips.insert("mesh_peers".to_string(), json!([]));
        fips.insert("peer_statuses".to_string(), json!([]));
    }
    normalize_daemon_status_for_clients(config_dir, &mut value);
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
            "current_app_key_npub",
            "provider_update_mode",
            "watch_debounce_ms",
            "mount",
            "relay_statuses",
            "embedded_hashtree",
            "browser_gateway",
            "fips_block_sync",
            "fips_block_sync_error",
            "fips",
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
    normalize_daemon_status_for_clients(config_dir, &mut payload);
    if let Some(parent) = daemon_status_path(config_dir).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&payload) {
        let _ = std::fs::write(daemon_status_path(config_dir), bytes);
    }
}

pub(crate) fn normalize_daemon_status_for_clients(config_dir: &Path, payload: &mut Value) {
    let Ok(config) = AppConfig::load_or_default(config_path_in(config_dir)) else {
        return;
    };
    let runtime_relays = string_vec_from_json_array(payload.get("relays"));
    let relay_statuses = if runtime_relays.is_empty() {
        normalized_relay_statuses(&config, Some(payload))
    } else {
        relay_statuses_json(&runtime_relays, Some(payload))
    };
    let fips = fips_network_diagnostics(&config, Some(payload));
    let sync_status = daemon_sync_status(Some(payload));
    let sync_status_label = sync_status_label(&sync_status);
    let sync_running = payload
        .get("running")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let profile_block = status_profile_block(&config);
    let (authorized_device_count, online_device_count) =
        daemon_summary_device_counts(&config, payload);
    let file_count = payload
        .get("summary")
        .and_then(|summary| summary.get("file_count"))
        .or_else(|| {
            payload
                .get("hashtree")
                .and_then(|hashtree| hashtree.get("file_count"))
        })
        .and_then(Value::as_u64)
        .and_then(|count| usize::try_from(count).ok());
    let visible_file_bytes = payload
        .get("summary")
        .and_then(|summary| summary.get("visible_file_bytes"))
        .or_else(|| {
            payload
                .get("hashtree")
                .and_then(|hashtree| hashtree.get("visible_file_bytes"))
        })
        .and_then(Value::as_u64);
    let current_root_cid = current_primary_root_cid(&config);
    let provider_refresh_key = payload
        .get("summary")
        .and_then(|summary| summary.get("provider_refresh_key"))
        .and_then(Value::as_str)
        .map_or_else(
            || provider_refresh_key(current_root_cid.as_deref(), &[]),
            ToOwned::to_owned,
        );
    let summary = status_summary(
        already_initialized(config_dir),
        profile_block.as_ref(),
        authorized_device_count,
        online_device_count,
        file_count,
        visible_file_bytes,
        &sync_status,
        &provider_refresh_key,
    );
    if let Some(object) = payload.as_object_mut() {
        object.insert("relay_statuses".to_string(), relay_statuses);
        object.insert("fips".to_string(), fips);
        object.insert("summary".to_string(), summary);
        object.insert(
            "sync".to_string(),
            json!({
                "running": sync_running,
                "status": sync_status,
                "status_label": sync_status_label,
            }),
        );
    }
}

fn daemon_summary_device_counts(config: &AppConfig, payload: &Value) -> (usize, usize) {
    let Some(account) = config.profile.as_ref() else {
        return (0, 0);
    };
    let Some(snapshot) = account.app_keys.as_ref() else {
        return (0, 0);
    };
    let fips_status = payload
        .get("fips_block_sync")
        .filter(|value| value.is_object());
    let connectivity = AppKeyConnectivity {
        online_app_keys: fips_online_devices_from_status(fips_status)
            .into_iter()
            .collect(),
        direct_app_keys: fips_direct_devices_from_status(fips_status)
            .into_iter()
            .collect(),
        mesh_app_keys: fips_mesh_devices_from_status(fips_status)
            .into_iter()
            .collect(),
        peer_statuses: BTreeMap::new(),
    };
    let daemon_running = payload
        .get("running")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let rows = app_key_roster_rows(
        &snapshot.app_actors,
        &account.app_key_pubkey,
        account.can_admin_profile(),
        daemon_running,
        &connectivity,
    );
    let online = rows.iter().filter(|row| row.is_online).count();
    (rows.len(), online)
}

pub(crate) fn normalized_relay_statuses(
    config: &AppConfig,
    daemon_status: Option<&Value>,
) -> Value {
    relay_statuses_json(&config.relays, daemon_status)
}

fn relay_statuses_json(relays: &[String], daemon_status: Option<&Value>) -> Value {
    json!(normalized_relay_statuses_for_relays(relays, daemon_status))
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
        &drive.root_scope_id,
        &drive.drive_id,
    ))
}

pub(crate) fn drive_iris_to_url_for_drive(root_scope_id: &str, drive_id: &str) -> String {
    format!(
        "{DRIVE_IRIS_TO_ORIGIN}/#/{}/{}",
        percent_encode_path_segment(root_scope_id),
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
