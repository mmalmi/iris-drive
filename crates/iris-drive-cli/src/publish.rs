#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn cmd_publish(
    config_dir: &std::path::Path,
    relay_override: &[String],
    timeout_secs: u64,
) -> Result<()> {
    use iris_drive_core::relay_sync;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let config = AppConfig::load_or_default(config_path_in(config_dir))?;
        let state = config
            .account
            .clone()
            .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
        let relays = pick_relays(&config, relay_override);
        let _ = timeout_secs; // connect timeout not used by add_relay; kept for future
        let client = relay_sync::connect(&relays)
            .await
            .context("connecting to relays")?;

        let report = publish_current_state(&client, config_dir, &config, &state, true).await?;

        let _ = client.disconnect().await;
        let drive_iris_to_url = report
            .root_cid
            .as_ref()
            .and_then(|_| drive_iris_to_url_for_primary_drive(&config));
        let snapshot_url = report
            .root_cid
            .as_deref()
            .and_then(drive_iris_to_snapshot_url_for_root);
        println!(
            "{}",
            json!({
                "relays": relays,
                "blossom_servers": config.blossom_servers,
                "published_profile_roster_ops": report.published_profile_roster_ops,
                "profile_roster_publish_error": report.profile_roster_publish_error,
                "published_drive_root": report.published_drive_root,
                "drive_root_publish_error": report.drive_root_publish_error,
                "published_files_root": report.published_files_root,
                "files_root_publish_error": report.files_root_publish_error,
                "root_cid": report.root_cid,
                "drive_iris_to_url": drive_iris_to_url,
                "files_iris_to_url": drive_iris_to_url,
                "snapshot_url": snapshot_url,
                "permalink_url": snapshot_url,
                "blossom_upload_error": report.blossom_upload_error,
                "blossom_upload": report.blossom_upload.map(|r| json!({
                    "total_hashes": r.total_hashes,
                    "uploaded": r.uploaded,
                    "already_present": r.already_present,
                })),
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}

#[derive(Debug, Default)]
pub(crate) struct PublishStateReport {
    published_profile_roster_ops: usize,
    profile_roster_publish_error: Option<String>,
    published_drive_root: bool,
    drive_root_publish_error: Option<String>,
    published_files_root: bool,
    files_root_publish_error: Option<String>,
    root_cid: Option<String>,
    blossom_upload: Option<UploadReport>,
    blossom_upload_error: Option<String>,
}

include!("publish/direct_root.rs");

pub(crate) async fn announce_current_state_direct(
    direct_roots: &mut DirectRootExchange,
    config_dir: &Path,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(state) = config.account.as_ref() else {
        return Ok(());
    };
    direct_roots
        .announce_current_state(config_dir, &config, state, fips_blocks)
        .await
}

pub(crate) async fn upload_tree_to_blossom_with_hashtree(
    config_dir: &std::path::Path,
    config: &AppConfig,
    device: &iris_drive_core::DeviceIdentity,
    root_cid: Cid,
    _previous_root_cid: Option<Cid>,
) -> Result<UploadReport> {
    if config.blossom_servers.is_empty() {
        return Err(anyhow::anyhow!("no blossom servers configured"));
    }

    let bclient =
        iris_drive_core::blossom_sync_client(device.keys().clone(), &config.blossom_servers);
    let daemon = Daemon::open(config_dir).context("opening daemon for blossom upload")?;
    iris_drive_core::blossom_sync::upload_tree(daemon.tree(), &root_cid, &bclient)
        .await
        .context("uploading tree to blossom")
}

pub(crate) async fn maybe_upload_root_to_blossom(
    config_dir: &std::path::Path,
    config: &AppConfig,
    device: &iris_drive_core::DeviceIdentity,
    root_cid_str: &str,
    previous_root_cid: Option<&str>,
) -> Result<(Option<UploadReport>, Option<String>)> {
    if config.blossom_servers.is_empty() {
        return Ok((None, None));
    }

    let root_cid =
        Cid::parse(root_cid_str).with_context(|| format!("parsing root cid {root_cid_str}"))?;
    let previous_root_cid = previous_root_cid
        .map(Cid::parse)
        .transpose()
        .context("parsing previous root cid")?;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(BLOSSOM_UPLOAD_TIMEOUT_SECS),
        upload_tree_to_blossom_with_hashtree(
            config_dir,
            config,
            device,
            root_cid,
            previous_root_cid,
        ),
    )
    .await;
    Ok(match result {
        Ok(Ok(upload)) => (Some(upload), None),
        Ok(Err(error)) => (None, Some(format!("{error:#}"))),
        Err(_) => (
            None,
            Some(format!("timed out after {BLOSSOM_UPLOAD_TIMEOUT_SECS}s")),
        ),
    })
}

pub(crate) async fn start_fips_block_sync(
    config_dir: &std::path::Path,
    config: &AppConfig,
) -> Result<FsFipsBlockSync> {
    let device = iris_drive_core::DeviceIdentity::load(key_path_in(config_dir))
        .context("loading device key")?;
    let daemon = Daemon::open(config_dir).context("opening daemon for direct FIPS sync")?;
    let local = daemon.tree().get_store().clone();
    iris_drive_core::FipsBlockSync::start(&device, local, config)
        .await
        .context("starting direct FIPS block sync")
}

pub(crate) async fn download_tree_over_fips_with_retry(
    fips: &FsFipsBlockSync,
    root: &Cid,
    policy: FipsDownloadPolicy,
) -> Result<DownloadReport> {
    let mut last_error: Option<anyhow::Error> = None;
    for delay in std::iter::once(0).chain(policy.retry_delays.iter().copied()) {
        if delay > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        }
        match tokio::time::timeout(policy.attempt_timeout, fips.download_tree(root)).await {
            Ok(Ok(report)) => return Ok(report),
            Ok(Err(error)) => last_error = Some(anyhow::Error::from(error)),
            Err(_) => {
                last_error = Some(anyhow::anyhow!(
                    "FIPS download timed out after {}s",
                    policy.attempt_timeout.as_secs()
                ));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("FIPS download failed")))
}

#[derive(Clone, Copy)]
pub(crate) struct FipsDownloadPolicy {
    retry_delays: &'static [u64],
    attempt_timeout: std::time::Duration,
}

pub(crate) fn fips_download_policy(config: &AppConfig) -> FipsDownloadPolicy {
    if config.blossom_servers.is_empty() {
        FipsDownloadPolicy {
            retry_delays: FIPS_DOWNLOAD_RETRY_DELAYS,
            attempt_timeout: std::time::Duration::from_secs(FIPS_DOWNLOAD_ATTEMPT_TIMEOUT_SECS),
        }
    } else {
        FipsDownloadPolicy {
            retry_delays: FIPS_DOWNLOAD_BEFORE_BLOSSOM_RETRY_DELAYS,
            attempt_timeout: std::time::Duration::from_secs(
                FIPS_DOWNLOAD_BEFORE_BLOSSOM_ATTEMPT_TIMEOUT_SECS,
            ),
        }
    }
}

pub(crate) async fn download_roots_over_blossom(
    config_dir: &std::path::Path,
    config: &AppConfig,
    root_cid_strs: &[String],
) -> Result<DownloadReport> {
    if config.blossom_servers.is_empty() {
        return Err(anyhow::anyhow!("no blossom servers configured"));
    }

    let device = iris_drive_core::DeviceIdentity::load(key_path_in(config_dir))
        .context("loading device key")?;
    let daemon = Daemon::open(config_dir).context("opening daemon for Blossom sync")?;
    let local = daemon.tree().get_store().clone();
    let bclient =
        iris_drive_core::blossom_sync_client(device.keys().clone(), &config.blossom_servers);
    let mut totals = DownloadReport::default();
    for cid_str in root_cid_strs {
        let cid = Cid::parse(cid_str).with_context(|| format!("parsing root cid {cid_str}"))?;
        let report = iris_drive_core::blossom_sync::download_tree_with_retry(
            local.clone(),
            &cid,
            bclient.clone(),
            BLOSSOM_DOWNLOAD_RETRY_DELAYS,
        )
        .await
        .with_context(|| format!("downloading tree from Blossom for {cid_str}"))?;
        add_download_report(&mut totals, report);
    }
    Ok(totals)
}

pub(crate) fn add_download_report(total: &mut DownloadReport, report: DownloadReport) {
    total.total_hashes += report.total_hashes;
    total.fetched += report.fetched;
    total.already_local += report.already_local;
}

pub(crate) fn download_report_json(report: &DownloadReport) -> serde_json::Value {
    json!({
        "total_hashes": report.total_hashes,
        "fetched": report.fetched,
        "already_local": report.already_local,
    })
}

pub(crate) fn publish_state_report_json(report: &PublishStateReport) -> serde_json::Value {
    json!({
        "published_profile_roster_ops": report.published_profile_roster_ops,
        "profile_roster_publish_error": report.profile_roster_publish_error,
        "published_drive_root": report.published_drive_root,
        "drive_root_publish_error": report.drive_root_publish_error,
        "published_files_root": report.published_files_root,
        "files_root_publish_error": report.files_root_publish_error,
        "root_cid": report.root_cid,
        "blossom_upload_error": report.blossom_upload_error,
        "blossom_upload": report.blossom_upload.as_ref().map(|r| json!({
            "total_hashes": r.total_hashes,
            "uploaded": r.uploaded,
            "already_present": r.already_present,
        })),
    })
}

pub(crate) async fn import_mount_root_and_publish(
    client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    visible_root: Cid,
    tombstone_base_root: Option<Cid>,
    direct_roots: &mut DirectRootExchange,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<()> {
    import_mount_root_and_publish_with_tombstone_paths(
        client,
        config_dir,
        visible_root,
        tombstone_base_root,
        None,
        direct_roots,
        fips_blocks,
    )
    .await
}

pub(crate) async fn import_mount_root_and_publish_with_tombstone_paths(
    client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    visible_root: Cid,
    tombstone_base_root: Option<Cid>,
    tombstone_paths: Option<&BTreeSet<String>>,
    direct_roots: &mut DirectRootExchange,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<()> {
    if !mount_visible_root_has_changed(&visible_root, tombstone_base_root.as_ref()) {
        let payload = json!({
            "event": "mounted_root_unchanged",
            "root_cid": visible_root.to_string(),
            "publish": {"queued": false},
        });
        write_daemon_status(config_dir, payload.clone());
        println!("{payload}");
        return Ok(());
    }

    let mut daemon = Daemon::open(config_dir)
        .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
    let import = daemon
        .import_visible_root_with_tombstone_base_and_paths(
            visible_root,
            tombstone_base_root,
            tombstone_paths,
        )
        .await
        .context("importing mounted root")?;
    let updated_config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(updated_state) = updated_config.account.clone() else {
        return Err(anyhow::anyhow!("missing account after mount import"));
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
        "mounted_root_publish_finished",
        json!({"root_cid": import.root_cid.clone()}),
    );
    println!(
        "{}",
        json!({
            "event": "mounted_root",
            "import": {
                "root_cid": import.root_cid,
                "file_count": import.file_count,
                "top_level_entries": import.top_level_entries,
            },
            "direct_root_mesh_error": direct_root_mesh_error,
            "publish": {"queued": true, "upload_blossom": true},
        })
    );
    Ok(())
}

fn mount_visible_root_has_changed(visible_root: &Cid, tombstone_base_root: Option<&Cid>) -> bool {
    !tombstone_base_root.is_some_and(|base| base == visible_root)
}

pub(crate) fn spawn_publish_current_state(
    client: nostr_sdk::Client,
    config_dir: PathBuf,
    config: AppConfig,
    state: AccountState,
    upload_blossom: bool,
    event_name: &'static str,
    context: Value,
) {
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        let payload = match publish_current_state(
            &client,
            &config_dir,
            &config,
            &state,
            upload_blossom,
        )
        .await
        {
            Ok(report) => json!({
                "event": event_name,
                "elapsed_ms": started.elapsed().as_millis(),
                "context": context,
                "publish": publish_state_report_json(&report),
            }),
            Err(error) => json!({
                "event": format!("{event_name}_error"),
                "elapsed_ms": started.elapsed().as_millis(),
                "context": context,
                "error": format!("{error:#}"),
            }),
        };
        write_daemon_status(&config_dir, payload.clone());
        println!("{payload}");
    });
}

pub(crate) async fn publish_current_state(
    client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    config: &AppConfig,
    state: &AccountState,
    upload_blossom: bool,
) -> Result<PublishStateReport> {
    use iris_drive_core::relay_sync;

    let mut report = PublishStateReport::default();
    if !state.profile_roster_ops.is_empty() {
        match relay_publish_with_timeout(relay_sync::publish_iris_profile_roster_ops(
            client,
            &state.profile_roster_ops,
        ))
        .await
        {
            Ok(event_ids) => report.published_profile_roster_ops = event_ids.len(),
            Err(error) => report.profile_roster_publish_error = Some(error),
        }
    }

    if let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)
        && let Some(root) = publishable_device_root(config_dir, drive, state).await?
    {
        ensure_publishable_root_locally_available(config_dir, &root.root_cid).await?;
        let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
        report.root_cid = Some(root.root_cid.clone());

        if upload_blossom {
            let (blossom_upload, blossom_upload_error) =
                maybe_upload_root_to_blossom(config_dir, config, &device, &root.root_cid, None)
                    .await?;
            report.blossom_upload = blossom_upload;
            report.blossom_upload_error = blossom_upload_error;
        }

        match relay_publish_with_timeout(relay_sync::publish_drive_root(
            client,
            device.keys(),
            &state.root_scope_id(),
            &drive.drive_id,
            &root,
            &authorized_device_pubkeys(state),
        ))
        .await
        {
            Ok(_) => report.published_drive_root = true,
            Err(error) => report.drive_root_publish_error = Some(error),
        }

        if state.can_write_roots() {
            let account = Account::load(state.clone(), config_dir).context("loading account")?;
            match relay_publish_with_timeout(relay_sync::publish_files_root(
                client,
                account.device.keys(),
                &drive.drive_id,
                &root,
            ))
            .await
            {
                Ok(_) => report.published_files_root = true,
                Err(error) => {
                    report.files_root_publish_error = Some(error);
                }
            }
        }
    }

    Ok(report)
}

pub(crate) fn spawn_initial_publish(
    client: nostr_sdk::Client,
    config_dir: PathBuf,
    startup_config: AppConfig,
    startup_state: AccountState,
) {
    tokio::spawn(async move {
        match tokio::time::timeout(
            std::time::Duration::from_secs(STARTUP_NETWORK_TIMEOUT_SECS),
            publish_current_state(&client, &config_dir, &startup_config, &startup_state, true),
        )
        .await
        {
            Ok(Ok(report)) => {
                let drive_iris_to_url = report
                    .root_cid
                    .as_ref()
                    .and_then(|_| drive_iris_to_url_for_primary_drive(&startup_config));
                let snapshot_url = report
                    .root_cid
                    .as_deref()
                    .and_then(drive_iris_to_snapshot_url_for_root);
                println!(
                    "{}",
                    json!({
                        "event": "initial_publish",
                        "published_profile_roster_ops": report.published_profile_roster_ops,
                        "profile_roster_publish_error": report.profile_roster_publish_error,
                        "published_drive_root": report.published_drive_root,
                        "drive_root_publish_error": report.drive_root_publish_error,
                        "published_files_root": report.published_files_root,
                        "files_root_publish_error": report.files_root_publish_error,
                        "root_cid": report.root_cid,
                        "drive_iris_to_url": drive_iris_to_url,
                        "files_iris_to_url": drive_iris_to_url,
                        "snapshot_url": snapshot_url,
                        "permalink_url": snapshot_url,
                        "blossom_upload_error": report.blossom_upload_error,
                        "blossom_upload": report.blossom_upload.map(|r| json!({
                            "total_hashes": r.total_hashes,
                            "uploaded": r.uploaded,
                            "already_present": r.already_present,
                        })),
                    })
                );
            }
            Ok(Err(error)) => {
                println!(
                    "{}",
                    json!({"event": "initial_publish_error", "error": error.to_string()})
                );
            }
            Err(_) => {
                println!(
                    "{}",
                    json!({
                        "event": "initial_publish_error",
                        "error": format!("timed out after {STARTUP_NETWORK_TIMEOUT_SECS}s"),
                    })
                );
            }
        }
    });
}

pub(crate) fn spawn_daemon_heartbeat(config_dir: PathBuf) {
    let _ = std::thread::Builder::new()
        .name("idrive-status-heartbeat".to_string())
        .spawn(move || {
            loop {
                write_daemon_status(&config_dir, json!({"event": "heartbeat"}));
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        });
}

pub(crate) async fn relay_publish_with_timeout<T, F>(future: F) -> std::result::Result<T, String>
where
    F: std::future::Future<
            Output = std::result::Result<T, iris_drive_core::relay_sync::RelayError>,
        >,
{
    relay_publish_with_timeout_duration(
        std::time::Duration::from_secs(RELAY_PUBLISH_TIMEOUT_SECS),
        future,
    )
    .await
}

pub(crate) async fn relay_publish_with_timeout_duration<T, F>(
    timeout: std::time::Duration,
    future: F,
) -> std::result::Result<T, String>
where
    F: std::future::Future<
            Output = std::result::Result<T, iris_drive_core::relay_sync::RelayError>,
        >,
{
    match tokio::time::timeout(timeout, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err(format!("timed out after {}s", timeout.as_secs())),
    }
}

#[cfg(test)]
mod tests;
