#[allow(clippy::wildcard_imports)]
use super::*;

fn emit_daemon_status_event(config_dir: &Path, payload: Value) {
    write_daemon_status(config_dir, payload.clone());
    println!("{payload}");
}

#[allow(
    clippy::needless_pass_by_value,
    clippy::too_many_arguments,
    clippy::too_many_lines
)]
pub(crate) fn cmd_daemon(
    config_dir: &std::path::Path,
    relay_override: &[String],
    watch_interval: u64,
    watch_debounce_ms: u64,
    gateway_port: u16,
    enable_gateway: bool,
    mount_drive: bool,
    mountpoint: Option<PathBuf>,
) -> Result<()> {
    use iris_drive_core::relay_sync;
    use nostr_sdk::RelayPoolNotification;
    use tokio::sync::broadcast::error::RecvError;
    use tokio::sync::mpsc;

    let _daemon_lock = DaemonProcessLock::acquire(config_dir)?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .context("building tokio runtime")?;

    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let relays = pick_relays(&config, relay_override);
    let filters =
        relay_sync::subscription_filters(&state.owner_pubkey, iris_drive_core::PRIMARY_DRIVE_ID);
    if filters.is_empty() {
        return Err(anyhow::anyhow!("no filters to subscribe to"));
    }
    let embedded_hashtree =
        EmbeddedHashtreeHost::start(config_dir, &config).context("starting embedded hashtree")?;
    let embedded_hashtree_status = embedded_hashtree.status_payload();

    runtime.block_on(async {
        let mut block_config = config.clone();
        block_config.relays = relays.clone();
        let (fips_blocks, fips_block_sync_error) =
            match start_fips_block_sync(config_dir, &block_config).await {
                Ok(sync) => (Some(Arc::new(sync)), None),
                Err(error) => (None, Some(error.to_string())),
            };
        let (webdav_root_tx, mut webdav_root_rx) =
            mpsc::unbounded_channel::<iris_drive_core::gateway::VirtualRootUpdate>();
        #[cfg(windows)]
        let (
            windows_cloud_root,
            mut windows_cloud_root_rx,
            _windows_cloud_root_watcher,
            windows_cloud_status,
        ) = match start_windows_cloud_root_watch() {
            Ok(watch) => watch,
            Err(error) => {
                let status = json!({
                    "root": null,
                    "watching": false,
                    "error": format!("{error:#}"),
                });
                println!("{}", json!({"event": "windows_cloud_root_watch_error", "error": format!("{error:#}")}));
                (None, None, None, Some(status))
            }
        };
        #[cfg(not(windows))]
        let (windows_cloud_root, mut windows_cloud_root_rx, windows_cloud_status) = (
            None::<PathBuf>,
            None::<mpsc::UnboundedReceiver<WindowsCloudRootChange>>,
            None::<Value>,
        );
        let gateway = if enable_gateway {
            let daemon = Daemon::open(config_dir).context("opening daemon for browser gateway")?;
            Some(
                GatewayServer::bind_with_tree_htree_daemon_and_root_updates(
                    config_dir,
                    daemon.tree_handle(),
                    embedded_hashtree.status().base_url.clone(),
                    webdav_root_tx.clone(),
                    GatewayBind::loopback_v4(gateway_port),
                )
                    .await
                    .context("starting browser gateway")?,
            )
        } else {
            None
        };
        let gateway_status = gateway.as_ref().map(|server| {
            let port = server.local_addr().port();
            json!({
                "bind": server.local_addr().to_string(),
                "portal_url": format!("http://sites.iris.localhost:{port}/"),
                "primary_drive_url": iris_drive_core::gateway::local_drive_url(
                    port,
                    iris_drive_core::PRIMARY_DRIVE_ID,
                ),
                "webdav_url": format!("http://127.0.0.1:{port}/dav/"),
                "webdav_unc": format!(r"\\127.0.0.1@{port}\dav"),
                "hashtree_base_url": embedded_hashtree.status().base_url.clone(),
            })
        });
        let client = relay_sync::connect(&relays)
            .await
            .context("connecting to relays")?;
        let relay_statuses = relay_status_payload(&client).await;
        client
            .subscribe(filters, None)
            .await
            .context("opening subscription")?;
        let mut notifications = client.notifications();
        let mut direct_roots = DirectRootExchange::default();
        let startup_fips_block_sync_status = fips_block_sync_status(fips_blocks.as_deref()).await;
        let mut config = config.clone();
        let mut mounted_drive = if mount_drive {
            let mountpoint = mountpoint
                .clone()
                .unwrap_or_else(|| default_mountpoint_in(config_dir));
            let mounted = mount::start_iris_drive_mount(config_dir, mountpoint).await?;
            config = AppConfig::load_or_default(config_path_in(config_dir))?;
            Some(mounted)
        } else {
            None
        };
        let mount_refresh = mounted_drive.as_ref().map(mount::IrisDriveMount::handle);
        let mut mount_root_updates = mounted_drive
            .as_mut()
            .map(mount::IrisDriveMount::take_updates);
        let mut mount_tombstone_base = mounted_drive
            .as_ref()
            .map(mount::IrisDriveMount::current_visible_root);
        let (mount_refresh_tx, mut mount_refresh_rx) = if mounted_drive.is_some() {
            let (tx, rx) = mpsc::channel::<&'static str>(8);
            (Some(tx), Some(rx))
        } else {
            (None, None)
        };
        let mount_status = mounted_drive.as_ref().map(|mounted| {
            json!({
                "mountpoint": mounted.mountpoint().display().to_string(),
                "backend": "hashtree-fuse",
            })
        });
        let subscribed_status = json!({
                "event": "subscribed",
                "relays": relays,
                "owner_npub": account_npub(&state.owner_pubkey),
                "watch_interval_secs": watch_interval,
                "watch_debounce_ms": watch_debounce_ms,
                "mount": mount_status,
                "relay_statuses": relay_statuses,
                "embedded_hashtree": embedded_hashtree_status,
                "browser_gateway": gateway_status,
                "windows_cloud_root": windows_cloud_status,
                "fips_block_sync": startup_fips_block_sync_status,
                "fips_block_sync_error": fips_block_sync_error,
        });
        write_daemon_status(config_dir, subscribed_status.clone());
        println!("{subscribed_status}");
        spawn_daemon_heartbeat(config_dir.to_path_buf());

        let startup_config = config.clone();
        let startup_state = state.clone();
        spawn_root_apply_followup(
            config_dir.to_path_buf(),
            config.clone(),
            None,
            fips_blocks.clone(),
            true,
            "startup_materialized",
            mount_refresh_tx.clone(),
        );
        spawn_initial_publish(
            client.clone(),
            config_dir.to_path_buf(),
            startup_config,
            startup_state,
        );
        println!("(running — Ctrl+C to stop)");

        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        let parent_exit = parent_exit_signal();
        tokio::pin!(parent_exit);

        let relay_status_period = std::time::Duration::from_secs(10);
        let mut relay_status_timer = tokio::time::interval_at(
            tokio::time::Instant::now() + relay_status_period,
            relay_status_period,
        );
        relay_status_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut direct_mesh_timer = tokio::time::interval(std::time::Duration::from_millis(100));
        direct_mesh_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let provider_root_period = std::time::Duration::from_secs(watch_interval.max(1));
        let mut provider_root_timer = tokio::time::interval_at(
            tokio::time::Instant::now() + provider_root_period,
            provider_root_period,
        );
        provider_root_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_provider_root_key = current_device_root_key(&config);

        loop {
            tokio::select! {
                _ = &mut ctrl_c => {
                    println!("{}", json!({ "event": "shutdown" }));
                    break;
                }
                () = &mut parent_exit => {
                    println!("{}", json!({ "event": "shutdown", "reason": "parent_exit" }));
                    break;
                }
                recv = notifications.recv() => {
                    match recv {
                        Ok(RelayPoolNotification::Event { event, .. }) => {
                            if let Err(e) =
                                apply_one_event(
                                    &client,
                                    config_dir,
                                    &event,
                                    fips_blocks.clone(),
                                    mount_refresh_tx.clone(),
                                )
                                .await
                            {
                                println!(
                                    "{}",
                                    json!({"event": "apply_error", "id": event.id.to_hex(), "error": e.to_string()})
                                );
                            } else if let Err(error) =
                                announce_current_state_direct(
                                    &mut direct_roots,
                                    config_dir,
                                    fips_blocks.as_deref(),
                                )
                                .await
                            {
                                println!(
                                    "{}",
                                    json!({"event": "direct_root_mesh_error", "error": format!("{error:#}")})
                                );
                            }
                        }
                        Ok(RelayPoolNotification::Shutdown)
                        | Err(RecvError::Closed) => break,
                        Ok(_) => {}
                        Err(RecvError::Lagged(n)) => {
                            println!("{}", json!({"event": "lagged", "skipped": n}));
                        }
                    }
                }
                Some(mut visible_root) = async {
                    if let Some(rx) = mount_root_updates.as_mut() {
                        rx.recv().await
                    } else {
                        std::future::pending::<Option<Cid>>().await
                    }
                } => {
                    if let Some(rx) = mount_root_updates.as_mut() {
                        while let Ok(next) = rx.try_recv() {
                            visible_root = next;
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(watch_debounce_ms)).await;
                        while let Ok(next) = rx.try_recv() {
                            visible_root = next;
                        }
                    }
                    let imported_visible_root = visible_root.clone();
                    match import_mount_root_and_publish(
                        &client,
                        config_dir,
                        visible_root,
                        mount_tombstone_base.clone(),
                        &mut direct_roots,
                        fips_blocks.as_deref(),
                    )
                    .await
                    {
                        Ok(()) => {
                            mount_tombstone_base = Some(imported_visible_root);
                        }
                        Err(error) => println!(
                            "{}",
                            json!({"event": "mount_publish_error", "error": format!("{error:#}")})
                        ),
                    }
                }
                Some(mut update) = webdav_root_rx.recv() => {
                    while let Ok(next) = webdav_root_rx.try_recv() {
                        update.visible_root = next.visible_root;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(watch_debounce_ms)).await;
                    while let Ok(next) = webdav_root_rx.try_recv() {
                        update.visible_root = next.visible_root;
                    }
                    match import_mount_root_and_publish(
                        &client,
                        config_dir,
                        update.visible_root,
                        Some(update.base_root),
                        &mut direct_roots,
                        fips_blocks.as_deref(),
                    )
                    .await
                    {
                        Ok(()) => {}
                        Err(error) => println!(
                            "{}",
                            json!({"event": "virtual_publish_error", "error": format!("{error:#}")})
                        ),
                    }
                }
                Some(change) = async {
                    if let Some(rx) = windows_cloud_root_rx.as_mut() {
                        rx.recv().await
                    } else {
                        std::future::pending::<Option<WindowsCloudRootChange>>().await
                    }
                } => {
                    let mut changes = vec![change];
                    if let Some(rx) = windows_cloud_root_rx.as_mut() {
                        while let Ok(next) = rx.try_recv() {
                            changes.push(next);
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(watch_debounce_ms)).await;
                        while let Ok(next) = rx.try_recv() {
                            changes.push(next);
                        }
                    }
                    if let Some(root) = windows_cloud_root.as_ref() {
                        match import_windows_cloud_root_changes_and_publish(
                            &client,
                            config_dir,
                            root,
                            changes,
                            &mut direct_roots,
                            fips_blocks.as_deref(),
                        )
                        .await
                        {
                            Ok(WindowsCloudImportOutcome::Changed { root_cid, paths }) => {
                                emit_daemon_status_event(config_dir, json!({
                                    "event": "windows_cloud_root_published",
                                    "root_cid": root_cid,
                                    "paths": paths,
                                }));
                            }
                            Ok(WindowsCloudImportOutcome::Unchanged) => {}
                            Err(error) => println!(
                                "{}",
                                json!({"event": "windows_cloud_root_publish_error", "error": format!("{error:#}")})
                            ),
                        }
                    }
                }
                Some(reason) = async {
                    if let Some(rx) = mount_refresh_rx.as_mut() {
                        rx.recv().await
                    } else {
                        std::future::pending::<Option<&'static str>>().await
                    }
                } => {
                    if let Some(handle) = mount_refresh.as_ref() {
                        match handle.refresh_from_config(config_dir).await {
                            Ok(visible) => {
                                mount_tombstone_base = Some(visible.root_cid.clone());
                                emit_daemon_status_event(config_dir, json!({
                                    "event": "mount_refreshed",
                                    "trigger": reason,
                                    "mountpoint": handle.mountpoint().display().to_string(),
                                    "root_cid": visible.root_cid.to_string(),
                                    "file_count": visible.file_count,
                                    "top_level_entries": visible.top_level_entries,
                                }));
                            }
                            Err(error) => println!(
                                "{}",
                                json!({"event": "mount_refresh_error", "trigger": reason, "error": format!("{error:#}")})
                            ),
                        }
                    }
                }
                _ = relay_status_timer.tick() => {
                    spawn_status_probe(client.clone(), config_dir.to_path_buf(), fips_blocks.clone());
                    direct_roots
                        .request_roots_from_new_peers(config_dir, fips_blocks.as_deref())
                        .await;
                    if let Err(error) =
                        announce_current_state_direct(
                            &mut direct_roots,
                            config_dir,
                            fips_blocks.as_deref(),
                        )
                        .await
                    {
                        println!(
                            "{}",
                            json!({"event": "direct_root_mesh_error", "error": format!("{error:#}")})
                        );
                    }
                }
                _ = provider_root_timer.tick() => {
                    if let Some(root) = windows_cloud_root.as_ref() {
                        match import_windows_cloud_root_changes_and_publish(
                            &client,
                            config_dir,
                            root,
                            vec![WindowsCloudRootChange::Rescan],
                            &mut direct_roots,
                            fips_blocks.as_deref(),
                        )
                        .await
                        {
                            Ok(WindowsCloudImportOutcome::Changed { root_cid, paths }) => {
                                emit_daemon_status_event(config_dir, json!({
                                    "event": "windows_cloud_root_published",
                                    "root_cid": root_cid,
                                    "paths": paths,
                                }));
                            }
                            Ok(WindowsCloudImportOutcome::Unchanged) => {}
                            Err(error) => println!(
                                "{}",
                                json!({"event": "windows_cloud_root_publish_error", "error": format!("{error:#}")})
                            ),
                        }
                    }
                    match publish_provider_root_if_changed(
                        &client,
                        config_dir,
                        &mut last_provider_root_key,
                        &mut direct_roots,
                        fips_blocks.as_deref(),
                    )
                    .await
                    {
                        Ok(Some(_updated_config)) => {}
                        Ok(None) => {}
                        Err(error) => println!(
                            "{}",
                            json!({"event": "provider_root_publish_error", "error": format!("{error:#}")})
                        ),
                    }
                }
                _ = direct_mesh_timer.tick() => {
                    if let Some(sync) = fips_blocks.as_ref()
                        && let Err(error) = direct_roots
                            .drain_mesh_events(
                                &client,
                                config_dir,
                                sync.clone(),
                                mount_refresh_tx.clone(),
                            )
                            .await
                    {
                        println!(
                            "{}",
                            json!({"event": "direct_root_mesh_error", "error": format!("{error:#}")})
                        );
                    }
                }
            }
        }
        let _ = client.disconnect().await;
        Ok::<_, anyhow::Error>(())
    })
}

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
    *last_root_key = current_key.clone();
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
    let publish =
        publish_current_state(client, config_dir, &updated_config, &updated_state, true).await?;
    emit_daemon_status_event(
        config_dir,
        json!({
            "event": "provider_root_published",
            "root_key": current_key,
            "direct_root_mesh_error": direct_root_mesh_error,
            "publish": publish_state_report_json(&publish),
        }),
    );

    Ok(Some(updated_config))
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

#[derive(Debug, Clone)]
#[cfg_attr(not(windows), allow(dead_code))]
enum WindowsCloudRootChange {
    Upsert(String),
    Delete(String),
    Rename { old_path: String, new_path: String },
    Rescan,
}

#[derive(Debug)]
enum WindowsCloudImportOutcome {
    Changed {
        root_cid: String,
        paths: Vec<String>,
    },
    Unchanged,
}

#[cfg(windows)]
fn start_windows_cloud_root_watch() -> Result<(
    Option<PathBuf>,
    Option<tokio::sync::mpsc::UnboundedReceiver<WindowsCloudRootChange>>,
    Option<notify::RecommendedWatcher>,
    Option<Value>,
)> {
    use notify::{RecursiveMode, Watcher};

    let home = dirs::home_dir().context("finding Windows profile directory")?;
    let root = home.join("Iris Drive");
    std::fs::create_dir_all(&root)
        .with_context(|| format!("creating Windows Cloud Files root {}", root.display()))?;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let callback_tx = tx.clone();
    let callback_root = root.clone();
    let mut watcher = notify::recommended_watcher(move |result| match result {
        Ok(event) => {
            for change in windows_cloud_changes_from_event(&callback_root, event) {
                let _ = callback_tx.send(change);
            }
        }
        Err(error) => {
            eprintln!("windows cloud root watch error: {error:#}");
        }
    })
    .context("creating Windows Cloud Files watcher")?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .with_context(|| format!("watching Windows Cloud Files root {}", root.display()))?;
    let _ = tx.send(WindowsCloudRootChange::Rescan);

    Ok((
        Some(root.clone()),
        Some(rx),
        Some(watcher),
        Some(json!({
            "root": root.display().to_string(),
            "watching": true,
        })),
    ))
}

#[cfg(windows)]
fn windows_cloud_changes_from_event(
    root: &Path,
    event: notify::Event,
) -> Vec<WindowsCloudRootChange> {
    use notify::event::{EventKind, ModifyKind, RenameMode};

    match event.kind {
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if event.paths.len() >= 2 => {
            match (
                windows_cloud_relative_path(root, &event.paths[0]),
                windows_cloud_relative_path(root, &event.paths[1]),
            ) {
                (Some(old_path), Some(new_path)) => {
                    vec![WindowsCloudRootChange::Rename { old_path, new_path }]
                }
                _ => Vec::new(),
            }
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) | EventKind::Remove(_) => event
            .paths
            .iter()
            .filter_map(|path| windows_cloud_relative_path(root, path))
            .map(WindowsCloudRootChange::Delete)
            .collect(),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both))
        | EventKind::Modify(ModifyKind::Name(RenameMode::To))
        | EventKind::Modify(ModifyKind::Name(RenameMode::Any))
        | EventKind::Modify(ModifyKind::Name(RenameMode::Other))
        | EventKind::Create(_)
        | EventKind::Modify(ModifyKind::Any)
        | EventKind::Modify(ModifyKind::Data(_))
        | EventKind::Modify(ModifyKind::Metadata(_))
        | EventKind::Modify(ModifyKind::Other)
        | EventKind::Any
        | EventKind::Other => event
            .paths
            .iter()
            .filter_map(|path| windows_cloud_relative_path(root, path))
            .map(WindowsCloudRootChange::Upsert)
            .collect(),
        EventKind::Access(_) => Vec::new(),
    }
}

async fn import_windows_cloud_root_changes_and_publish(
    client: &nostr_sdk::Client,
    config_dir: &Path,
    sync_root: &Path,
    changes: Vec<WindowsCloudRootChange>,
    direct_roots: &mut DirectRootExchange,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<WindowsCloudImportOutcome> {
    let daemon = Daemon::open(config_dir).context("opening daemon for Windows Cloud Files root")?;
    let visible = iris_drive_core::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .context("building Windows Cloud Files provider root")?;
    let provider = HashTreeProviderFs::open(daemon.tree_handle(), visible.root_cid.clone()).await?;
    let before = provider.current_root().await;
    let placeholder_paths = load_windows_cloud_provider_path_cache(config_dir);
    let previous_local_state = load_windows_cloud_local_state(config_dir);
    let mut changed_paths = BTreeSet::new();
    for path in prune_ignored_provider_paths(&provider).await? {
        changed_paths.insert(path);
    }
    let expected_entries = windows_cloud_provider_expected_entries(&provider).await?;
    let expected_paths: BTreeSet<String> = expected_entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    for path in windows_cloud_remove_stale_synced_local_items(
        sync_root,
        &expected_paths,
        &previous_local_state,
    ) {
        changed_paths.insert(path);
    }

    for change in changes {
        match change {
            WindowsCloudRootChange::Upsert(path) => {
                if apply_windows_cloud_upsert(&provider, sync_root, &path, &placeholder_paths)
                    .await?
                {
                    changed_paths.insert(path);
                }
            }
            WindowsCloudRootChange::Delete(path) => {
                if apply_windows_cloud_delete(&provider, &path).await? {
                    changed_paths.insert(path);
                }
            }
            WindowsCloudRootChange::Rename { old_path, new_path } => {
                if apply_windows_cloud_rename(
                    &provider,
                    sync_root,
                    &old_path,
                    &new_path,
                    &placeholder_paths,
                )
                .await?
                {
                    changed_paths.insert(old_path);
                    changed_paths.insert(new_path);
                }
            }
            WindowsCloudRootChange::Rescan => {
                for path in
                    windows_cloud_missing_cached_provider_paths(sync_root, &placeholder_paths)?
                {
                    if apply_windows_cloud_delete(&provider, &path).await? {
                        changed_paths.insert(path);
                    }
                }
                for path in windows_cloud_local_materialized_paths(sync_root)? {
                    if apply_windows_cloud_upsert(&provider, sync_root, &path, &placeholder_paths)
                        .await?
                    {
                        changed_paths.insert(path);
                    }
                }
            }
        }
    }

    let root = provider.current_root().await;
    let current_entries = windows_cloud_provider_expected_entries(&provider).await?;
    write_windows_cloud_local_state(config_dir, sync_root, &current_entries);
    drop(provider);
    drop(daemon);

    if root == before {
        return Ok(WindowsCloudImportOutcome::Unchanged);
    }

    import_mount_root_and_publish(
        client,
        config_dir,
        root.clone(),
        Some(before),
        direct_roots,
        fips_blocks,
    )
    .await
    .context("publishing Windows Cloud Files root")?;

    Ok(WindowsCloudImportOutcome::Changed {
        root_cid: root.to_string(),
        paths: changed_paths.into_iter().collect(),
    })
}

async fn apply_windows_cloud_rename(
    provider: &HashTreeProviderFs<FsBlobStore>,
    sync_root: &Path,
    old_path: &str,
    new_path: &str,
    placeholder_paths: &BTreeSet<String>,
) -> Result<bool> {
    let old_path = normalize_provider_path(old_path)?;
    let new_path = normalize_provider_path(new_path)?;
    if iris_drive_core::path_has_ignored_component(&new_path) {
        let deleted_old = apply_windows_cloud_delete(provider, &old_path).await?;
        let deleted_new = apply_windows_cloud_delete(provider, &new_path).await?;
        return Ok(deleted_old || deleted_new);
    }
    if iris_drive_core::path_has_ignored_component(&old_path) {
        let deleted_old = apply_windows_cloud_delete(provider, &old_path).await?;
        let upserted_new =
            apply_windows_cloud_upsert(provider, sync_root, &new_path, placeholder_paths).await?;
        return Ok(deleted_old || upserted_new);
    }
    let new_full_path = windows_cloud_full_path(sync_root, &new_path);
    if windows_cloud_path_is_reparse_point(&new_full_path) {
        match provider.item(&old_path).await {
            Ok(_) => {
                rename_provider_path(provider, &old_path, &new_path).await?;
                return Ok(true);
            }
            Err(_) => return Ok(false),
        }
    }

    let deleted = apply_windows_cloud_delete(provider, &old_path).await?;
    let upserted =
        apply_windows_cloud_upsert(provider, sync_root, &new_path, placeholder_paths).await?;
    Ok(deleted || upserted)
}

async fn apply_windows_cloud_delete(
    provider: &HashTreeProviderFs<FsBlobStore>,
    path: &str,
) -> Result<bool> {
    let path = match normalize_provider_path(path) {
        Ok(path) => path,
        Err(_) => return Ok(false),
    };
    match provider.item(&path).await {
        Ok(_) => {
            delete_provider_path(provider, &path).await?;
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}

async fn apply_windows_cloud_upsert(
    provider: &HashTreeProviderFs<FsBlobStore>,
    sync_root: &Path,
    path: &str,
    placeholder_paths: &BTreeSet<String>,
) -> Result<bool> {
    let path = match normalize_provider_path(path) {
        Ok(path) => path,
        Err(_) => return Ok(false),
    };
    if iris_drive_core::path_has_ignored_component(&path) {
        return apply_windows_cloud_delete(provider, &path).await;
    }
    if placeholder_paths.contains(&path) && provider.item(&path).await.is_err() {
        return Ok(false);
    }
    let mut changed = false;
    let mut stack = vec![path];
    while let Some(path) = stack.pop() {
        let full_path = windows_cloud_full_path(sync_root, &path);
        let metadata = match std::fs::symlink_metadata(&full_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading metadata for {}", full_path.display()));
            }
        };
        if windows_cloud_metadata_is_reparse_point(&metadata) {
            continue;
        }
        if metadata.is_dir() {
            if !provider_dir_exists(provider, &path).await? {
                create_provider_dir(provider, &path).await?;
                changed = true;
            }
            let mut children = Vec::new();
            for entry in
                std::fs::read_dir(&full_path).with_context(|| format!("reading {}", path))?
            {
                let entry = entry?;
                let child = entry.path();
                let child_metadata = match std::fs::symlink_metadata(&child) {
                    Ok(metadata) => metadata,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                    Err(error) => {
                        return Err(error)
                            .with_context(|| format!("reading metadata for {}", child.display()));
                    }
                };
                if windows_cloud_metadata_is_reparse_point(&child_metadata) {
                    continue;
                }
                if let Some(relative) = windows_cloud_relative_path(sync_root, &child) {
                    children.push(relative);
                }
            }
            children.sort_by(|a, b| b.cmp(a));
            stack.extend(children);
        } else if metadata.is_file() {
            let bytes = match std::fs::read(&full_path) {
                Ok(bytes) => bytes,
                Err(error) if windows_cloud_file_read_should_skip(&error) => continue,
                Err(error) => {
                    return Err(error).with_context(|| format!("reading {}", full_path.display()));
                }
            };
            if provider_file_matches(provider, &path, &bytes).await? {
                continue;
            }
            write_provider_file(provider, &path, &bytes).await?;
            changed = true;
        }
    }
    Ok(changed)
}

async fn provider_dir_exists(
    provider: &HashTreeProviderFs<FsBlobStore>,
    path: &str,
) -> Result<bool> {
    match provider.item(&path.to_string()).await {
        Ok(item) => Ok(item.kind == ItemKind::Directory),
        Err(_) => Ok(false),
    }
}

async fn provider_file_matches(
    provider: &HashTreeProviderFs<FsBlobStore>,
    path: &str,
    bytes: &[u8],
) -> Result<bool> {
    let path = path.to_string();
    let item = match provider.item(&path).await {
        Ok(item) if item.kind == ItemKind::File => item,
        Ok(_) | Err(_) => return Ok(false),
    };
    if item.size != bytes.len() as u64 {
        return Ok(false);
    }
    let existing = provider
        .read(&path, 0, item.size)
        .await
        .with_context(|| format!("reading provider file {path}"))?;
    Ok(existing == bytes)
}

async fn prune_ignored_provider_paths(
    provider: &HashTreeProviderFs<FsBlobStore>,
) -> Result<Vec<String>> {
    let mut pruned = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(parent) = stack.pop() {
        let mut children = provider.read_dir(&parent).await?;
        children.sort_by(|a, b| a.id.cmp(&b.id));
        for child in children {
            let path = child.id;
            if iris_drive_core::path_has_ignored_component(&path) {
                if apply_windows_cloud_delete(provider, &path).await? {
                    pruned.push(path);
                }
                continue;
            }
            let item = provider.item(&path).await?;
            if item.kind == ItemKind::Directory {
                stack.push(path);
            }
        }
    }
    Ok(pruned)
}

fn windows_cloud_local_materialized_paths(root: &Path) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    if !root.is_dir() {
        return Ok(paths);
    }
    for entry in std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let entry = entry?;
        let path = entry.path();
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading metadata for {}", path.display()));
            }
        };
        if windows_cloud_metadata_is_reparse_point(&metadata) {
            continue;
        }
        let Some(relative) = windows_cloud_relative_path(root, &path) else {
            continue;
        };
        paths.push(relative);
    }
    paths.sort();
    Ok(paths)
}

fn windows_cloud_missing_cached_provider_paths(
    root: &Path,
    cached_paths: &BTreeSet<String>,
) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for path in cached_paths {
        let full_path = windows_cloud_full_path(root, path);
        match std::fs::symlink_metadata(&full_path) {
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                paths.push(path.clone());
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("reading metadata for {}", full_path.display()));
            }
        }
    }
    paths.sort_by(|a, b| {
        a.split('/')
            .count()
            .cmp(&b.split('/').count())
            .then_with(|| a.cmp(b))
    });
    Ok(paths)
}

fn load_windows_cloud_provider_path_cache(config_dir: &Path) -> BTreeSet<String> {
    let path = config_dir.join("windows-cloud-provider-paths.json");
    let Ok(raw) = std::fs::read_to_string(path) else {
        return BTreeSet::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return BTreeSet::new();
    };
    value
        .get("paths")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(|path| normalize_provider_path(path).ok())
        .collect()
}

const WINDOWS_CLOUD_LOCAL_STATE_FILE: &str = "windows-cloud-local-state.json";

#[derive(Debug, Clone, Eq, PartialEq)]
struct WindowsCloudExpectedEntry {
    path: String,
    kind: &'static str,
    size: u64,
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize)]
struct WindowsCloudLocalStateEntry {
    path: String,
    kind: String,
    size: u64,
    sha256: Option<String>,
}

impl WindowsCloudLocalStateEntry {
    fn is_directory(&self) -> bool {
        self.kind.eq_ignore_ascii_case("directory")
    }
}

async fn windows_cloud_provider_expected_entries(
    provider: &HashTreeProviderFs<FsBlobStore>,
) -> Result<Vec<WindowsCloudExpectedEntry>> {
    let mut entries = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(parent) = stack.pop() {
        let mut children = provider.read_dir(&parent).await?;
        children.sort_by(|a, b| a.id.cmp(&b.id));
        for child in children {
            let item = provider.item(&child.id).await?;
            let kind = match item.kind {
                ItemKind::Directory => {
                    stack.push(child.id.clone());
                    "directory"
                }
                ItemKind::File => "file",
            };
            entries.push(WindowsCloudExpectedEntry {
                path: child.id,
                kind,
                size: item.size,
            });
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(entries)
}

fn load_windows_cloud_local_state(config_dir: &Path) -> Vec<WindowsCloudLocalStateEntry> {
    let path = config_dir.join(WINDOWS_CLOUD_LOCAL_STATE_FILE);
    let Ok(raw) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<Value>(&raw) else {
        return Vec::new();
    };
    value
        .get("entries")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(windows_cloud_local_state_entry_from_json)
        .collect()
}

fn windows_cloud_local_state_entry_from_json(value: &Value) -> Option<WindowsCloudLocalStateEntry> {
    let path = windows_cloud_json_string(value, "path", "Path")
        .and_then(|path| normalize_provider_path(path).ok())?;
    if iris_drive_core::path_has_ignored_component(&path) {
        return None;
    }
    let kind = windows_cloud_json_string(value, "kind", "Kind")
        .unwrap_or("file")
        .to_string();
    let size = windows_cloud_json_u64(value, "size", "Size").unwrap_or(0);
    let sha256 = windows_cloud_json_string(value, "sha256", "Sha256")
        .filter(|hash| !hash.trim().is_empty())
        .map(str::to_string);
    Some(WindowsCloudLocalStateEntry {
        path,
        kind,
        size,
        sha256,
    })
}

fn windows_cloud_json_string<'a>(value: &'a Value, lower: &str, upper: &str) -> Option<&'a str> {
    value
        .get(lower)
        .or_else(|| value.get(upper))
        .and_then(Value::as_str)
}

fn windows_cloud_json_u64(value: &Value, lower: &str, upper: &str) -> Option<u64> {
    value
        .get(lower)
        .or_else(|| value.get(upper))
        .and_then(Value::as_u64)
}

fn windows_cloud_remove_stale_synced_local_items(
    sync_root: &Path,
    expected_paths: &BTreeSet<String>,
    previous_state: &[WindowsCloudLocalStateEntry],
) -> Vec<String> {
    if previous_state.is_empty() {
        return Vec::new();
    }
    let mut state = previous_state.to_vec();
    state.sort_by(|a, b| {
        b.path
            .split('/')
            .count()
            .cmp(&a.path.split('/').count())
            .then_with(|| b.path.cmp(&a.path))
    });
    let mut removed = Vec::new();

    for previous in state {
        let Ok(path) = normalize_provider_path(&previous.path) else {
            continue;
        };
        if iris_drive_core::path_has_ignored_component(&path) || expected_paths.contains(&path) {
            continue;
        }
        let full_path = windows_cloud_full_path(sync_root, &path);
        if previous.is_directory() {
            if full_path.is_dir()
                && !windows_cloud_path_is_reparse_point(&full_path)
                && std::fs::remove_dir(&full_path).is_ok()
            {
                removed.push(path);
            }
            continue;
        }

        let Some(expected_hash) = previous.sha256.as_deref() else {
            continue;
        };
        if !full_path.is_file() || windows_cloud_path_is_reparse_point(&full_path) {
            continue;
        }
        let Ok(Some(snapshot)) = windows_cloud_snapshot_local_file(&full_path) else {
            continue;
        };
        if snapshot.size != previous.size || snapshot.sha256 != expected_hash {
            continue;
        }
        let _ = windows_cloud_clear_readonly(&full_path);
        if std::fs::remove_file(&full_path).is_ok() {
            removed.push(path);
        }
    }

    removed
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct WindowsCloudLocalFileSnapshot {
    size: u64,
    sha256: String,
}

fn windows_cloud_snapshot_local_file(
    path: &Path,
) -> std::io::Result<Option<WindowsCloudLocalFileSnapshot>> {
    if windows_cloud_path_is_reparse_point(path) {
        return Ok(None);
    }
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if windows_cloud_file_read_should_skip(&error) => return Ok(None),
        Err(error) => return Err(error),
    };
    Ok(Some(WindowsCloudLocalFileSnapshot {
        size: bytes.len() as u64,
        sha256: to_hex(&hashtree_core::sha256(&bytes)),
    }))
}

#[cfg(windows)]
fn windows_cloud_clear_readonly(path: &Path) -> std::io::Result<()> {
    let metadata = std::fs::metadata(path)?;
    let mut permissions = metadata.permissions();
    if permissions.readonly() {
        permissions.set_readonly(false);
        std::fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn windows_cloud_clear_readonly(path: &Path) -> std::io::Result<()> {
    let _ = std::fs::metadata(path)?;
    Ok(())
}

fn write_windows_cloud_local_state(
    config_dir: &Path,
    sync_root: &Path,
    entries: &[WindowsCloudExpectedEntry],
) {
    let state = snapshot_windows_cloud_local_state(sync_root, entries);
    let value = json!({ "entries": state });
    if let Ok(raw) = serde_json::to_string(&value) {
        let _ = std::fs::create_dir_all(config_dir);
        let _ = std::fs::write(config_dir.join(WINDOWS_CLOUD_LOCAL_STATE_FILE), raw);
    }
}

fn snapshot_windows_cloud_local_state(
    sync_root: &Path,
    entries: &[WindowsCloudExpectedEntry],
) -> Vec<WindowsCloudLocalStateEntry> {
    let mut state = Vec::new();
    for entry in entries {
        if iris_drive_core::path_has_ignored_component(&entry.path) {
            continue;
        }
        let full_path = windows_cloud_full_path(sync_root, &entry.path);
        if entry.kind == "directory" {
            if full_path.is_dir() {
                state.push(WindowsCloudLocalStateEntry {
                    path: entry.path.clone(),
                    kind: "directory".to_string(),
                    size: 0,
                    sha256: None,
                });
            }
            continue;
        }
        if !full_path.is_file() {
            continue;
        }
        if windows_cloud_path_is_reparse_point(&full_path) {
            state.push(WindowsCloudLocalStateEntry {
                path: entry.path.clone(),
                kind: "file".to_string(),
                size: entry.size,
                sha256: None,
            });
            continue;
        }
        if let Ok(Some(snapshot)) = windows_cloud_snapshot_local_file(&full_path) {
            state.push(WindowsCloudLocalStateEntry {
                path: entry.path.clone(),
                kind: "file".to_string(),
                size: snapshot.size,
                sha256: Some(snapshot.sha256),
            });
        }
    }
    state.sort_by(|a, b| a.path.cmp(&b.path));
    state
}

fn windows_cloud_full_path(root: &Path, virtual_path: &str) -> PathBuf {
    let mut full_path = root.to_path_buf();
    for part in virtual_path.split('/').filter(|part| !part.is_empty()) {
        full_path.push(part);
    }
    full_path
}

fn windows_cloud_relative_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            std::path::Component::Normal(part) => {
                let part = part.to_string_lossy();
                if part.is_empty() || part == "." || part == ".." {
                    return None;
                }
                parts.push(part.into_owned());
            }
            _ => return None,
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("/"))
    }
}

fn windows_cloud_path_is_reparse_point(path: &Path) -> bool {
    std::fs::symlink_metadata(path)
        .map(|metadata| windows_cloud_metadata_is_reparse_point(&metadata))
        .unwrap_or(false)
}

fn windows_cloud_file_read_should_skip(error: &std::io::Error) -> bool {
    if matches!(
        error.kind(),
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::WouldBlock
    ) {
        return true;
    }

    // Cloud Files placeholders can report cloud-specific read errors even when
    // the regular reparse-point bit is not enough to identify them.
    matches!(
        error.raw_os_error(),
        Some(395 | 396 | 397 | 398 | 400 | 402)
    )
}

#[cfg(windows)]
fn windows_cloud_metadata_is_reparse_point(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn windows_cloud_metadata_is_reparse_point(_metadata: &std::fs::Metadata) -> bool {
    false
}

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
        let status = json!({
            "event": "relay_statuses",
            "relay_statuses": relay_statuses,
            "fips_block_sync": fips_status,
        });
        write_daemon_status(&config_dir, status.clone());
        println!("{status}");
    });
}

pub(crate) async fn relay_status_payload(client: &nostr_sdk::Client) -> Vec<serde_json::Value> {
    let relays = client.relays().await;
    let mut payload = Vec::with_capacity(relays.len());
    for (url, relay) in relays {
        let url = normalize_relay_url(url.as_ref());
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
    Some(json!({
        "endpoint_npub": sync.endpoint_npub(),
        "discovery_scope": sync.discovery_scope(),
        "nostr_discovery_app": sync.nostr_discovery_app(),
        "udp_enabled": transport.enable_udp,
        "udp_bind_addr": transport.udp_bind_addr.as_deref(),
        "udp_public": transport.udp_public,
        "udp_external_addr": transport.udp_external_addr.as_deref(),
        "webrtc_enabled": transport.enable_webrtc,
        "mesh_peer_count": sync.mesh_peer_count().await,
        "mesh_peers": sync.mesh_peer_ids().await,
        "authorized_peers": sync.authorized_peer_ids().await,
        "connected_peers": sync.connected_peer_ids().await,
        "relay_statuses": sync.fips_relay_statuses().await,
    }))
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

pub(crate) async fn apply_one_event(
    _client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    event: &nostr_sdk::Event,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
) -> Result<()> {
    use iris_drive_core::relay_sync;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let kind = event.kind.as_u16();
    if kind == iris_drive_core::nostr_events::KIND_APP_KEYS
        && event.identifier() == Some(iris_drive_core::nostr_events::D_TAG_APP_KEYS)
    {
        let outcome = relay_sync::apply_remote_app_keys_event(&mut config, event)?;
        println!(
            "{}",
            json!({
                "event": "app_keys",
                "event_id": event.id.to_hex(),
                "outcome": format!("{outcome:?}"),
            })
        );
        if let Some(sync) = fips_blocks.as_deref() {
            sync.refresh_authorized_peers(&config).await;
        }
    } else if kind == iris_drive_core::nostr_events::KIND_HASHTREE_ROOT {
        let Some(account_state) = config.account.clone() else {
            return Ok(());
        };
        return apply_files_root_event(
            config_dir,
            event,
            fips_blocks,
            mount_refresh,
            &mut config,
            account_state,
        );
    } else if kind == iris_drive_core::nostr_events::KIND_DRIVE_ROOT {
        let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
        let parsed =
            iris_drive_core::nostr_events::parse_drive_root_event_for_device(event, device.keys())
                .ok();
        let outcome =
            relay_sync::apply_remote_drive_root_event(&mut config, event, Some(device.keys()))?;
        let was_applied = matches!(outcome, relay_sync::DriveRootApply::Applied);
        let stale_current_root = matches!(outcome, relay_sync::DriveRootApply::StaleTimestamp)
            && parsed
                .as_ref()
                .is_some_and(|(device_pubkey, _, drive_id, root_ref)| {
                    config
                        .drive(drive_id)
                        .and_then(|drive| drive.device_roots.get(device_pubkey))
                        .is_some_and(|stored| stored.root_cid == root_ref.root_cid)
                });
        let root_cid_to_pull = parsed
            .as_ref()
            .filter(|_| was_applied || stale_current_root)
            .map(|(_, _, _, root_ref)| root_ref.root_cid.clone());
        emit_daemon_status_event(
            config_dir,
            json!({
                "event": "drive_root",
                "event_id": event.id.to_hex(),
                "author": account_npub(&event.pubkey.to_hex()),
                "outcome": format!("{outcome:?}"),
                "root_cid": root_cid_to_pull.clone(),
            }),
        );
        config.save(config_path_in(config_dir))?;
        if let Some(sync) = fips_blocks.as_deref() {
            sync.refresh_authorized_peers(&config).await;
        }

        spawn_root_apply_followup(
            config_dir.to_path_buf(),
            config.clone(),
            root_cid_to_pull,
            fips_blocks,
            was_applied || stale_current_root,
            "materialized_drive_root",
            mount_refresh,
        );
        return Ok(());
    } else {
        // Unknown kind; ignore.
        return Ok(());
    }
    config.save(config_path_in(config_dir))?;
    Ok(())
}

pub(crate) fn apply_files_root_event(
    config_dir: &std::path::Path,
    event: &nostr_sdk::Event,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
    config: &mut AppConfig,
    account_state: AccountState,
) -> Result<()> {
    use iris_drive_core::relay_sync;
    if !account_state.has_owner_signing_authority {
        println!(
            "{}",
            json!({
                "event": "files_root",
                "event_id": event.id.to_hex(),
                "author": account_npub(&event.pubkey.to_hex()),
                "outcome": "owner_key_unavailable",
            })
        );
        return Ok(());
    }
    let account = Account::load(account_state, config_dir).context("loading owner account")?;
    let owner_keys = account
        .owner_key
        .as_ref()
        .map(iris_drive_core::OwnerKey::keys);
    let outcome = relay_sync::apply_remote_files_root_event(config, event, owner_keys)?;
    let was_applied = matches!(outcome, relay_sync::FilesRootApply::Applied);
    let root_cid_to_pull = if was_applied {
        config
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .and_then(|drive| drive.device_roots.get(&account.state.device_pubkey))
            .map(|root| root.root_cid.clone())
    } else {
        None
    };
    emit_daemon_status_event(
        config_dir,
        json!({
            "event": "files_root",
            "event_id": event.id.to_hex(),
            "author": account_npub(&event.pubkey.to_hex()),
            "outcome": files_root_apply_label(&outcome),
            "root_cid": root_cid_to_pull.clone(),
        }),
    );
    config.save(config_path_in(config_dir))?;
    spawn_root_apply_followup(
        config_dir.to_path_buf(),
        config.clone(),
        root_cid_to_pull,
        fips_blocks,
        was_applied,
        "materialized_files_root",
        mount_refresh,
    );
    Ok(())
}

pub(crate) fn spawn_root_apply_followup(
    config_dir: PathBuf,
    config: AppConfig,
    root_cid_to_pull: Option<String>,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    should_materialize: bool,
    materialize_event: &'static str,
    mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
) {
    if root_cid_to_pull.is_none() && !should_materialize {
        return;
    }

    tokio::spawn(async move {
        if let Some(root_cid) = root_cid_to_pull
            && let Err(error) = pull_blocks_for_root_bounded(
                &config_dir,
                &config,
                &root_cid,
                fips_blocks.as_deref(),
            )
            .await
        {
            println!(
                "{}",
                json!({"event": "block_download_error", "error": error})
            );
        }

        if should_materialize {
            if let Some(tx) = mount_refresh {
                if tx.send(materialize_event).await.is_err() {
                    println!(
                        "{}",
                        json!({"event": "mount_refresh_error", "error": "mount refresh worker stopped"})
                    );
                }
                return;
            }
            println!(
                "{}",
                json!({"event": "mount_refresh_skipped", "reason": "no_virtual_mount"})
            );
        }
    });
}

pub(crate) async fn pull_blocks_for_root(
    config_dir: &std::path::Path,
    config: &AppConfig,
    root_cid_str: &str,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<()> {
    let cid =
        Cid::parse(root_cid_str).with_context(|| format!("parsing root cid {root_cid_str}"))?;
    let mut attempted = false;
    let mut errors = Vec::new();
    if let Some(sync) = fips_blocks {
        let connected_peers = sync.connected_peer_ids().await;
        let mesh_peers = sync.mesh_peer_ids().await;
        if connected_peers.is_empty() && mesh_peers.is_empty() {
            println!(
                "{}",
                json!({
                    "event": "fips_download_skipped",
                    "root_cid": root_cid_str,
                    "reason": "no_connected_peers",
                })
            );
        } else {
            attempted = true;
            match download_tree_over_fips_with_retry(sync, &cid, fips_download_policy(config)).await
            {
                Ok(report) => {
                    record_block_sync(config_dir, root_cid_str, "fips", &report);
                    println!(
                        "{}",
                        json!({
                            "event": "fips_downloaded",
                            "root_cid": root_cid_str,
                            "report": download_report_json(&report),
                        })
                    );
                    return Ok(());
                }
                Err(error) => {
                    let error = format!("{error:#}");
                    errors.push(format!("fips: {error}"));
                    println!(
                        "{}",
                        json!({
                            "event": "fips_download_error",
                            "root_cid": root_cid_str,
                            "error": error,
                        })
                    );
                }
            }
        }
    }

    if !config.blossom_servers.is_empty() {
        attempted = true;
        match download_roots_over_blossom(config_dir, config, &[root_cid_str.to_string()]).await {
            Ok(report) => {
                record_block_sync(config_dir, root_cid_str, "blossom", &report);
                println!(
                    "{}",
                    json!({
                        "event": "blossom_downloaded",
                        "root_cid": root_cid_str,
                        "report": download_report_json(&report),
                    })
                );
                return Ok(());
            }
            Err(error) => {
                let error = error.to_string();
                errors.push(format!("blossom: {error}"));
                println!(
                    "{}",
                    json!({
                        "event": "blossom_download_error",
                        "root_cid": root_cid_str,
                        "error": error,
                    })
                );
            }
        }
    }

    if attempted {
        Err(anyhow::anyhow!(
            "all block download transports failed for {root_cid_str}: {}",
            errors.join("; ")
        ))
    } else {
        Err(anyhow::anyhow!(
            "no block download transport available for {root_cid_str}"
        ))
    }
}

pub(crate) async fn pull_blocks_for_root_bounded(
    config_dir: &std::path::Path,
    config: &AppConfig,
    root_cid_str: &str,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> std::result::Result<(), String> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(EVENT_BLOCK_PULL_TIMEOUT_SECS),
        pull_blocks_for_root(config_dir, config, root_cid_str, fips_blocks),
    )
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err(format!("timed out after {EVENT_BLOCK_PULL_TIMEOUT_SECS}s")),
    }
}

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
    let mut devices: Vec<String> = state
        .app_keys
        .as_ref()
        .map(|snap| snap.devices.iter().map(|d| d.pubkey.clone()).collect())
        .unwrap_or_default();
    if !devices.contains(&state.device_pubkey) {
        devices.push(state.device_pubkey.clone());
    }
    devices
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
    use super::*;

    async fn fresh_test_provider() -> (tempfile::TempDir, HashTreeProviderFs<FsBlobStore>) {
        let dir = tempfile::tempdir().unwrap();
        let store = FsBlobStore::new(dir.path()).unwrap();
        let tree = Arc::new(HashTree::new(HashTreeConfig::new(Arc::new(store)).public()));
        let provider = HashTreeProviderFs::fresh(tree).await.unwrap();
        (dir, provider)
    }

    #[test]
    fn windows_cloud_file_read_skip_only_ignores_transient_placeholder_errors() {
        assert!(windows_cloud_file_read_should_skip(&std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "hydrating placeholder"
        )));
        assert!(windows_cloud_file_read_should_skip(
            &std::io::Error::from_raw_os_error(395)
        ));
        assert!(!windows_cloud_file_read_should_skip(&std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "real missing file"
        )));
    }

    #[test]
    fn windows_cloud_rescan_detects_deleted_cached_placeholder_paths() {
        let sync_root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(sync_root.path().join("present")).unwrap();
        std::fs::write(sync_root.path().join("present").join("file.txt"), b"keep").unwrap();
        let cached = BTreeSet::from([
            "gone".to_string(),
            "gone/child.txt".to_string(),
            "gone.txt".to_string(),
            "present".to_string(),
            "present/file.txt".to_string(),
        ]);

        let missing =
            windows_cloud_missing_cached_provider_paths(sync_root.path(), &cached).unwrap();

        assert_eq!(
            missing,
            vec![
                "gone".to_string(),
                "gone.txt".to_string(),
                "gone/child.txt".to_string(),
            ]
        );
    }

    #[test]
    fn windows_cloud_local_state_loads_pascal_case_cache() {
        let config_dir = tempfile::tempdir().unwrap();
        std::fs::write(
            config_dir.path().join(WINDOWS_CLOUD_LOCAL_STATE_FILE),
            r#"{"entries":[{"Path":"remote.txt","Kind":"file","Size":4,"Sha256":"abcd"},{"Path":".Trash-1000/nope","Kind":"file","Size":1,"Sha256":"eeee"}]}"#,
        )
        .unwrap();

        let state = load_windows_cloud_local_state(config_dir.path());

        assert_eq!(
            state,
            vec![WindowsCloudLocalStateEntry {
                path: "remote.txt".to_string(),
                kind: "file".to_string(),
                size: 4,
                sha256: Some("abcd".to_string()),
            }]
        );
    }

    #[test]
    fn windows_cloud_stale_cleanup_removes_unchanged_synced_file() {
        let sync_root = tempfile::tempdir().unwrap();
        let path = sync_root.path().join("remote.txt");
        std::fs::write(&path, b"same").unwrap();
        let state = vec![WindowsCloudLocalStateEntry {
            path: "remote.txt".to_string(),
            kind: "file".to_string(),
            size: 4,
            sha256: Some(to_hex(&hashtree_core::sha256(b"same"))),
        }];

        let removed = windows_cloud_remove_stale_synced_local_items(
            sync_root.path(),
            &BTreeSet::new(),
            &state,
        );

        assert_eq!(removed, vec!["remote.txt".to_string()]);
        assert!(!path.exists());
    }

    #[test]
    fn windows_cloud_stale_cleanup_preserves_local_edit() {
        let sync_root = tempfile::tempdir().unwrap();
        let path = sync_root.path().join("remote.txt");
        std::fs::write(&path, b"edited").unwrap();
        let state = vec![WindowsCloudLocalStateEntry {
            path: "remote.txt".to_string(),
            kind: "file".to_string(),
            size: 4,
            sha256: Some(to_hex(&hashtree_core::sha256(b"same"))),
        }];

        let removed = windows_cloud_remove_stale_synced_local_items(
            sync_root.path(),
            &BTreeSet::new(),
            &state,
        );

        assert!(removed.is_empty());
        assert!(path.exists());
    }

    #[tokio::test]
    async fn windows_cloud_upsert_prunes_ignored_local_tree_from_provider() {
        let (_blocks, provider) = fresh_test_provider().await;
        write_provider_file(&provider, ".Trash-1000/files/removed.txt", b"trash")
            .await
            .unwrap();
        write_provider_file(&provider, "keep.txt", b"keep")
            .await
            .unwrap();

        let sync_root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(sync_root.path().join(".Trash-1000").join("files")).unwrap();
        std::fs::write(
            sync_root
                .path()
                .join(".Trash-1000")
                .join("files")
                .join("removed.txt"),
            b"trash",
        )
        .unwrap();

        assert!(
            apply_windows_cloud_upsert(
                &provider,
                sync_root.path(),
                ".Trash-1000",
                &BTreeSet::new(),
            )
            .await
            .unwrap()
        );
        let trash = ".Trash-1000".to_string();
        let keep = "keep.txt".to_string();
        assert!(provider.item(&trash).await.is_err());
        assert!(provider.item(&keep).await.is_ok());
    }

    #[tokio::test]
    async fn windows_cloud_provider_prune_removes_ignored_merged_paths() {
        let (_blocks, provider) = fresh_test_provider().await;
        write_provider_file(&provider, "noise/.DS_Store", b"finder")
            .await
            .unwrap();
        write_provider_file(&provider, "$RECYCLE.BIN/S-1-5-21/removed.txt", b"recycle")
            .await
            .unwrap();
        write_provider_file(&provider, "keep.txt", b"keep")
            .await
            .unwrap();

        let pruned = prune_ignored_provider_paths(&provider).await.unwrap();

        assert_eq!(
            pruned,
            vec!["$RECYCLE.BIN".to_string(), "noise/.DS_Store".to_string()]
        );
        let recycle = "$RECYCLE.BIN".to_string();
        let noise = "noise".to_string();
        let keep = "keep.txt".to_string();
        assert!(provider.item(&recycle).await.is_err());
        assert!(provider.item(&noise).await.is_ok());
        assert!(provider.item(&keep).await.is_ok());
    }

    #[tokio::test]
    async fn windows_cloud_upsert_skips_matching_existing_file() {
        let (_blocks, provider) = fresh_test_provider().await;
        write_provider_file(&provider, "remote.txt", b"same")
            .await
            .unwrap();
        let before = provider.current_root().await;

        let sync_root = tempfile::tempdir().unwrap();
        std::fs::write(sync_root.path().join("remote.txt"), b"same").unwrap();

        assert!(
            !apply_windows_cloud_upsert(
                &provider,
                sync_root.path(),
                "remote.txt",
                &BTreeSet::new(),
            )
            .await
            .unwrap()
        );
        assert_eq!(provider.current_root().await, before);
    }

    #[tokio::test]
    async fn windows_cloud_upsert_skips_existing_directory() {
        let (_blocks, provider) = fresh_test_provider().await;
        create_provider_dir(&provider, "existing").await.unwrap();
        let before = provider.current_root().await;

        let sync_root = tempfile::tempdir().unwrap();
        std::fs::create_dir(sync_root.path().join("existing")).unwrap();

        assert!(
            !apply_windows_cloud_upsert(&provider, sync_root.path(), "existing", &BTreeSet::new(),)
                .await
                .unwrap()
        );
        assert_eq!(provider.current_root().await, before);
    }

    #[tokio::test]
    async fn windows_cloud_upsert_skips_stale_cached_placeholder() {
        let (_blocks, provider) = fresh_test_provider().await;
        let before = provider.current_root().await;

        let sync_root = tempfile::tempdir().unwrap();
        std::fs::write(sync_root.path().join("remote-deleted.txt"), b"stale").unwrap();
        let placeholder_paths = BTreeSet::from(["remote-deleted.txt".to_string()]);

        assert!(
            !apply_windows_cloud_upsert(
                &provider,
                sync_root.path(),
                "remote-deleted.txt",
                &placeholder_paths,
            )
            .await
            .unwrap()
        );
        assert_eq!(provider.current_root().await, before);
        assert!(
            provider
                .item(&"remote-deleted.txt".to_string())
                .await
                .is_err()
        );
    }
}
