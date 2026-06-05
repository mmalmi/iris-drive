#[allow(clippy::wildcard_imports)]
use super::*;
use iris_drive_core::provider::normalize_provider_path;
use iris_drive_core::relay_config::normalize_relay_url;

include!("daemon/runtime.rs");
async fn publish_provider_root_if_changed(
    client: &nostr_sdk::Client,
    config_dir: &Path,
    last_root_key: &mut Option<String>,
    direct_roots: &mut DirectRootExchange,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<Option<AppConfig>> {
    let updated_config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let current_key = current_device_root_key(&updated_config);
    if current_key == *last_root_key {
        return Ok(None);
    }
    last_root_key.clone_from(&current_key);
    let Some(current_key) = current_key else {
        return Ok(Some(updated_config));
    };
    let Some(updated_state) = updated_config.account.clone() else {
        return Ok(Some(updated_config));
    };

    let direct_root_mesh_error =
        match announce_current_state_direct(direct_roots, config_dir, fips_blocks).await {
            Ok(()) => None,
            Err(error) => Some(format!("{error:#}")),
        };
    spawn_publish_current_state(
        client.clone(),
        config_dir.to_path_buf(),
        updated_config,
        updated_state,
        true,
        "provider_root_publish_finished",
        json!({"root_key": current_key.clone()}),
    );
    emit_daemon_status_event(
        config_dir,
        json!({
            "event": "provider_root_published",
            "root_key": current_key,
            "direct_root_mesh_error": direct_root_mesh_error,
            "publish": {"queued": true, "upload_blossom": true},
        }),
    );

    Ok(Some(AppConfig::load_or_default(config_path_in(
        config_dir,
    ))?))
}

fn current_device_root_key(config: &AppConfig) -> Option<String> {
    let state = config.account.as_ref()?;
    let drive = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)?;
    let root = drive.device_roots.get(&state.device_pubkey)?;
    Some(format!(
        "{}:{}:{}",
        drive.drive_id, state.device_pubkey, root.root_cid
    ))
}

fn merged_drive_roots_key(config: &AppConfig) -> Option<String> {
    let drive = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)?;
    let mut key = format!("drive:{}", drive.drive_id);
    for (device_pubkey, root) in &drive.device_roots {
        key.push('|');
        key.push_str(device_pubkey);
        key.push(':');
        key.push_str(&root.root_cid);
        key.push(':');
        key.push_str(&root.device_seq.to_string());
    }
    Some(key)
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RootApplyFollowupKey {
    DeviceRoots(Vec<(String, String, u64)>),
    MergedDriveRoots(String),
}

fn update_last_provider_root_key(config_dir: &Path, last_root_key: &mut Option<String>) {
    if let Ok(config) = AppConfig::load_or_default(config_path_in(config_dir)) {
        *last_root_key = current_device_root_key(&config);
    }
}

fn root_apply_followup_key(
    config: &AppConfig,
    root_cid_to_pull: Option<&str>,
    should_refresh_projection: bool,
) -> Option<RootApplyFollowupKey> {
    if let Some(root_cid) = root_cid_to_pull
        && let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)
    {
        let roots = drive
            .device_roots
            .iter()
            .filter(|(_, root)| root.root_cid == root_cid)
            .map(|(device, root)| (device.clone(), root.root_cid.clone(), root.device_seq))
            .collect::<Vec<_>>();
        if !roots.is_empty() {
            return Some(RootApplyFollowupKey::DeviceRoots(roots));
        }
    }
    if should_refresh_projection {
        return merged_drive_roots_key(config).map(RootApplyFollowupKey::MergedDriveRoots);
    }
    None
}

fn root_apply_followup_is_stale(
    config_dir: &Path,
    expected_root_key: Option<&RootApplyFollowupKey>,
) -> bool {
    let Some(expected_root_key) = expected_root_key else {
        return false;
    };
    let Ok(config) = AppConfig::load_or_default(config_path_in(config_dir)) else {
        return false;
    };
    match expected_root_key {
        RootApplyFollowupKey::DeviceRoots(roots) => {
            let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID) else {
                return true;
            };
            roots.iter().any(|(device, root_cid, device_seq)| {
                drive
                    .device_roots
                    .get(device)
                    .is_none_or(|root| root.root_cid != *root_cid || root.device_seq != *device_seq)
            })
        }
        RootApplyFollowupKey::MergedDriveRoots(expected) => {
            merged_drive_roots_key(&config).as_deref() != Some(expected.as_str())
        }
    }
}

fn root_apply_followup_key_label(expected_root_key: Option<&RootApplyFollowupKey>) -> String {
    expected_root_key.map_or_else(|| "none".to_string(), |key| format!("{key:?}"))
}

fn root_update_debounce_duration(watch_debounce_ms: u64) -> std::time::Duration {
    std::time::Duration::from_millis(watch_debounce_ms.max(ROOT_UPDATE_THROTTLE_MS))
}

async fn drain_latest_mount_root_update(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<Cid>,
    debounce: std::time::Duration,
    first: Option<Cid>,
) -> Option<Cid> {
    let mut latest = first;
    while let Ok(next) = rx.try_recv() {
        latest = Some(next);
    }
    if latest.is_some() {
        tokio::time::sleep(debounce).await;
        while let Ok(next) = rx.try_recv() {
            latest = Some(next);
        }
    }
    latest
}

async fn import_mount_visible_root_update(
    client: &nostr_sdk::Client,
    config_dir: &Path,
    visible_root: Cid,
    mount_tombstone_base: &mut Option<Cid>,
    direct_roots: &mut DirectRootExchange,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<()> {
    let imported_visible_root = visible_root.clone();
    let _config_lock = ConfigMutationLock::acquire(config_dir).await?;
    import_mount_root_and_publish(
        client,
        config_dir,
        visible_root,
        mount_tombstone_base.clone(),
        direct_roots,
        fips_blocks,
    )
    .await?;
    *mount_tombstone_base = Some(imported_visible_root);
    Ok(())
}

fn start_config_root_watch(
    config_dir: &Path,
) -> Result<(
    tokio::sync::mpsc::UnboundedReceiver<()>,
    notify::RecommendedWatcher,
    Value,
)> {
    use notify::{RecursiveMode, Watcher};

    let config_path = config_path_in(config_dir);
    let parent = config_path.parent().unwrap_or(config_dir).to_path_buf();
    std::fs::create_dir_all(&parent)
        .with_context(|| format!("creating config directory {}", parent.display()))?;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let callback_tx = tx.clone();
    let watched_config = config_path.clone();
    let mut watcher = notify::recommended_watcher(move |result| match result {
        Ok(event) => {
            if event_touches_path(&event, &watched_config) {
                let _ = callback_tx.send(());
            }
        }
        Err(error) => {
            eprintln!("config root watch error: {error:#}");
        }
    })
    .context("creating config root watcher")?;
    watcher
        .watch(&parent, RecursiveMode::NonRecursive)
        .with_context(|| format!("watching config directory {}", parent.display()))?;

    Ok((
        rx,
        watcher,
        json!({
            "watching": true,
            "path": config_path.display().to_string(),
        }),
    ))
}

fn event_touches_path(event: &notify::Event, target: &Path) -> bool {
    event
        .paths
        .iter()
        .any(|path| paths_refer_to_same_file(path, target))
}

fn paths_refer_to_same_file(path: &Path, target: &Path) -> bool {
    if path == target {
        return true;
    }
    path.file_name() == target.file_name()
        && path
            .parent()
            .zip(target.parent())
            .is_some_and(|(a, b)| a == b)
}

include!("daemon/windows_cloud_watch.rs");
include!("daemon/windows_cloud_apply.rs");
include!("daemon/windows_cloud_state.rs");

pub(crate) fn spawn_status_probe(
    client: nostr_sdk::Client,
    config_dir: PathBuf,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
) {
    tokio::spawn(async move {
        let relay_statuses = match tokio::time::timeout(
            std::time::Duration::from_secs(STATUS_PROBE_TIMEOUT_SECS),
            relay_status_payload(&client),
        )
        .await
        {
            Ok(statuses) => statuses,
            Err(_) => vec![json!({"url": "*", "status": "timeout"})],
        };
        let fips_status = match tokio::time::timeout(
            std::time::Duration::from_secs(STATUS_PROBE_TIMEOUT_SECS),
            fips_block_sync_status(fips_blocks.as_deref()),
        )
        .await
        {
            Ok(status) => status,
            Err(_) => Some(json!({"status": "timeout"})),
        };
        let mut status = json!({
            "event": "relay_statuses",
            "relay_statuses": relay_statuses,
            "fips_block_sync": fips_status,
        });
        normalize_daemon_status_for_clients(&config_dir, &mut status);
        write_daemon_status(&config_dir, status.clone());
        println!("{status}");
    });
}

pub(crate) async fn relay_status_payload(client: &nostr_sdk::Client) -> Vec<serde_json::Value> {
    let relays = client.relays().await;
    let mut payload = Vec::with_capacity(relays.len());
    for (url, relay) in relays {
        let Ok(url) = normalize_relay_url(url.as_ref()) else {
            continue;
        };
        payload.push(json!({
            "url": url,
            "status": relay_status_label(relay.status().await),
        }));
    }
    payload
}

pub(crate) async fn fips_block_sync_status(sync: Option<&FsFipsBlockSync>) -> Option<Value> {
    let sync = sync?;
    let transport = sync.transport_settings();
    let direct_devices = sync.connected_peer_ids().await;
    let mesh_devices = sync.mesh_peer_ids().await;
    let online_devices = fips_online_device_ids(&direct_devices, &mesh_devices);
    Some(json!({
        "endpoint_npub": sync.endpoint_npub(),
        "discovery_scope": sync.discovery_scope(),
        "nostr_discovery_app": sync.nostr_discovery_app(),
        "udp_enabled": transport.enable_udp,
        "udp_bind_addr": transport.udp_bind_addr.as_deref(),
        "udp_public": transport.udp_public,
        "udp_external_addr": transport.udp_external_addr.as_deref(),
        "webrtc_enabled": transport.enable_webrtc,
        "webrtc_max_connections": transport.webrtc_max_connections,
        "open_discovery_max_pending": transport.open_discovery_max_pending,
        "online_devices": online_devices.clone(),
        "online_peers": online_devices,
        "mesh_peer_count": mesh_devices.len(),
        "mesh_devices": mesh_devices.clone(),
        "mesh_peers": mesh_devices,
        "authorized_peers": sync.authorized_peer_ids().await,
        "direct_devices": direct_devices.clone(),
        "direct_peers": direct_devices.clone(),
        "connected_peers": direct_devices,
        "peer_statuses": sync.fips_peer_statuses().await,
        "relay_statuses": sync.fips_relay_statuses().await,
    }))
}

fn fips_online_device_ids(direct_devices: &[String], mesh_devices: &[String]) -> Vec<String> {
    direct_devices
        .iter()
        .chain(mesh_devices)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn relay_status_label(status: RelayStatus) -> &'static str {
    match status {
        RelayStatus::Initialized | RelayStatus::Pending | RelayStatus::Connecting => "connecting",
        RelayStatus::Connected => "connected",
        RelayStatus::Disconnected => "offline",
        RelayStatus::Terminated => "terminated",
    }
}

pub(crate) struct DaemonProcessLock {
    path: PathBuf,
}

impl DaemonProcessLock {
    pub(crate) fn acquire(config_dir: &std::path::Path) -> Result<Self> {
        std::fs::create_dir_all(config_dir)
            .with_context(|| format!("creating config dir {}", config_dir.display()))?;
        let path = config_dir.join("daemon.lock");
        match Self::try_create(&path) {
            Ok(lock) => return Ok(lock),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("creating daemon lock {}", path.display()));
            }
        }

        if let Ok(contents) = std::fs::read_to_string(&path)
            && let Ok(pid) = contents.trim().parse::<u32>()
            && !process_is_running(pid)
        {
            let _ = std::fs::remove_file(&path);
            return Self::try_create(&path)
                .with_context(|| format!("replacing stale daemon lock {}", path.display()));
        }

        Err(anyhow::anyhow!(
            "iris-drive daemon already appears to be running for {}",
            config_dir.display()
        ))
    }

    fn try_create(path: &Path) -> std::io::Result<Self> {
        use std::io::Write;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        writeln!(file, "{}", std::process::id())?;
        Ok(Self {
            path: path.to_path_buf(),
        })
    }
}

impl Drop for DaemonProcessLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) struct ConfigMutationLock {
    path: PathBuf,
}

impl ConfigMutationLock {
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);
    const WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    const STALE_AFTER: std::time::Duration = std::time::Duration::from_mins(2);

    pub(crate) async fn acquire(config_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(config_dir)
            .with_context(|| format!("creating config dir {}", config_dir.display()))?;
        let path = config_dir.join("config-mutation.lock");
        let started = std::time::Instant::now();

        loop {
            match Self::try_create(&path) {
                Ok(lock) => return Ok(lock),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    Self::remove_stale_lock(&path);
                    if started.elapsed() >= Self::WAIT_TIMEOUT {
                        return Err(anyhow::anyhow!(
                            "timed out waiting for config mutation lock {}",
                            path.display()
                        ));
                    }
                    tokio::time::sleep(Self::POLL_INTERVAL).await;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("creating config mutation lock {}", path.display())
                    });
                }
            }
        }
    }

    fn try_create(path: &Path) -> std::io::Result<Self> {
        use std::io::Write;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        writeln!(file, "{}", std::process::id())?;
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    fn remove_stale_lock(path: &Path) {
        if let Ok(contents) = std::fs::read_to_string(path)
            && let Ok(pid) = contents.trim().parse::<u32>()
            && !process_is_running(pid)
        {
            let _ = std::fs::remove_file(path);
            return;
        }

        if let Ok(metadata) = std::fs::metadata(path)
            && let Ok(modified) = metadata.modified()
            && modified
                .elapsed()
                .is_ok_and(|elapsed| elapsed >= Self::STALE_AFTER)
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for ConfigMutationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
pub(crate) fn process_is_running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    std::process::Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(windows)]
pub(crate) fn process_is_running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    let filter = format!("PID eq {pid}");
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().any(|line| {
        let mut fields = line.split(',');
        let _image = fields.next();
        fields
            .next()
            .map(|value| value.trim_matches('"').trim() == pid.to_string())
            .unwrap_or(false)
    })
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn process_is_running(pid: u32) -> bool {
    pid == std::process::id()
}

pub(crate) async fn parent_exit_signal() {
    let Some(parent_pid) = std::env::var("IRIS_DRIVE_PARENT_PID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
    else {
        std::future::pending::<()>().await;
        return;
    };

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if !process_is_running(parent_pid) {
            return;
        }
    }
}

include!("daemon/apply_and_blocks.rs");

pub(crate) fn record_block_sync(
    config_dir: &Path,
    root_cid: &str,
    transport: &str,
    report: &DownloadReport,
) {
    let value = json!({
        "root_cid": root_cid,
        "transport": transport,
        "updated_at": unix_now(),
        "total_hashes": report.total_hashes,
        "fetched": report.fetched,
        "already_local": report.already_local,
    });
    merge_daemon_status(config_dir, |status| {
        status.insert("last_block_sync".to_string(), value.clone());
        let entry = status
            .entry("block_sync_by_root".to_string())
            .or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({});
        }
        if let Some(map) = entry.as_object_mut() {
            map.insert(root_cid.to_string(), value);
        }
    });
}

pub(crate) fn pick_relays(config: &AppConfig, override_list: &[String]) -> Vec<String> {
    if override_list.is_empty() {
        config.relays.clone()
    } else {
        override_list.to_vec()
    }
}

pub(crate) fn authorized_device_pubkeys(state: &AccountState) -> Vec<String> {
    let mut app_actors: Vec<String> = state
        .app_keys
        .as_ref()
        .map(|snap| snap.app_actors.iter().map(|d| d.pubkey.clone()).collect())
        .unwrap_or_default();
    if !app_actors.contains(&state.device_pubkey) {
        app_actors.push(state.device_pubkey.clone());
    }
    app_actors
}

pub(crate) fn files_root_apply_label(
    outcome: &iris_drive_core::relay_sync::FilesRootApply,
) -> &'static str {
    match outcome {
        iris_drive_core::relay_sync::FilesRootApply::NotOurOwner => "not_our_owner",
        iris_drive_core::relay_sync::FilesRootApply::UnknownDrive => "unknown_drive",
        iris_drive_core::relay_sync::FilesRootApply::StaleTimestamp => "stale_timestamp",
        iris_drive_core::relay_sync::FilesRootApply::Applied => "applied",
    }
}

#[cfg(test)]
mod tests {
    include!("daemon/tests_part1.rs");
    include!("daemon/tests_part2.rs");
}
