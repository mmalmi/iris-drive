#[allow(clippy::wildcard_imports)]
use super::*;

#[allow(clippy::too_many_lines)]
pub(crate) fn cmd_sync(
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
        let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
        let state = config
            .account
            .clone()
            .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
        let relays = pick_relays(&config, relay_override);
        let client = relay_sync::connect(&relays)
            .await
            .context("connecting to relays")?;
        let timeout = std::time::Duration::from_secs(timeout_secs);

        // 1) AppKeys rosters are not relayed; they arrive over direct/FIPS
        // link messages or the direct root-event mesh.
        let app_keys_applied = "none";

        // 2) Pull drive roots for every authorized device.
        let authorized_devices: Vec<String> = config
            .account
            .as_ref()
            .and_then(|s| s.app_keys.as_ref())
            .map(|s| s.devices.iter().map(|d| d.pubkey.clone()).collect())
            .unwrap_or_default();
        let drive_root_events = relay_sync::fetch_drive_roots(
            &client,
            &state.owner_pubkey,
            iris_drive_core::PRIMARY_DRIVE_ID,
            &authorized_devices,
            timeout,
        )
        .await
        .context("fetching drive roots")?;
        let mut drive_roots_applied = 0usize;
        let mut drive_roots_skipped = 0usize;
        let mut root_cids_to_download: Vec<String> = Vec::new();
        let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
        for ev in &drive_root_events {
            let parsed =
                iris_drive_core::nostr_events::parse_drive_root_event_for_device(ev, device.keys())
                    .ok();
            if let Some((_, _, _, root_ref)) = parsed.as_ref()
                && !root_cids_to_download
                    .iter()
                    .any(|root_cid| root_cid == &root_ref.root_cid)
            {
                root_cids_to_download.push(root_ref.root_cid.clone());
            }
            match relay_sync::apply_remote_drive_root_event(&mut config, ev, Some(device.keys()))
                .context("applying drive-root event")?
            {
                relay_sync::DriveRootApply::Applied => {
                    drive_roots_applied += 1;
                }
                _ => drive_roots_skipped += 1,
            }
        }

        // 3) Pull the owner-signed web root for drive.iris.to. This is the
        // standard hashtree mutable-root event used by all web Iris apps, so a
        // native restore of a web account can import browser-origin changes.
        let mut files_root_event_seen = false;
        let mut files_root_event_outcome = "none".to_string();
        let mut files_root_fetch_error: Option<String> = None;
        match relay_sync::fetch_latest_files_root(
            &client,
            &state.owner_pubkey,
            iris_drive_core::PRIMARY_DRIVE_ID,
            timeout,
        )
        .await
        {
            Ok(Some(ev)) => {
                files_root_event_seen = true;
                if state.can_manage_devices() && state.device_pubkey == state.owner_pubkey {
                    let account_state = config.account.clone().ok_or_else(|| {
                        anyhow::anyhow!("not initialized; run `idrive init` first")
                    })?;
                    let account =
                        Account::load(account_state, config_dir).context("loading account")?;
                    let outcome = relay_sync::apply_remote_files_root_event(
                        &mut config,
                        &ev,
                        Some(account.device.keys()),
                    )
                    .context("applying files-root event")?;
                    files_root_event_outcome = files_root_apply_label(&outcome).to_string();
                    if matches!(outcome, relay_sync::FilesRootApply::Applied)
                        && let Some(root_ref) = config
                            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
                            .and_then(|drive| drive.device_roots.get(&account.state.device_pubkey))
                        && !root_cids_to_download
                            .iter()
                            .any(|root_cid| root_cid == &root_ref.root_cid)
                    {
                        root_cids_to_download.push(root_ref.root_cid.clone());
                    }
                } else {
                    files_root_event_outcome = "account_key_unavailable".to_string();
                }
            }
            Ok(None) => {}
            Err(error) => {
                files_root_event_outcome = "fetch_error".to_string();
                files_root_fetch_error = Some(format!("{error:#}"));
            }
        }

        config.save(config_path_in(config_dir))?;

        // 4) Replicate blocks for each seen drive root. Prefer direct
        // FIPS peer transfer between authorized Iris Drive instances;
        // Blossom stays as the public fallback/cache path.
        let mut fips_download_report: Option<DownloadReport> = None;
        let mut fips_download_error: Option<String> = None;
        let mut blossom_download_report: Option<DownloadReport> = None;
        let mut blossom_download_error: Option<String> = None;
        if !root_cids_to_download.is_empty() {
            let fips_policy = fips_download_policy(&config);
            let mut block_config = config.clone();
            block_config.relays = relays.clone();
            match start_fips_block_sync(config_dir, &block_config).await {
                Ok(fips) => {
                    match download_roots_over_fips(&fips, &root_cids_to_download, fips_policy).await
                    {
                        Ok(report) => fips_download_report = Some(report),
                        Err(error) => fips_download_error = Some(format!("{error:#}")),
                    }
                }
                Err(error) => fips_download_error = Some(format!("{error:#}")),
            }

            if fips_download_report.is_none() && !config.blossom_servers.is_empty() {
                match download_roots_over_blossom(config_dir, &config, &root_cids_to_download).await
                {
                    Ok(report) => blossom_download_report = Some(report),
                    Err(error) => blossom_download_error = Some(error.to_string()),
                }
            }
        }

        let _ = client.disconnect().await;

        println!(
            "{}",
            json!({
                "relays": relays,
                "blossom_servers": config.blossom_servers,
                "app_keys_event_applied": app_keys_applied,
                "drive_root_events_seen": drive_root_events.len(),
                "drive_root_events_applied": drive_roots_applied,
                "drive_root_events_skipped": drive_roots_skipped,
                "files_root_event_seen": files_root_event_seen,
                "files_root_event_outcome": files_root_event_outcome,
                "files_root_fetch_error": files_root_fetch_error,
                "fips_download": fips_download_report.as_ref().map(download_report_json),
                "fips_download_error": fips_download_error,
                "blossom_download": blossom_download_report.as_ref().map(download_report_json),
                "blossom_download_error": blossom_download_error,
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}
