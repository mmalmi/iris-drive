#[allow(clippy::wildcard_imports)]
use super::*;
use iris_drive_core::daemon_liveness::{daemon_lock_path, process_is_running};
use iris_drive_core::provider::normalize_provider_path;
use iris_drive_core::relay_config::normalize_relay_url;

#[derive(Clone, Default)]
pub(crate) struct DaemonTaskSet {
    tasks: Arc<std::sync::Mutex<Vec<ManagedDaemonTask>>>,
    active_keys: Arc<std::sync::Mutex<std::collections::BTreeSet<String>>>,
}

struct ManagedDaemonTask {
    join: tokio::task::JoinHandle<()>,
    abort_inner: Option<tokio::task::AbortHandle>,
}

impl DaemonTaskSet {
    pub(crate) fn push(&self, task: tokio::task::JoinHandle<()>) {
        match self.tasks.lock() {
            Ok(mut tasks) => tasks.push(ManagedDaemonTask {
                join: task,
                abort_inner: None,
            }),
            Err(_) => task.abort(),
        }
    }

    pub(crate) fn push_keyed(&self, key: String, task: tokio::task::JoinHandle<()>) -> bool {
        let Ok(mut active_keys) = self.active_keys.lock() else {
            task.abort();
            return false;
        };
        if !active_keys.insert(key.clone()) {
            task.abort();
            return false;
        }
        drop(active_keys);

        let active_keys = self.active_keys.clone();
        let task_key = key.clone();
        let abort_inner = task.abort_handle();
        let abort_inner_on_error = abort_inner.clone();
        let join = tokio::spawn(async move {
            let _ = task.await;
            if let Ok(mut active_keys) = active_keys.lock() {
                active_keys.remove(&task_key);
            }
        });
        match self.tasks.lock() {
            Ok(mut tasks) => {
                tasks.push(ManagedDaemonTask {
                    join,
                    abort_inner: Some(abort_inner),
                });
                true
            }
            Err(_) => {
                if let Ok(mut active_keys) = self.active_keys.lock() {
                    active_keys.remove(&key);
                }
                abort_inner_on_error.abort();
                join.abort();
                false
            }
        }
    }

    async fn abort_all(&self) {
        let tasks = match self.tasks.lock() {
            Ok(mut tasks) => std::mem::take(&mut *tasks),
            Err(_) => Vec::new(),
        };
        if let Ok(mut active_keys) = self.active_keys.lock() {
            active_keys.clear();
        }
        for task in &tasks {
            if let Some(abort_inner) = &task.abort_inner {
                abort_inner.abort();
            }
            task.join.abort();
        }
        for task in tasks {
            let _ = task.join.await;
        }
    }
}

include!("daemon/runtime.rs");

#[derive(Debug, Default)]
struct ProviderRootPublishCache {
    last_config_fingerprint: Option<ConfigFileFingerprint>,
    last_current_key: ProviderRootKeySnapshot,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum ProviderRootKeySnapshot {
    #[default]
    Unknown,
    Current(Option<String>),
}

impl ProviderRootPublishCache {
    fn is_current(
        &self,
        fingerprint: &ConfigFileFingerprint,
        current_key: Option<&String>,
    ) -> bool {
        self.last_config_fingerprint.as_ref() == Some(fingerprint)
            && self.last_current_key == ProviderRootKeySnapshot::Current(current_key.cloned())
    }

    fn update(&mut self, fingerprint: ConfigFileFingerprint, current_key: Option<String>) {
        self.last_config_fingerprint = Some(fingerprint);
        self.last_current_key = ProviderRootKeySnapshot::Current(current_key);
    }
}

async fn publish_provider_root_if_changed(
    client: &nostr_sdk::Client,
    config_dir: &Path,
    last_root_key: &mut Option<String>,
    cache: &mut ProviderRootPublishCache,
    direct_roots: &mut DirectRootExchange,
    fips_blocks: Option<&FsFipsBlockSync>,
    daemon_tasks: &DaemonTaskSet,
) -> Result<Option<AppConfig>> {
    let config_path = config_path_in(config_dir);
    let config_fingerprint = config_file_fingerprint(&config_path)?;
    if cache.is_current(&config_fingerprint, last_root_key.as_ref()) {
        return Ok(None);
    }

    let updated_config = AppConfig::load_or_default(&config_path)?;
    let current_key = current_app_key_root_key(&updated_config);
    cache.update(config_fingerprint, current_key.clone());
    if current_key == *last_root_key {
        return Ok(None);
    }
    last_root_key.clone_from(&current_key);
    let Some(current_key) = current_key else {
        return Ok(Some(updated_config));
    };
    let Some(updated_state) = updated_config.profile.clone() else {
        return Ok(Some(updated_config));
    };

    direct_roots.invalidate_current_sync_events_cache();
    let direct_root_mesh_error =
        match announce_current_state_direct(direct_roots, config_dir, fips_blocks).await {
            Ok(()) => None,
            Err(error) => Some(format!("{error:#}")),
        };
    daemon_tasks.push(spawn_publish_current_state(
        client.clone(),
        config_dir.to_path_buf(),
        updated_config.clone(),
        updated_state,
        true,
        "provider_root_publish_finished",
        json!({"root_key": current_key.clone()}),
    ));
    let mut payload = json!({
            "event": "provider_root_published",
            "root_key": current_key,
            "direct_root_mesh_error": direct_root_mesh_error,
            "publish": {"queued": true, "upload_blossom": true},
    });
    if let Some(hashtree) = primary_drive_status_payload(config_dir, &updated_config).await {
        payload["hashtree"] = hashtree;
    }
    emit_daemon_status_event(config_dir, payload);

    Ok(Some(updated_config))
}

async fn primary_drive_status_payload(config_dir: &Path, config: &AppConfig) -> Option<Value> {
    let daemon = Daemon::open(config_dir).ok()?;
    let merged = iris_drive_core::primary_merged_view(daemon.tree(), config)
        .await
        .ok()?;
    Some(json!({
        "current_root_cid": crate::status::current_primary_root_cid(config),
        "file_count": merged.file_count(),
        "top_level_entries": merged.top_level_entries(),
        "visible_file_bytes": merged.view.files.iter().map(|entry| entry.size).sum::<u64>(),
    }))
}

const PROVIDER_ROOT_SAFETY_POLL_MIN_SECS: u64 = 30;

fn provider_root_poll_enabled(config_root_watch_active: bool) -> bool {
    !config_root_watch_active
}

fn provider_root_poll_period(watch_interval_secs: u64) -> std::time::Duration {
    std::time::Duration::from_secs(watch_interval_secs.max(PROVIDER_ROOT_SAFETY_POLL_MIN_SECS))
}

fn current_app_key_root_key(config: &AppConfig) -> Option<String> {
    let state = config.profile.as_ref()?;
    let mut roots = config
        .drives
        .iter()
        .filter(|drive| {
            drive.drive_id == iris_drive_core::PRIMARY_DRIVE_ID
                || drive.drive_id == iris_drive_core::calendar::CALENDAR_TREE_NAME
        })
        .filter_map(|drive| {
            let root = drive.app_key_roots.get(&state.app_key_pubkey)?;
            Some(format!(
                "{}:{}:{}",
                drive.drive_id, state.app_key_pubkey, root.root_cid
            ))
        })
        .collect::<Vec<_>>();
    roots.sort();
    (!roots.is_empty()).then(|| roots.join("|"))
}

fn merged_drive_roots_key(config: &AppConfig) -> Option<String> {
    let drive = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)?;
    let mut key = format!("drive:{}", drive.drive_id);
    for (app_key_pubkey, root) in &drive.app_key_roots {
        key.push('|');
        key.push_str(app_key_pubkey);
        key.push(':');
        key.push_str(&root.root_cid);
        key.push(':');
        key.push_str(&root.app_key_seq.to_string());
    }
    Some(key)
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RootApplyFollowupKey {
    AppKeyRoots(Vec<(String, String, u64)>),
    MergedDriveRoots(String),
}

fn update_last_provider_root_key(config_dir: &Path, last_root_key: &mut Option<String>) {
    if let Ok(config) = AppConfig::load_or_default(config_path_in(config_dir)) {
        *last_root_key = current_app_key_root_key(&config);
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
            .app_key_roots
            .iter()
            .filter(|(_, root)| root.root_cid == root_cid)
            .map(|(device, root)| (device.clone(), root.root_cid.clone(), root.app_key_seq))
            .collect::<Vec<_>>();
        if !roots.is_empty() {
            return Some(RootApplyFollowupKey::AppKeyRoots(roots));
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
        RootApplyFollowupKey::AppKeyRoots(roots) => {
            let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID) else {
                return true;
            };
            roots.iter().any(|(device, root_cid, app_key_seq)| {
                drive.app_key_roots.get(device).is_none_or(|root| {
                    root.root_cid != *root_cid || root.app_key_seq != *app_key_seq
                })
            })
        }
        RootApplyFollowupKey::MergedDriveRoots(expected) => {
            merged_drive_roots_key(&config).as_deref() != Some(expected.as_str())
        }
    }
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
    daemon_tasks: &DaemonTaskSet,
) -> Result<()> {
    let imported_visible_root = visible_root.clone();
    let config_lock = ConfigMutationLock::acquire(config_dir).await?;
    let import =
        import_mount_root_for_publish(config_dir, visible_root, mount_tombstone_base.clone(), None)
            .await?;
    drop(config_lock);
    if let Some(import) = import {
        publish_imported_mount_root(
            client,
            config_dir,
            import,
            direct_roots,
            fips_blocks,
            daemon_tasks,
        )
        .await?;
    }
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
    let provider_signal_path = iris_drive_core::paths::provider_root_signal_path_in(config_dir);
    let parent = config_path.parent().unwrap_or(config_dir).to_path_buf();
    std::fs::create_dir_all(&parent)
        .with_context(|| format!("creating config directory {}", parent.display()))?;
    if !provider_signal_path.exists() {
        std::fs::write(&provider_signal_path, b"").with_context(|| {
            format!(
                "creating provider signal {}",
                provider_signal_path.display()
            )
        })?;
    }

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let callback_tx = tx.clone();
    let watched_parent = parent.clone();
    let mut watcher = notify::recommended_watcher(move |result| match result {
        Ok(event) => {
            if event_touches_config_root(&event, &watched_parent) {
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
    watcher
        .watch(&provider_signal_path, RecursiveMode::NonRecursive)
        .with_context(|| {
            format!(
                "watching provider signal {}",
                provider_signal_path.display()
            )
        })?;

    Ok((
        rx,
        watcher,
        json!({
            "watching": true,
            "path": config_path.display().to_string(),
            "provider_signal_path": provider_signal_path.display().to_string(),
            "provider_signal_file_watch": true,
        }),
    ))
}

async fn start_provider_root_wake_listener(
    config_dir: &Path,
) -> Result<(
    tokio::sync::mpsc::UnboundedReceiver<Option<Value>>,
    tokio::task::JoinHandle<()>,
    Value,
)> {
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .context("binding provider root wake listener")?;
    let port = listener
        .local_addr()
        .context("reading provider root wake listener address")?
        .port();
    let wake_path = iris_drive_core::paths::provider_root_wake_path_in(config_dir);
    if let Some(parent) = wake_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    std::fs::write(&wake_path, serde_json::to_vec(&json!({ "port": port }))?).with_context(
        || {
            format!(
                "writing provider root wake endpoint {}",
                wake_path.display()
            )
        },
    )?;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let task = tokio::spawn(async move {
        use tokio::io::AsyncReadExt as _;

        while let Ok((mut stream, _addr)) = listener.accept().await {
            let mut bytes = vec![0_u8; 4096];
            let payload = match tokio::time::timeout(
                std::time::Duration::from_millis(200),
                stream.read(&mut bytes),
            )
            .await
            {
                Ok(Ok(len)) if len > 0 => serde_json::from_slice::<Value>(&bytes[..len]).ok(),
                _ => None,
            };
            let _ = tx.send(payload);
        }
    });

    Ok((
        rx,
        task,
        json!({
            "running": true,
            "bind": format!("127.0.0.1:{port}"),
            "path": wake_path.display().to_string(),
        }),
    ))
}

fn provider_root_wake_status_payload(wake_payload: &Value) -> Option<Value> {
    let file_count = wake_payload.get("file_count").and_then(Value::as_u64)?;
    let mut hashtree = json!({ "file_count": file_count });
    if let Some(root_cid) = wake_payload.get("root_cid").and_then(Value::as_str) {
        hashtree["current_root_cid"] = json!(root_cid);
    }
    if let Some(top_level_entries) = wake_payload
        .get("top_level_entries")
        .and_then(Value::as_u64)
    {
        hashtree["top_level_entries"] = json!(top_level_entries);
    }
    Some(json!({
        "event": "provider_root_local_update",
        "root_cid": wake_payload.get("root_cid").cloned(),
        "hashtree": hashtree,
    }))
}

fn drain_latest_provider_root_wake_payload(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<Option<Value>>,
    mut latest: Option<Value>,
) -> Option<Value> {
    while let Ok(next) = rx.try_recv() {
        if next.is_some() {
            latest = next;
        }
    }
    latest
}

fn event_touches_config_root(event: &notify::Event, config_dir: &Path) -> bool {
    event.paths.is_empty()
        || event.paths.iter().any(|path| {
            paths_refer_to_same_file(path, config_dir)
                || path
                    .parent()
                    .is_some_and(|parent| paths_refer_to_same_file(parent, config_dir))
        })
}

#[cfg(test)]
fn event_touches_path(event: &notify::Event, target: &Path) -> bool {
    let parent = target.parent();
    event.paths.iter().any(|path| {
        paths_refer_to_same_file(path, target)
            || parent.is_some_and(|parent| paths_refer_to_same_file(path, parent))
    })
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
) -> tokio::task::JoinHandle<()> {
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
        let (fips_status, fips_block_sync_error) = match tokio::time::timeout(
            std::time::Duration::from_secs(STATUS_PROBE_TIMEOUT_SECS),
            fips_block_sync_status(fips_blocks.as_deref()),
        )
        .await
        {
            Ok(status) => (status, Value::Null),
            Err(_) => (
                Some(json!({"status": "timeout"})),
                json!("FIPS status probe timed out"),
            ),
        };
        let status = json!({
            "event": "relay_statuses",
            "relay_statuses": relay_statuses,
            "fips_block_sync": fips_status,
            "fips_block_sync_error": fips_block_sync_error,
        });
        let status = write_daemon_status(&config_dir, status);
        println!("{status}");
    })
}

pub(crate) async fn relay_status_payload(client: &nostr_sdk::Client) -> Vec<serde_json::Value> {
    let relays = client.relays().await;
    let mut payload = Vec::with_capacity(relays.len());
    for (url, relay) in relays {
        let Ok(url) = normalize_relay_url(url.as_str()) else {
            continue;
        };
        payload.push(json!({
            "url": url,
            "status": relay_status_label(relay.status()),
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
        RelayStatus::Banned => "banned",
        RelayStatus::Sleeping => "sleeping",
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
        let path = daemon_lock_path(config_dir);
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

#[derive(Debug)]
struct ConfigMutationLockTimeout {
    path: PathBuf,
}

impl std::fmt::Display for ConfigMutationLockTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "timed out waiting for config mutation lock {}",
            self.path.display()
        )
    }
}

impl std::error::Error for ConfigMutationLockTimeout {}

impl ConfigMutationLock {
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);
    const WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    const STALE_AFTER: std::time::Duration = std::time::Duration::from_mins(2);

    pub(crate) async fn acquire(config_dir: &Path) -> Result<Self> {
        Self::acquire_with_timeout(config_dir, Self::WAIT_TIMEOUT).await
    }

    async fn acquire_for_background<F>(config_dir: &Path, is_stale: F) -> Result<Option<Self>>
    where
        F: FnMut() -> bool,
    {
        let retry_delays = [
            std::time::Duration::from_millis(250),
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(4),
        ];
        Self::acquire_for_background_with_options(
            config_dir,
            is_stale,
            Self::WAIT_TIMEOUT,
            &retry_delays,
        )
        .await
    }

    async fn acquire_for_background_with_options<F>(
        config_dir: &Path,
        mut is_stale: F,
        wait_timeout: std::time::Duration,
        retry_delays: &[std::time::Duration],
    ) -> Result<Option<Self>>
    where
        F: FnMut() -> bool,
    {
        for retry_delay in std::iter::once(std::time::Duration::ZERO).chain(
            retry_delays
                .iter()
                .copied()
                .filter(|delay| !delay.is_zero()),
        ) {
            if retry_delay > std::time::Duration::ZERO {
                tokio::time::sleep(retry_delay).await;
            }
            if is_stale() {
                return Ok(None);
            }
            match Self::acquire_with_timeout(config_dir, wait_timeout).await {
                Ok(lock) => {
                    if is_stale() {
                        return Ok(None);
                    }
                    return Ok(Some(lock));
                }
                Err(error) if error.downcast_ref::<ConfigMutationLockTimeout>().is_some() => {}
                Err(error) => return Err(error),
            }
        }
        if is_stale() {
            return Ok(None);
        }
        Self::acquire_with_timeout(config_dir, wait_timeout)
            .await
            .map(Some)
    }

    async fn acquire_with_timeout(
        config_dir: &Path,
        wait_timeout: std::time::Duration,
    ) -> Result<Self> {
        std::fs::create_dir_all(config_dir)
            .with_context(|| format!("creating config dir {}", config_dir.display()))?;
        let path = config_dir.join("config-mutation.lock");
        let started = std::time::Instant::now();

        loop {
            match Self::try_create(&path) {
                Ok(lock) => return Ok(lock),
                Err(error) if Self::lock_create_error_is_contention(&path, &error) => {
                    Self::remove_stale_lock(&path);
                    if started.elapsed() >= wait_timeout {
                        return Err(ConfigMutationLockTimeout { path }.into());
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

    fn lock_create_error_is_contention(path: &Path, error: &std::io::Error) -> bool {
        error.kind() == std::io::ErrorKind::AlreadyExists
            || (error.kind() == std::io::ErrorKind::PermissionDenied && path.exists())
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

pub(crate) async fn parent_exit_signal(service_mode: bool) {
    if service_mode {
        std::future::pending::<()>().await;
        return;
    }
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

pub(crate) fn authorized_app_key_pubkeys(state: &ProfileState) -> Vec<String> {
    state.active_root_writer_app_key_pubkeys()
}

pub(crate) fn files_root_apply_label(
    outcome: &iris_drive_core::relay_sync::FilesRootApply,
) -> &'static str {
    match outcome {
        iris_drive_core::relay_sync::FilesRootApply::NotOurAppKey => "not_our_app_key",
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
