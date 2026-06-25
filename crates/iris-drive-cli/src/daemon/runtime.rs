#[allow(clippy::needless_pass_by_value)]
fn emit_daemon_status_event(config_dir: &Path, payload: Value) {
    let payload = write_runtime_daemon_status(config_dir, payload);
    println!("{payload}");
}

#[allow(clippy::needless_pass_by_value)]
fn write_runtime_daemon_status(config_dir: &Path, payload: Value) -> Value {
    write_daemon_status(config_dir, payload)
}

const DIRECT_APP_MESSAGE_DRAIN_LIMIT: usize = 4096;
const DIRECT_ROOT_CHANGE_ANNOUNCE_COALESCE_MS: u64 = 750;

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
    service_mode: bool,
    gateway_port: u16,
    enable_gateway: bool,
    mount_drive: bool,
    mountpoint: Option<PathBuf>,
) -> Result<()> {
    use iris_drive_core::relay_sync;
    use nostr_sdk::RelayPoolNotification;
    use tokio::sync::broadcast::error::{RecvError, TryRecvError};
    use tokio::sync::mpsc;

    let _daemon_lock = DaemonProcessLock::acquire(config_dir)?;

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .context("building tokio runtime")?;

    let config = AppConfig::load_or_default_cached_profile(config_path_in(config_dir))?;
    let state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    if state.authorization_state == iris_drive_core::AppKeyAuthorizationState::Revoked {
        write_runtime_daemon_status(config_dir, json!({
            "event": "revoked",
            "error": "device removed",
        }));
        return Err(anyhow::anyhow!(
            "this device has been removed from Iris Drive; link it again or log out"
        ));
    }
    let relays = pick_relays(&config, relay_override);
    let root_scope_id = state.root_scope_id();
    let share_ids = config
        .shared_folders
        .iter()
        .map(|folder| folder.share_id)
        .collect::<Vec<_>>();
    let filters = relay_sync::subscription_filters_for_shared_roots(
        &state.app_key_pubkey,
        &root_scope_id,
        iris_drive_core::PRIMARY_DRIVE_ID,
        &share_ids,
    );
    if filters.is_empty() {
        return Err(anyhow::anyhow!("no filters to subscribe to"));
    }
    let embedded_hashtree_requested = enable_gateway && config.local_nhash_resolver_enabled;
    let (embedded_hashtree, embedded_hashtree_status) = if embedded_hashtree_requested {
        match EmbeddedHashtreeHost::start(config_dir, &config) {
            Ok(host) => {
                let status = host.status_payload();
                (Some(host), status)
            }
            Err(error) => {
                let error = format!("{error:#}");
                println!(
                    "{}",
                    json!({
                        "event": "embedded_hashtree_unavailable",
                        "error": error,
                    })
                );
                (None, json!({"running": false, "error": error}))
            }
        }
    } else {
        let disabled_by = if enable_gateway { "settings" } else { "cli" };
        (
            None,
            json!({
                "running": false,
                "disabled_by": disabled_by,
            }),
        )
    };

    runtime.block_on(async {
        let mut block_config = config.clone();
        block_config.relays = relays.clone();
        let daemon_tasks = DaemonTaskSet::default();
        let (fips_blocks, fips_block_sync_error) =
            match start_fips_block_sync(config_dir, &block_config).await {
                Ok(sync) => (Some(Arc::new(sync)), None),
                Err(error) => (None, Some(error.to_string())),
            };
        let mut direct_app_message_rx = fips_blocks
            .as_ref()
            .map(|sync| sync.subscribe_app_messages());
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
        let (mut config_root_change_rx, _config_root_watcher, config_root_watch_status) =
            match start_config_root_watch(config_dir) {
                Ok((rx, watcher, status)) => (Some(rx), Some(watcher), Some(status)),
                Err(error) => {
                    println!(
                        "{}",
                        json!({"event": "config_root_watch_error", "error": format!("{error:#}")})
                    );
                    (None, None, Some(json!({
                        "watching": false,
                        "error": format!("{error:#}"),
                    })))
                }
            };
        let (mut provider_root_wake_rx, provider_root_wake_status) =
            match start_provider_root_wake_listener(config_dir).await {
                Ok((rx, task, status)) => {
                    daemon_tasks.push(task);
                    (Some(rx), Some(status))
                }
                Err(error) => {
                    println!(
                        "{}",
                        json!({"event": "provider_root_wake_error", "error": format!("{error:#}")})
                    );
                    (None, Some(json!({
                        "running": false,
                        "error": format!("{error:#}"),
                    })))
                }
            };
        let gateway_enabled = embedded_hashtree_requested && embedded_hashtree.is_some();
        let gateway_disabled_by = if !enable_gateway {
            Some("cli")
        } else if !config.local_nhash_resolver_enabled {
            Some("settings")
        } else if embedded_hashtree.is_none() {
            Some("embedded_hashtree")
        } else {
            None
        };
        let gateway = if gateway_enabled {
            let embedded_hashtree = embedded_hashtree.as_ref().ok_or_else(|| {
                anyhow::anyhow!("gateway was enabled without an embedded hashtree")
            })?;
            let daemon = Daemon::open(config_dir).context("opening daemon for browser gateway")?;
            Some(
                GatewayServer::bind_with_tree_and_htree_daemon(
                    config_dir,
                    daemon.tree_handle(),
                    embedded_hashtree.status().base_url.clone(),
                    GatewayBind::loopback_v4(gateway_port),
                )
                    .await
                    .context("starting browser gateway")?,
            )
        } else {
            None
        };
        let gateway_status = if let Some(server) = gateway.as_ref() {
            let port = server.local_addr().port();
            json!({
                "enabled": true,
                "running": true,
                "bind": server.local_addr().to_string(),
                "portal_url": iris_drive_core::gateway::local_portal_url(port),
                "primary_drive_url": iris_drive_core::gateway::local_drive_url(
                    port,
                    iris_drive_core::PRIMARY_DRIVE_ID,
                ),
                "caldav_url": iris_drive_core::gateway::local_caldav_url_for_identity(
                    port,
                    &pubkey_npub(&state.app_key_pubkey),
                ),
                "nhash_resolver_url": format!(
                    "http://{}:{port}/",
                    iris_drive_core::gateway::LOCAL_NHASH_RESOLVER_HOST,
                ),
                "hashtree_base_url": embedded_hashtree
                    .as_ref()
                    .map(|host| host.status().base_url.clone()),
            })
        } else {
            json!({
                "enabled": false,
                "requested": embedded_hashtree_requested,
                "running": false,
                "disabled_by": gateway_disabled_by,
                "host": iris_drive_core::gateway::LOCAL_NHASH_RESOLVER_HOST,
                "port": gateway_port,
            })
        };
        let client = relay_sync::connect(&relays)
            .await
            .context("connecting to relays")?;
        let relay_statuses = relay_status_payload(&client).await;
        for filter in filters {
            client
                .subscribe(filter, None)
                .await
                .context("opening subscription")?;
        }
        let mut notifications = client.notifications();
        let mut relay_notifications_open = true;
        let mut direct_roots = DirectRootExchange::default();
        let startup_fips_block_sync_status = fips_block_sync_status(fips_blocks.as_deref()).await;
        let mut config = config.clone();
        let mut mounted_drive = if mount_drive {
            let mountpoint = mountpoint
                .clone()
                .unwrap_or_else(|| default_mountpoint_in(config_dir));
            let mounted = mount::start_iris_drive_mount(config_dir, mountpoint).await?;
            config = AppConfig::load_or_default_cached_profile(config_path_in(config_dir))?;
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
        let root_update_debounce = root_update_debounce_duration(watch_debounce_ms);
        let mut subscribed_status = json!({
                "event": "subscribed",
                "relays": relays,
                "current_app_key_npub": pubkey_npub(&state.app_key_pubkey),
                "provider_update_mode": "event_driven",
                "supervision": if service_mode { "service" } else { "app" },
                "watch_debounce_ms": watch_debounce_ms,
                "root_update_throttle_ms": root_update_debounce.as_millis(),
                "mount": mount_status,
                "relay_statuses": relay_statuses,
                "embedded_hashtree": embedded_hashtree_status,
                "browser_gateway": gateway_status,
                "windows_cloud_root": windows_cloud_status,
                "config_root_watch": config_root_watch_status,
                "provider_root_wake": provider_root_wake_status,
                "fips_block_sync": startup_fips_block_sync_status,
                "fips_block_sync_error": fips_block_sync_error,
        });
        if let Some(hashtree) = primary_drive_status_payload(config_dir, &config).await {
            subscribed_status["hashtree"] = hashtree;
        }
        let subscribed_status = write_runtime_daemon_status(config_dir, subscribed_status);
        println!("{subscribed_status}");
        spawn_daemon_heartbeat(config_dir.to_path_buf());

        let startup_config = config.clone();
        let startup_state = state.clone();
        for root_cid in startup_root_cids_needing_sync(config_dir, &config) {
            enqueue_root_apply_followup(
                config_dir.to_path_buf(),
                config.clone(),
                Some(root_cid),
                fips_blocks.clone(),
                true,
                "startup_root_sync",
                mount_refresh_tx.clone(),
                &daemon_tasks,
            );
        }
        enqueue_root_apply_followup(
            config_dir.to_path_buf(),
            config.clone(),
            None,
            fips_blocks.clone(),
            true,
            "startup_projection",
            mount_refresh_tx.clone(),
            &daemon_tasks,
        );
        daemon_tasks.push(spawn_initial_publish(
            client.clone(),
            config_dir.to_path_buf(),
            startup_config,
            startup_state,
        ));
        if let Err(error) =
            announce_current_state_direct(&mut direct_roots, config_dir, fips_blocks.as_deref())
                .await
        {
            println!(
                "{}",
                json!({"event": "direct_root_mesh_error", "error": format!("{error:#}")})
            );
        }
        println!("(running — Ctrl+C to stop)");

        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        let parent_exit = parent_exit_signal(service_mode);
        tokio::pin!(parent_exit);

        let relay_status_period = std::time::Duration::from_secs(10);
        let mut relay_status_timer = tokio::time::interval_at(
            tokio::time::Instant::now() + relay_status_period,
            relay_status_period,
        );
        relay_status_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let direct_root_announce_period =
            std::time::Duration::from_secs(DIRECT_ROOT_PERIODIC_ANNOUNCE_SECS);
        let mut direct_root_announce_timer = tokio::time::interval_at(
            tokio::time::Instant::now() + direct_root_announce_period,
            direct_root_announce_period,
        );
        direct_root_announce_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut direct_root_change_announce_pending = false;
        let mut direct_root_change_announce_timer =
            Box::pin(tokio::time::sleep(std::time::Duration::from_secs(86_400)));
        let config_root_watch_active = config_root_change_rx.is_some();
        let mut provider_root_poll_timer =
            tokio::time::interval(provider_root_poll_period(watch_interval));
        provider_root_poll_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut app_key_link_timer = tokio::time::interval_at(
            tokio::time::Instant::now(),
            std::time::Duration::from_millis(APP_KEY_LINK_TICK_MILLIS),
        );
        app_key_link_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut sent_app_key_link_requests = BTreeMap::new();
        let mut app_key_link_request_config_cache = AppConfigLoadCache::default();
        let mut sent_app_key_link_rosters = AuthorizedAppKeyLinkRosterSendCache::default();
        let mut acked_app_key_link_rosters = BTreeSet::new();
        let mut last_provider_root_key = current_app_key_root_key(&config);
        let mut provider_root_publish_cache = ProviderRootPublishCache::default();

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
                recv = async {
                    if relay_notifications_open {
                        Some(notifications.recv().await)
                    } else {
                        std::future::pending::<Option<Result<RelayPoolNotification, RecvError>>>().await
                    }
                } => {
                    match recv {
                        Some(Ok(RelayPoolNotification::Event { event, .. })) => {
                            match apply_one_event(
                                &client,
                                config_dir,
                                &event,
                                fips_blocks.clone(),
                                mount_refresh_tx.clone(),
                                &daemon_tasks,
                            )
                            .await
                            {
                                Ok(outcome) if outcome.should_announce_current_state() => {
                                    direct_roots.invalidate_current_sync_events_cache();
                                    direct_root_change_announce_pending = true;
                                    direct_root_change_announce_timer.as_mut().reset(
                                        tokio::time::Instant::now()
                                            + std::time::Duration::from_millis(
                                                DIRECT_ROOT_CHANGE_ANNOUNCE_COALESCE_MS,
                                            ),
                                    );
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    println!(
                                        "{}",
                                        json!({"event": "apply_error", "id": event.id.to_hex(), "error": e.to_string()})
                                    );
                                }
                            }
                        }
                        Some(Ok(RelayPoolNotification::Shutdown)) => {
                            relay_notifications_open = false;
                            println!("{}", json!({"event": "relay_notifications_closed", "reason": "shutdown"}));
                        }
                        Some(Ok(_)) | None => {}
                        Some(Err(RecvError::Closed)) => {
                            relay_notifications_open = false;
                            println!("{}", json!({"event": "relay_notifications_closed", "reason": "closed"}));
                        }
                        Some(Err(RecvError::Lagged(n))) => {
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
                        let Some(drained_visible_root) = drain_latest_mount_root_update(
                            rx,
                            root_update_debounce,
                            Some(visible_root),
                        )
                        .await
                        else {
                            println!("{}", json!({"event": "mount_root_update_missing"}));
                            continue;
                        };
                        visible_root = drained_visible_root;
                    }
                    match import_mount_visible_root_update(
	                        &client,
	                        config_dir,
	                        visible_root,
	                        &mut mount_tombstone_base,
	                        &mut direct_roots,
	                        fips_blocks.as_deref(),
	                        &daemon_tasks,
	                    )
                    .await
                    {
                        Ok(()) => update_last_provider_root_key(config_dir, &mut last_provider_root_key),
                        Err(error) => println!(
                            "{}",
                            json!({"event": "mount_publish_error", "error": format!("{error:#}")})
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
                        tokio::time::sleep(root_update_debounce).await;
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
	                            &daemon_tasks,
	                        )
                        .await
                        {
                            Ok(WindowsCloudImportOutcome::Changed { root_cid, paths }) => {
                                update_last_provider_root_key(config_dir, &mut last_provider_root_key);
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
                Some(()) = async {
                    if let Some(rx) = config_root_change_rx.as_mut() {
                        rx.recv().await
                    } else {
                        std::future::pending::<Option<()>>().await
                    }
                } => {
                    if let Some(rx) = config_root_change_rx.as_mut() {
                        while rx.try_recv().is_ok() {}
                        tokio::time::sleep(root_update_debounce).await;
                        while rx.try_recv().is_ok() {}
                    }
                    if let Some(rx) = provider_root_wake_rx.as_mut()
                        && let Some(wake_payload) =
                            drain_latest_provider_root_wake_payload(rx, None)
                        && let Some(status_payload) =
                            provider_root_wake_status_payload(&wake_payload)
                    {
                        emit_daemon_status_event(config_dir, status_payload);
                    }
                    match publish_provider_root_if_changed(
                        &client,
	                        config_dir,
	                        &mut last_provider_root_key,
	                        &mut provider_root_publish_cache,
	                        &mut direct_roots,
	                        fips_blocks.as_deref(),
	                        &daemon_tasks,
	                    )
                    .await
                    {
                        Ok(Some(_updated_config)) => {}
                        Ok(None) => {}
                        Err(error) => println!(
                            "{}",
                            json!({"event": "provider_root_publish_error", "trigger": "config_root_watch", "error": format!("{error:#}")})
                        ),
                    }
                }
                Some(wake_payload) = async {
                    if let Some(rx) = provider_root_wake_rx.as_mut() {
                        rx.recv().await
                    } else {
                        std::future::pending::<Option<Option<Value>>>().await
                    }
                } => {
                    let mut latest_wake_payload = wake_payload;
                    if let Some(rx) = provider_root_wake_rx.as_mut() {
                        latest_wake_payload =
                            drain_latest_provider_root_wake_payload(rx, latest_wake_payload);
                    }
                    if let Some(wake_payload) = latest_wake_payload.as_ref()
                        && let Some(status_payload) = provider_root_wake_status_payload(wake_payload)
                    {
                        emit_daemon_status_event(config_dir, status_payload);
                    }
                    match publish_provider_root_if_changed(
                        &client,
	                        config_dir,
	                        &mut last_provider_root_key,
	                        &mut provider_root_publish_cache,
	                        &mut direct_roots,
	                        fips_blocks.as_deref(),
	                        &daemon_tasks,
	                    )
                    .await
                    {
                        Ok(Some(_updated_config)) => {}
                        Ok(None) => {}
                        Err(error) => println!(
                            "{}",
                            json!({"event": "provider_root_publish_error", "trigger": "provider_root_wake", "error": format!("{error:#}")})
                        ),
                    }
                }
                Some(reason) = async {
                    if let Some(rx) = mount_refresh_rx.as_mut() {
                        rx.recv().await
                    } else {
                        std::future::pending::<Option<&'static str>>().await
                    }
                } => {
                    if let Some(rx) = mount_root_updates.as_mut()
                        && let Some(visible_root) =
                            drain_latest_mount_root_update(rx, root_update_debounce, None).await
                    {
                        match import_mount_visible_root_update(
                            &client,
                            config_dir,
	                            visible_root,
	                            &mut mount_tombstone_base,
	                            &mut direct_roots,
	                            fips_blocks.as_deref(),
	                            &daemon_tasks,
	                        )
                        .await
                        {
                            Ok(()) => {
                                update_last_provider_root_key(config_dir, &mut last_provider_root_key);
                                emit_daemon_status_event(config_dir, json!({
                                    "event": "mount_pending_root_imported_before_refresh",
                                    "trigger": reason,
                                }));
                            }
                            Err(error) => {
                                println!(
                                    "{}",
                                    json!({"event": "mount_publish_error", "trigger": reason, "error": format!("{error:#}")})
                                );
                                continue;
                            }
                        }
                    }
                    if let Some(handle) = mount_refresh.as_ref() {
                        let mut attempts = 0;
                        loop {
                            let expected = mount_tombstone_base
                                .clone()
                                .unwrap_or_else(|| handle.current_visible_root());
                            match handle
                                .refresh_from_config_if_current(config_dir, &expected)
                                .await
                            {
                                Ok(mount::MountRefreshOutcome::Refreshed(visible)) => {
                                    mount_tombstone_base = Some(visible.root_cid.clone());
                                    emit_daemon_status_event(config_dir, json!({
                                        "event": "mount_refreshed",
                                        "trigger": reason,
                                        "mountpoint": handle.mountpoint().display().to_string(),
                                        "root_cid": visible.root_cid.to_string(),
                                        "file_count": visible.file_count,
                                        "top_level_entries": visible.top_level_entries,
                                    }));
                                    break;
                                }
                                Ok(mount::MountRefreshOutcome::Dirty(visible_root)) => {
                                    attempts += 1;
                                    match import_mount_visible_root_update(
                                        &client,
                                        config_dir,
	                                        visible_root,
	                                        &mut mount_tombstone_base,
	                                        &mut direct_roots,
	                                        fips_blocks.as_deref(),
	                                        &daemon_tasks,
	                                    )
                                    .await
                                    {
                                        Ok(()) => {
                                            update_last_provider_root_key(
                                                config_dir,
                                                &mut last_provider_root_key,
                                            );
                                            emit_daemon_status_event(config_dir, json!({
                                                "event": "mount_dirty_root_imported_before_refresh",
                                                "trigger": reason,
                                                "attempt": attempts,
                                            }));
                                        }
                                        Err(error) => {
                                            println!(
                                                "{}",
                                                json!({"event": "mount_publish_error", "trigger": reason, "error": format!("{error:#}")})
                                            );
                                            break;
                                        }
                                    }
                                    if attempts >= 4 {
                                        emit_daemon_status_event(config_dir, json!({
                                            "event": "mount_refresh_deferred_dirty",
                                            "trigger": reason,
                                            "attempts": attempts,
                                        }));
                                        break;
                                    }
                                }
                                Err(error) => {
                                    println!(
                                        "{}",
                                        json!({"event": "mount_refresh_error", "trigger": reason, "error": format!("{error:#}")})
                                    );
                                    break;
                                }
                            }
                        }
                    }
                }
                _ = relay_status_timer.tick() => {
                    daemon_tasks.push(spawn_status_probe(
                        client.clone(),
                        config_dir.to_path_buf(),
                        fips_blocks.clone(),
                    ));
                    if let Err(error) = direct_roots
                        .request_roots_from_new_peers(config_dir, fips_blocks.as_deref())
                        .await
                    {
                        println!(
                            "{}",
                            json!({"event": "direct_root_mesh_error", "trigger": "peer_refresh", "error": format!("{error:#}")})
                        );
                    }
                }
                _ = direct_root_announce_timer.tick() => {
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
                _ = &mut direct_root_change_announce_timer, if direct_root_change_announce_pending => {
                    direct_root_change_announce_pending = false;
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
                            json!({"event": "direct_root_mesh_error", "trigger": "changed_event_coalesced", "error": format!("{error:#}")})
                        );
                    }
                }
                _ = provider_root_poll_timer.tick(), if provider_root_poll_enabled(config_root_watch_active) => {
                    match publish_provider_root_if_changed(
                        &client,
	                        config_dir,
	                        &mut last_provider_root_key,
	                        &mut provider_root_publish_cache,
	                        &mut direct_roots,
	                        fips_blocks.as_deref(),
	                        &daemon_tasks,
	                    )
                    .await
                    {
                        Ok(Some(_updated_config)) => {}
                        Ok(None) => {}
                        Err(error) => println!(
                            "{}",
                            json!({"event": "provider_root_publish_error", "trigger": "config_root_poll", "error": format!("{error:#}")})
                        ),
                    }
                }
                _ = app_key_link_timer.tick() => {
                    match send_pending_app_key_link_request(
                        config_dir,
                        &client,
                        fips_blocks.as_deref(),
                        &mut sent_app_key_link_requests,
                        &mut app_key_link_request_config_cache,
                    )
                    .await
                    {
                        Ok(Some(payload)) => println!("{payload}"),
                        Ok(None) => {}
                        Err(error) => println!(
                            "{}",
                            json!({"event": "app_key_link_request_send_error", "error": format!("{error:#}")})
                        ),
                    }
                    match send_authorized_app_key_link_rosters(
                        config_dir,
                        fips_blocks.as_deref(),
                        &mut sent_app_key_link_rosters,
                        &acked_app_key_link_rosters,
                    )
                    .await
                    {
                        Ok(Some(payload)) => println!("{payload}"),
                        Ok(None) => {}
                        Err(error) => println!(
                            "{}",
                            json!({"event": "app_key_link_roster_send_error", "error": format!("{error:#}")})
                        ),
                    }
                }
                recv = async {
                    if let Some(rx) = direct_app_message_rx.as_mut() {
                        Some(rx.recv().await)
                    } else {
                        std::future::pending().await
                    }
                } => {
                    match recv {
                        Some(Ok(message)) => {
                            let mut messages = vec![message];
                            let mut receiver_closed = false;
                            if let Some(rx) = direct_app_message_rx.as_mut() {
                                while messages.len() < DIRECT_APP_MESSAGE_DRAIN_LIMIT {
                                    match rx.try_recv() {
                                        Ok(message) => messages.push(message),
                                        Err(TryRecvError::Empty) => break,
                                        Err(TryRecvError::Lagged(n)) => {
                                            println!("{}", json!({"event": "direct_root_app_lagged", "skipped": n}));
                                        }
                                        Err(TryRecvError::Closed) => {
                                            receiver_closed = true;
                                            break;
                                        }
                                    }
                                }
                            }
                            let received_messages = messages.len();
                            let (messages, coalesced_roots) =
                                iris_drive_core::coalesce_direct_root_app_messages(messages);
                            if coalesced_roots > 0 {
                                println!(
                                    "{}",
                                    json!({
                                        "event": "direct_root_app_coalesced",
                                        "received_messages": received_messages,
                                        "applied_messages": messages.len(),
                                        "skipped_roots": coalesced_roots,
                                    })
                                );
                            }
                            for message in messages {
                                match handle_app_key_link_app_message(
                                    config_dir,
                                    &message,
                                    fips_blocks.as_deref(),
                                    &mut acked_app_key_link_rosters,
                                )
                                .await
                                {
                                    Ok(true) => continue,
                                    Ok(false) => {}
                                    Err(error) => {
                                        println!(
                                            "{}",
                                            json!({"event": "app_key_link_request_receive_error", "error": format!("{error:#}")})
                                        );
                                        continue;
                                    }
                                }
                                if let Some(sync) = fips_blocks.as_ref() {
                                    match direct_roots
                                        .handle_app_message(
                                            &client,
                                            config_dir,
                                            sync.clone(),
                                            mount_refresh_tx.clone(),
                                            &daemon_tasks,
                                            message,
                                        )
                                        .await
                                    {
                                        Ok(true) => {
                                            direct_root_change_announce_pending = true;
                                            direct_root_change_announce_timer.as_mut().reset(
                                                tokio::time::Instant::now()
                                                    + std::time::Duration::from_millis(
                                                        DIRECT_ROOT_CHANGE_ANNOUNCE_COALESCE_MS,
                                                    ),
                                            );
                                        }
                                        Ok(false) => {}
                                        Err(error) => {
                                            println!(
                                                "{}",
                                                json!({"event": "direct_root_app_error", "error": format!("{error:#}")})
                                            );
                                        }
                                    }
                                }
                            }
                            if receiver_closed {
                                direct_app_message_rx = None;
                                println!("{}", json!({"event": "direct_root_app_closed"}));
                            }
                        }
                        Some(Err(RecvError::Lagged(n))) => {
                            println!("{}", json!({"event": "direct_root_app_lagged", "skipped": n}));
                        }
                        Some(Err(RecvError::Closed)) | None => {
                            direct_app_message_rx = None;
                            println!("{}", json!({"event": "direct_root_app_closed"}));
                        }
                    }
                }
                message = async {
                    if let Some(sync) = fips_blocks.clone() {
                        sync.recv_mesh_pubsub_event().await
                    } else {
                        std::future::pending::<iris_drive_core::FipsMeshPubsubEvent>().await
                    }
                } => {
                    if let Some(sync) = fips_blocks.as_ref() {
                        let mut messages = vec![message];
                        messages.extend(sync.drain_mesh_pubsub_events().await);
                        let result = direct_roots
                            .handle_mesh_events(
                                &client,
                                config_dir,
                                sync.clone(),
                                mount_refresh_tx.clone(),
                                &daemon_tasks,
                                messages,
                            )
                            .await;
                        match result {
                            Ok(true) => {
                                direct_root_change_announce_pending = true;
                                direct_root_change_announce_timer.as_mut().reset(
                                    tokio::time::Instant::now()
                                        + std::time::Duration::from_millis(
                                            DIRECT_ROOT_CHANGE_ANNOUNCE_COALESCE_MS,
                                        ),
                                );
                            }
                            Ok(false) => {}
                            Err(error) => {
                                println!(
                                    "{}",
                                    json!({"event": "direct_root_mesh_error", "error": format!("{error:#}")})
                                );
                            }
                        }
                    }
                }
            }
        }
        drop(notifications);
        drop(direct_app_message_rx);
        daemon_tasks.abort_all().await;
        if let Some(sync) = fips_blocks {
            match Arc::try_unwrap(sync) {
                Ok(sync) => {
                    if let Err(error) = sync.shutdown().await {
                        println!(
                            "{}",
                            json!({"event": "fips_block_sync_shutdown_error", "error": format!("{error:#}")})
                        );
                    }
                }
                Err(sync) => {
                    if let Err(error) = sync.shutdown_endpoint().await {
                        println!(
                            "{}",
                            json!({"event": "fips_block_sync_shutdown_error", "error": format!("{error:#}")})
                        );
                    }
                    println!(
                        "{}",
                        json!({"event": "fips_block_sync_shutdown_retained", "owners": Arc::strong_count(&sync)})
                    );
                    std::mem::forget(sync);
                }
            }
        }
        relay_sync::shutdown_client_for_process_exit(client).await;
        Ok::<_, anyhow::Error>(())
    })
}
