//! One-shot network sync for native shells.
//!
//! CLI, mobile, and desktop shells all need the same sequence after a device
//! is authorized: fetch signed root metadata, apply it to local config, and
//! pull the referenced blocks into the local hashtree store.

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use hashtree_core::Cid;
use nostr_sdk::Event;

use crate::account::{Account, AccountState};
use crate::blossom_sync::DownloadReport;
use crate::config::AppConfig;
use crate::daemon::Daemon;
use crate::fips_sync::FsFipsBlockSync;
use crate::identity::DeviceIdentity;
use crate::paths::{config_path_in, key_path_in};
use crate::{PRIMARY_DRIVE_ID, blossom_sync, blossom_sync_client, relay_sync};

const FIPS_DOWNLOAD_RETRY_DELAYS: &[u64] = &[1, 2, 4];
const FIPS_DOWNLOAD_BEFORE_BLOSSOM_RETRY_DELAYS: &[u64] = &[];
const FIPS_DOWNLOAD_ATTEMPT_TIMEOUT_SECS: u64 = 8;
const FIPS_DOWNLOAD_BEFORE_BLOSSOM_ATTEMPT_TIMEOUT_SECS: u64 = 3;
const BLOSSOM_DOWNLOAD_RETRY_DELAYS: &[u64] = &[1, 2, 4];

#[derive(Debug, Default, Clone)]
pub struct NetworkSyncReport {
    pub relays: Vec<String>,
    pub blossom_servers: Vec<String>,
    pub app_keys_event_applied: String,
    pub profile_roster_ops_seen: usize,
    pub profile_roster_ops_applied: usize,
    pub drive_root_events_seen: usize,
    pub drive_root_events_applied: usize,
    pub drive_root_events_skipped: usize,
    pub files_root_event_seen: bool,
    pub files_root_event_outcome: String,
    pub files_root_fetch_error: Option<String>,
    pub fips_download: Option<DownloadReport>,
    pub fips_download_error: Option<String>,
    pub blossom_download: Option<DownloadReport>,
    pub blossom_download_error: Option<String>,
    pub materialized_root_cid: Option<String>,
}

#[derive(Debug, Default, Clone)]
pub struct DriveRootEventApplyReport {
    pub seen: usize,
    pub applied: usize,
    pub skipped: usize,
    pub root_cids_to_download: Vec<String>,
}

pub async fn sync_once(
    config_dir: &Path,
    relay_override: &[String],
    timeout: Duration,
) -> Result<NetworkSyncReport> {
    sync_once_with_fips(config_dir, relay_override, timeout, None).await
}

#[allow(clippy::too_many_lines)]
pub async fn sync_once_with_fips(
    config_dir: &Path,
    relay_override: &[String],
    timeout: Duration,
    fips: Option<&FsFipsBlockSync>,
) -> Result<NetworkSyncReport> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; create or link a profile first"))?;
    let relays = pick_relays(&config, relay_override);
    let client = relay_sync::connect(&relays)
        .await
        .context("connecting to relays")?;
    let mut report = NetworkSyncReport {
        relays,
        blossom_servers: config.blossom_servers.clone(),
        app_keys_event_applied: "none".to_string(),
        files_root_event_outcome: "none".to_string(),
        ..NetworkSyncReport::default()
    };

    let profile_events =
        relay_sync::fetch_iris_profile_roster_ops(&client, state.profile_id, timeout)
            .await
            .context("fetching IrisProfile roster ops")?;
    report.profile_roster_ops_seen = profile_events.len();
    for event in &profile_events {
        if matches!(
            relay_sync::apply_remote_iris_profile_roster_op_event(&mut config, event)
                .context("applying IrisProfile roster op")?,
            relay_sync::IrisProfileRosterOpApply::Applied
        ) {
            report.profile_roster_ops_applied += 1;
        }
    }

    let authorized_devices = config
        .account
        .as_ref()
        .map(authorized_device_pubkeys)
        .unwrap_or_default();
    let root_scope_id = state.root_scope_id();
    let drive_root_events = relay_sync::fetch_drive_roots(
        &client,
        &root_scope_id,
        PRIMARY_DRIVE_ID,
        &authorized_devices,
        timeout,
    )
    .await
    .context("fetching drive roots")?;
    let drive_roots = apply_drive_root_events(config_dir, &mut config, &drive_root_events)
        .context("applying drive-root events")?;
    report.drive_root_events_seen = drive_roots.seen;
    report.drive_root_events_applied = drive_roots.applied;
    report.drive_root_events_skipped = drive_roots.skipped;
    let mut root_cids_to_download = drive_roots.root_cids_to_download;

    match relay_sync::fetch_latest_files_root(
        &client,
        &state.owner_pubkey,
        PRIMARY_DRIVE_ID,
        timeout,
    )
    .await
    {
        Ok(Some(ev)) => {
            report.files_root_event_seen = true;
            if state.can_manage_devices() && state.device_pubkey == state.owner_pubkey {
                let account_state = config
                    .account
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("not initialized; create a profile first"))?;
                let account =
                    Account::load(account_state, config_dir).context("loading account")?;
                let outcome = relay_sync::apply_remote_files_root_event(
                    &mut config,
                    &ev,
                    Some(account.device.keys()),
                )
                .context("applying files-root event")?;
                report.files_root_event_outcome = files_root_apply_label(&outcome).to_string();
                if matches!(outcome, relay_sync::FilesRootApply::Applied)
                    && let Some(root_ref) = config
                        .drive(PRIMARY_DRIVE_ID)
                        .and_then(|drive| drive.device_roots.get(&account.state.device_pubkey))
                {
                    push_unique(&mut root_cids_to_download, root_ref.root_cid.clone());
                }
            } else {
                report.files_root_event_outcome = "account_key_unavailable".to_string();
            }
        }
        Ok(None) => {}
        Err(error) => {
            report.files_root_event_outcome = "fetch_error".to_string();
            report.files_root_fetch_error = Some(format!("{error:#}"));
        }
    }

    config.save(config_path_in(config_dir))?;
    download_roots(
        config_dir,
        &config,
        &root_cids_to_download,
        fips,
        &mut report,
    )
    .await;
    if root_cids_to_download
        .iter()
        .any(|root_cid| root_cid_belongs_to_peer(&config, root_cid))
        && (report.fips_download.is_some() || report.blossom_download.is_some())
    {
        let mut daemon =
            Daemon::open(config_dir).context("opening daemon to materialize merged root")?;
        if let Some(import) = daemon
            .materialize_primary_merged_root()
            .await
            .context("materializing merged root")?
        {
            report.materialized_root_cid = Some(import.root_cid);
        }
    }

    let _ = client.disconnect().await;
    Ok(report)
}

pub fn apply_drive_root_events(
    config_dir: &Path,
    config: &mut AppConfig,
    events: &[Event],
) -> Result<DriveRootEventApplyReport> {
    let device = DeviceIdentity::load(key_path_in(config_dir)).context("loading device key")?;
    let mut report = DriveRootEventApplyReport {
        seen: events.len(),
        ..DriveRootEventApplyReport::default()
    };
    for event in events {
        let parsed =
            crate::nostr_events::parse_drive_root_event_for_device(event, device.keys()).ok();
        if let Some((_, _, _, root_ref)) = parsed.as_ref() {
            push_unique(&mut report.root_cids_to_download, root_ref.root_cid.clone());
        }
        match relay_sync::apply_remote_drive_root_event(config, event, Some(device.keys()))
            .context("applying drive-root event")?
        {
            relay_sync::DriveRootApply::Applied => report.applied += 1,
            _ => report.skipped += 1,
        }
    }
    Ok(report)
}

#[must_use]
pub fn authorized_device_pubkeys(state: &AccountState) -> Vec<String> {
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

fn pick_relays(config: &AppConfig, relay_override: &[String]) -> Vec<String> {
    if relay_override.is_empty() {
        config.relays.clone()
    } else {
        relay_override.to_vec()
    }
}

fn files_root_apply_label(outcome: &relay_sync::FilesRootApply) -> &'static str {
    match outcome {
        relay_sync::FilesRootApply::Applied => "applied",
        relay_sync::FilesRootApply::NotOurOwner => "not_our_owner",
        relay_sync::FilesRootApply::UnknownDrive => "unknown_drive",
        relay_sync::FilesRootApply::StaleTimestamp => "stale",
    }
}

fn root_cid_belongs_to_peer(config: &AppConfig, root_cid: &str) -> bool {
    let Some(account) = config.account.as_ref() else {
        return false;
    };
    config.drive(PRIMARY_DRIVE_ID).is_some_and(|drive| {
        drive
            .device_roots
            .iter()
            .any(|(device, root)| device != &account.device_pubkey && root.root_cid == root_cid)
    })
}

async fn download_roots(
    config_dir: &Path,
    config: &AppConfig,
    root_cid_strs: &[String],
    fips: Option<&FsFipsBlockSync>,
    report: &mut NetworkSyncReport,
) {
    if root_cid_strs.is_empty() {
        return;
    }

    let fips_policy = fips_download_policy(config);
    if let Some(fips) = fips {
        match download_roots_over_fips(fips, root_cid_strs, fips_policy).await {
            Ok(download) => report.fips_download = Some(download),
            Err(error) => report.fips_download_error = Some(format!("{error:#}")),
        }
    } else {
        let mut block_config = config.clone();
        block_config.relays = report.relays.clone();
        match start_fips_block_sync(config_dir, &block_config).await {
            Ok(fips) => match download_roots_over_fips(&fips, root_cid_strs, fips_policy).await {
                Ok(download) => report.fips_download = Some(download),
                Err(error) => report.fips_download_error = Some(format!("{error:#}")),
            },
            Err(error) => report.fips_download_error = Some(format!("{error:#}")),
        }
    }

    if report.fips_download.is_none() && !config.blossom_servers.is_empty() {
        match download_roots_over_blossom(config_dir, config, root_cid_strs).await {
            Ok(download) => report.blossom_download = Some(download),
            Err(error) => report.blossom_download_error = Some(error.to_string()),
        }
    }
}

async fn start_fips_block_sync(config_dir: &Path, config: &AppConfig) -> Result<FsFipsBlockSync> {
    let device = DeviceIdentity::load(key_path_in(config_dir)).context("loading device key")?;
    let daemon = Daemon::open(config_dir).context("opening daemon for direct FIPS sync")?;
    let local = daemon.tree().get_store().clone();
    crate::FipsBlockSync::start(&device, local, config)
        .await
        .context("starting direct FIPS block sync")
}

async fn download_roots_over_fips(
    fips: &FsFipsBlockSync,
    root_cid_strs: &[String],
    policy: FipsDownloadPolicy,
) -> Result<DownloadReport> {
    let mut totals = DownloadReport::default();
    for cid_str in root_cid_strs {
        let cid = Cid::parse(cid_str).with_context(|| format!("parsing root cid {cid_str}"))?;
        let report = download_tree_over_fips_with_retry(fips, &cid, policy)
            .await
            .with_context(|| format!("downloading tree over FIPS for {cid_str}"))?;
        add_download_report(&mut totals, report);
    }
    Ok(totals)
}

async fn download_tree_over_fips_with_retry(
    fips: &FsFipsBlockSync,
    root: &Cid,
    policy: FipsDownloadPolicy,
) -> Result<DownloadReport> {
    let mut last_error: Option<anyhow::Error> = None;
    for delay in std::iter::once(0).chain(policy.retry_delays.iter().copied()) {
        if delay > 0 {
            tokio::time::sleep(Duration::from_secs(delay)).await;
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
struct FipsDownloadPolicy {
    retry_delays: &'static [u64],
    attempt_timeout: Duration,
}

fn fips_download_policy(config: &AppConfig) -> FipsDownloadPolicy {
    if config.blossom_servers.is_empty() {
        FipsDownloadPolicy {
            retry_delays: FIPS_DOWNLOAD_RETRY_DELAYS,
            attempt_timeout: Duration::from_secs(FIPS_DOWNLOAD_ATTEMPT_TIMEOUT_SECS),
        }
    } else {
        FipsDownloadPolicy {
            retry_delays: FIPS_DOWNLOAD_BEFORE_BLOSSOM_RETRY_DELAYS,
            attempt_timeout: Duration::from_secs(FIPS_DOWNLOAD_BEFORE_BLOSSOM_ATTEMPT_TIMEOUT_SECS),
        }
    }
}

async fn download_roots_over_blossom(
    config_dir: &Path,
    config: &AppConfig,
    root_cid_strs: &[String],
) -> Result<DownloadReport> {
    if config.blossom_servers.is_empty() {
        return Err(anyhow::anyhow!("no blossom servers configured"));
    }

    let device = DeviceIdentity::load(key_path_in(config_dir)).context("loading device key")?;
    let daemon = Daemon::open(config_dir).context("opening daemon for Blossom sync")?;
    let local = daemon.tree().get_store().clone();
    let bclient = blossom_sync_client(device.keys().clone(), &config.blossom_servers);
    let mut totals = DownloadReport::default();
    for cid_str in root_cid_strs {
        let cid = Cid::parse(cid_str).with_context(|| format!("parsing root cid {cid_str}"))?;
        let report = blossom_sync::download_tree_with_retry(
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

fn add_download_report(total: &mut DownloadReport, report: DownloadReport) {
    total.total_hashes += report.total_hashes;
    total.fetched += report.fetched;
    total.already_local += report.already_local;
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.iter().any(|existing| existing == &value) {
        values.push(value);
    }
}
