#[allow(clippy::wildcard_imports)]
use super::*;
use iris_drive_core::daemon_liveness::{daemon_lock_path, process_is_running};
use iris_drive_core::provider::normalize_provider_path;
use iris_drive_core::relay_config::normalize_relay_url;

use crate::provider_staging::{clear_provider_staging, read_provider_staging};

const PARENT_EXIT_POLL_SECS: u64 = 60;

#[derive(Clone, Default)]
pub(crate) struct DaemonTaskSet {
    tasks: Arc<std::sync::Mutex<Vec<ManagedDaemonTask>>>,
    keyed_tasks: Arc<std::sync::Mutex<KeyedDaemonTasks>>,
}

struct ManagedDaemonTask {
    join: tokio::task::JoinHandle<()>,
    abort_inner: Option<tokio::task::AbortHandle>,
}

#[derive(Default)]
struct KeyedDaemonTasks {
    next_id: u64,
    active: std::collections::BTreeMap<String, ActiveKeyedTask>,
    groups: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
}

struct ActiveKeyedTask {
    id: u64,
    abort: tokio::task::AbortHandle,
    group: Option<String>,
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
        self.push_keyed_inner(key, None, task)
    }

    pub(crate) fn push_keyed_replacing_group(
        &self,
        key: String,
        group: String,
        task: tokio::task::JoinHandle<()>,
    ) -> bool {
        self.push_keyed_inner(key, Some(group), task)
    }

    // The key is retained by both the completion task and the synchronous
    // task-registration failure path, so an owned value avoids caller lifetimes.
    #[allow(clippy::needless_pass_by_value)]
    fn push_keyed_inner(
        &self,
        key: String,
        group: Option<String>,
        task: tokio::task::JoinHandle<()>,
    ) -> bool {
        let abort_inner = task.abort_handle();
        let Ok(mut keyed_tasks) = self.keyed_tasks.lock() else {
            task.abort();
            return false;
        };
        if keyed_tasks.active.contains_key(&key) {
            task.abort();
            return false;
        }
        if let Some(group) = group.as_ref() {
            let stale_keys = keyed_tasks
                .groups
                .get(group)
                .into_iter()
                .flat_map(|keys| keys.iter().cloned())
                .collect::<Vec<_>>();
            for stale_key in stale_keys {
                if stale_key == key {
                    continue;
                }
                if let Some(stale_task) = keyed_tasks.active.remove(&stale_key) {
                    if let Some(stale_group) = stale_task.group.as_ref()
                        && let Some(keys) = keyed_tasks.groups.get_mut(stale_group)
                    {
                        keys.remove(&stale_key);
                        if keys.is_empty() {
                            keyed_tasks.groups.remove(stale_group);
                        }
                    }
                    stale_task.abort.abort();
                }
            }
        }
        keyed_tasks.next_id = keyed_tasks.next_id.wrapping_add(1);
        let task_id = keyed_tasks.next_id;
        if let Some(group) = group.as_ref() {
            keyed_tasks
                .groups
                .entry(group.clone())
                .or_default()
                .insert(key.clone());
        }
        keyed_tasks.active.insert(
            key.clone(),
            ActiveKeyedTask {
                id: task_id,
                abort: abort_inner.clone(),
                group,
            },
        );
        drop(keyed_tasks);

        let keyed_tasks = self.keyed_tasks.clone();
        let task_key = key.clone();
        let abort_inner_on_error = abort_inner.clone();
        let join = tokio::spawn(async move {
            let _ = task.await;
            if let Ok(mut keyed_tasks) = keyed_tasks.lock()
                && keyed_tasks
                    .active
                    .get(&task_key)
                    .is_some_and(|active| active.id == task_id)
            {
                let group = keyed_tasks
                    .active
                    .get(&task_key)
                    .and_then(|active| active.group.clone());
                keyed_tasks.active.remove(&task_key);
                if let Some(group) = group
                    && let Some(keys) = keyed_tasks.groups.get_mut(&group)
                {
                    keys.remove(&task_key);
                    if keys.is_empty() {
                        keyed_tasks.groups.remove(&group);
                    }
                }
            }
        });
        if let Ok(mut tasks) = self.tasks.lock() {
            tasks.push(ManagedDaemonTask {
                join,
                abort_inner: Some(abort_inner),
            });
            true
        } else {
            if let Ok(mut keyed_tasks) = self.keyed_tasks.lock()
                && keyed_tasks
                    .active
                    .get(&key)
                    .is_some_and(|active| active.id == task_id)
            {
                let group = keyed_tasks
                    .active
                    .get(&key)
                    .and_then(|active| active.group.clone());
                keyed_tasks.active.remove(&key);
                if let Some(group) = group
                    && let Some(keys) = keyed_tasks.groups.get_mut(&group)
                {
                    keys.remove(&key);
                    if keys.is_empty() {
                        keyed_tasks.groups.remove(&group);
                    }
                }
            }
            abort_inner_on_error.abort();
            join.abort();
            false
        }
    }

    async fn abort_all(&self) {
        let tasks = match self.tasks.lock() {
            Ok(mut tasks) => std::mem::take(&mut *tasks),
            Err(_) => Vec::new(),
        };
        if let Ok(mut keyed_tasks) = self.keyed_tasks.lock() {
            for active in keyed_tasks.active.values() {
                active.abort.abort();
            }
            keyed_tasks.active.clear();
            keyed_tasks.groups.clear();
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
include!("daemon/gateway_runtime.rs");

#[derive(Debug, Default)]
struct ProviderRootPublishCache {
    last_config_fingerprint: Option<ConfigFileFingerprint>,
    last_publish_key: ProviderRootPublishKeySnapshot,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum ProviderRootPublishKeySnapshot {
    #[default]
    Unknown,
    Current(ProviderRootPublishKey),
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ProviderRootPublishKey {
    current_root_key: Option<String>,
    profile_roster_key: Option<String>,
}

impl ProviderRootPublishCache {
    fn fingerprint_matches(&self, fingerprint: &ConfigFileFingerprint) -> bool {
        self.last_config_fingerprint.as_ref() == Some(fingerprint)
    }

    fn is_current(
        &self,
        fingerprint: &ConfigFileFingerprint,
        publish_key: &ProviderRootPublishKey,
    ) -> bool {
        self.last_config_fingerprint.as_ref() == Some(fingerprint)
            && self.last_publish_key == ProviderRootPublishKeySnapshot::Current(publish_key.clone())
    }

    fn publish_key_matches(&self, publish_key: &ProviderRootPublishKey) -> bool {
        self.last_publish_key == ProviderRootPublishKeySnapshot::Current(publish_key.clone())
    }

    fn update(&mut self, fingerprint: ConfigFileFingerprint, publish_key: ProviderRootPublishKey) {
        self.last_config_fingerprint = Some(fingerprint);
        self.last_publish_key = ProviderRootPublishKeySnapshot::Current(publish_key);
    }
}

impl ProviderRootPublishKey {
    fn from_config(config: &AppConfig, current_root_key: Option<String>) -> Self {
        Self {
            current_root_key,
            profile_roster_key: profile_roster_publish_key(config),
        }
    }

    const fn has_publishable_state(&self) -> bool {
        self.current_root_key.is_some() || self.profile_roster_key.is_some()
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
    if cache.fingerprint_matches(&config_fingerprint) {
        return Ok(None);
    }
    let updated_config = AppConfig::load_or_default(&config_path)?;
    if let Some(sync) = fips_blocks {
        sync.refresh_authorized_peers(&updated_config).await;
    }
    let current_key = current_app_key_root_key(&updated_config);
    let publish_key = ProviderRootPublishKey::from_config(&updated_config, current_key.clone());
    if cache.is_current(&config_fingerprint, &publish_key) {
        return Ok(None);
    }
    if cache.publish_key_matches(&publish_key) {
        cache.update(config_fingerprint, publish_key);
        last_root_key.clone_from(&current_key);
        return Ok(None);
    }
    cache.update(config_fingerprint, publish_key.clone());
    last_root_key.clone_from(&current_key);
    if !publish_key.has_publishable_state() {
        return Ok(Some(updated_config));
    }
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
            "profile_roster_key": publish_key.profile_roster_key,
            "direct_root_mesh_error": direct_root_mesh_error,
            "publish": {"queued": true, "upload_blossom": true},
    });
    if let Some(hashtree) = primary_drive_status_payload(config_dir, &updated_config).await {
        payload["hashtree"] = hashtree;
    }
    emit_daemon_status_event(config_dir, payload);

    Ok(Some(updated_config))
}

fn profile_roster_publish_key(config: &AppConfig) -> Option<String> {
    let profile = config.profile.as_ref()?;
    if profile.profile_roster_ops.is_empty() {
        return None;
    }
    let mut op_ids = profile
        .profile_roster_ops
        .iter()
        .map(|op| op.op_id.as_str())
        .collect::<Vec<_>>();
    op_ids.sort_unstable();
    Some(format!("{}:{}", profile.profile_id, op_ids.join(",")))
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

async fn import_staged_provider_root(
    config_dir: &Path,
) -> Result<Option<iris_drive_core::daemon::ImportReport>> {
    let _config_lock = ConfigMutationLock::acquire(config_dir).await?;
    let Some(staged) = read_provider_staging(config_dir)? else {
        return Ok(None);
    };
    let root = staged.root()?;
    let tombstone_base_root = staged.tombstone_base_root()?;
    let tombstone_paths = staged.tombstone_paths;
    let mut daemon = Daemon::open(config_dir)
        .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
    let report = daemon
        .import_visible_root_with_tombstone_base_and_paths(
            root,
            tombstone_base_root,
            Some(&tombstone_paths),
        )
        .await?;
    clear_provider_staging(config_dir)?;
    Ok(Some(report))
}

fn provider_root_import_status_payload(
    event: &str,
    report: &iris_drive_core::daemon::ImportReport,
) -> Value {
    json!({
        "event": event,
        "root_cid": report.root_cid.clone(),
        "hashtree": {
            "current_root_cid": report.root_cid.clone(),
            "file_count": report.file_count,
            "top_level_entries": report.top_level_entries,
        },
    })
}

async fn handle_provider_root_wake_payload(config_dir: &Path, wake_payload: &Value, trigger: &str) {
    if provider_root_wake_payload_is_staged(wake_payload) {
        match import_staged_provider_root(config_dir).await {
            Ok(Some(report)) => emit_daemon_status_event(
                config_dir,
                provider_root_import_status_payload("provider_root_staged_imported", &report),
            ),
            Ok(None) => {}
            Err(error) => println!(
                "{}",
                json!({"event": "provider_root_staged_import_error", "trigger": trigger, "error": format!("{error:#}")})
            ),
        }
    } else if let Some(status_payload) = provider_root_wake_status_payload(wake_payload) {
        emit_daemon_status_event(config_dir, status_payload);
    }
}

const PROVIDER_ROOT_SAFETY_POLL_MIN_SECS: u64 = 30;

fn provider_root_poll_enabled(config_root_watch_active: bool) -> bool {
    !config_root_watch_active
}

fn provider_root_poll_period(watch_interval_secs: u64) -> std::time::Duration {
    std::time::Duration::from_secs(watch_interval_secs.max(PROVIDER_ROOT_SAFETY_POLL_MIN_SECS))
}

fn provider_root_event_recheck_delay(debounce: std::time::Duration) -> std::time::Duration {
    debounce
        .saturating_mul(4)
        .max(std::time::Duration::from_secs(1))
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

#[derive(Clone, Debug, Eq, PartialEq)]
enum RootApplyFollowupQueueKey {
    AppKeyRoots(Vec<String>),
    MergedDriveRoots(String),
}

fn update_last_provider_root_key(config_dir: &Path, last_root_key: &mut Option<String>) {
    if let Ok(config) = AppConfig::load_or_default_cached_profile(config_path_in(config_dir)) {
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

fn root_apply_followup_queue_key(
    config: &AppConfig,
    root_cid_to_pull: Option<&str>,
    should_refresh_projection: bool,
) -> Option<RootApplyFollowupQueueKey> {
    if let Some(root_cid) = root_cid_to_pull
        && let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)
    {
        let roots = drive
            .app_key_roots
            .iter()
            .filter(|(_, root)| root.root_cid == root_cid)
            .map(|(device, _)| device.clone())
            .collect::<Vec<_>>();
        if !roots.is_empty() {
            return Some(RootApplyFollowupQueueKey::AppKeyRoots(roots));
        }
    }
    if should_refresh_projection {
        return merged_drive_roots_key(config).map(RootApplyFollowupQueueKey::MergedDriveRoots);
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
    let Ok(config) = AppConfig::load_or_default_cached_profile(config_path_in(config_dir)) else {
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
    if provider_root_wake_payload_is_staged(wake_payload) {
        return None;
    }
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

fn provider_root_wake_payload_is_staged(wake_payload: &Value) -> bool {
    wake_payload
        .get("staged")
        .and_then(Value::as_bool)
        .unwrap_or(false)
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

async fn drain_latest_provider_root_wake_payload_after_debounce(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<Option<Value>>,
    debounce: std::time::Duration,
    latest: Option<Value>,
) -> Option<Value> {
    tokio::time::sleep(debounce).await;
    drain_latest_provider_root_wake_payload(rx, latest)
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
        tokio::time::sleep(std::time::Duration::from_secs(PARENT_EXIT_POLL_SECS)).await;
        if !process_is_running(parent_pid) {
            return;
        }
    }
}

include!("daemon/direct_root_state_request.rs");
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
        status.remove("last_block_sync_error");
        let entry = status
            .entry("block_sync_by_root".to_string())
            .or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({});
        }
        if let Some(map) = entry.as_object_mut() {
            map.insert(root_cid.to_string(), value);
        }
        if let Some(map) = status
            .get_mut("block_sync_error_by_root")
            .and_then(Value::as_object_mut)
        {
            map.remove(root_cid);
        }
    });
}

pub(crate) fn record_block_sync_error(config_dir: &Path, root_cid: &str, error: &str) {
    let value = json!({
        "root_cid": root_cid,
        "updated_at": unix_now(),
        "error": error,
    });
    merge_daemon_status(config_dir, |status| {
        status.insert("last_block_sync_error".to_string(), value.clone());
        let entry = status
            .entry("block_sync_error_by_root".to_string())
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

pub(crate) fn authorized_app_keys_missing_primary_roots(config: &AppConfig) -> Vec<String> {
    let Some(state) = config.profile.as_ref() else {
        return Vec::new();
    };
    let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID) else {
        return Vec::new();
    };
    iris_drive_core::drive_root_recipient_app_key_pubkeys(state, drive)
        .into_iter()
        .filter(|app_key_pubkey| app_key_pubkey != &state.app_key_pubkey)
        .filter(|app_key_pubkey| !drive.app_key_roots.contains_key(app_key_pubkey))
        .collect()
}

pub(crate) fn online_authorized_app_key_missing_primary_root_count(
    config_dir: &Path,
    fallback_config: &AppConfig,
    _sync: &FsFipsBlockSync,
) -> usize {
    let current_config = AppConfig::load_or_default_cached_profile(config_path_in(config_dir))
        .unwrap_or_else(|_| fallback_config.clone());
    authorized_app_keys_missing_primary_roots(&current_config).len()
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
    include!("daemon/tests_part3.rs");
}
