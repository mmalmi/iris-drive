#[allow(clippy::wildcard_imports)]
use super::*;

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
        let (webdav_root_tx, mut webdav_root_rx) = mpsc::unbounded_channel::<Cid>();
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
                    match import_mount_root_and_publish(
                        &client,
                        config_dir,
                        visible_root,
                        &mut direct_roots,
                        fips_blocks.as_deref(),
                    )
                    .await
                    {
                        Ok(()) => {}
                        Err(error) => println!(
                            "{}",
                            json!({"event": "mount_publish_error", "error": format!("{error:#}")})
                        ),
                    }
                }
                Some(mut visible_root) = webdav_root_rx.recv() => {
                    while let Ok(next) = webdav_root_rx.try_recv() {
                        visible_root = next;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(watch_debounce_ms)).await;
                    while let Ok(next) = webdav_root_rx.try_recv() {
                        visible_root = next;
                    }
                    match import_mount_root_and_publish(
                        &client,
                        config_dir,
                        visible_root,
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
                Some(reason) = async {
                    if let Some(rx) = mount_refresh_rx.as_mut() {
                        rx.recv().await
                    } else {
                        std::future::pending::<Option<&'static str>>().await
                    }
                } => {
                    if let Some(handle) = mount_refresh.as_ref() {
                        match handle.refresh_from_config(config_dir).await {
                            Ok(visible) => println!(
                                "{}",
                                json!({
                                    "event": "mount_refreshed",
                                    "trigger": reason,
                                    "mountpoint": handle.mountpoint().display().to_string(),
                                    "root_cid": visible.root_cid.to_string(),
                                    "file_count": visible.file_count,
                                    "top_level_entries": visible.top_level_entries,
                                })
                            ),
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
        "udp_enabled": transport.enable_udp,
        "udp_bind_addr": transport.udp_bind_addr.as_deref(),
        "udp_public": transport.udp_public,
        "udp_external_addr": transport.udp_external_addr.as_deref(),
        "webrtc_enabled": transport.enable_webrtc,
        "mesh_peer_count": sync.mesh_peer_count().await,
        "mesh_peers": sync.mesh_peer_ids().await,
        "authorized_peers": sync.authorized_peer_ids().await,
        "connected_peers": sync.connected_peer_ids().await,
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
        println!(
            "{}",
            json!({
                "event": "drive_root",
                "event_id": event.id.to_hex(),
                "author": account_npub(&event.pubkey.to_hex()),
                "outcome": format!("{outcome:?}"),
            })
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
    println!(
        "{}",
        json!({
            "event": "files_root",
            "event_id": event.id.to_hex(),
            "author": account_npub(&event.pubkey.to_hex()),
            "outcome": files_root_apply_label(&outcome),
        })
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
        if connected_peers.is_empty() {
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
