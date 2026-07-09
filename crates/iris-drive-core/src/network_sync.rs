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

use crate::blossom_sync::DownloadReport;
use crate::config::{AppConfig, Drive};
use crate::daemon::Daemon;
use crate::fips_sync::FsFipsBlockSync;
use crate::identity::AppKey;
use crate::paths::{config_path_in, key_path_in};
use crate::profile::ProfileState;
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
    pub device_approval_receipts_seen: usize,
    pub device_approval_receipts_applied: usize,
    pub profile_roster_ops_seen: usize,
    pub profile_roster_ops_applied: usize,
    pub share_access_snapshots_seen: usize,
    pub share_access_snapshots_applied: usize,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkSyncOptions {
    pub start_direct_fips_download: bool,
}

impl Default for NetworkSyncOptions {
    fn default() -> Self {
        Self {
            start_direct_fips_download: true,
        }
    }
}

impl NetworkSyncOptions {
    #[must_use]
    pub const fn without_direct_fips_download() -> Self {
        Self {
            start_direct_fips_download: false,
        }
    }
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
    sync_once_with_options(
        config_dir,
        relay_override,
        timeout,
        NetworkSyncOptions::default(),
    )
    .await
}

pub async fn sync_once_with_options(
    config_dir: &Path,
    relay_override: &[String],
    timeout: Duration,
    options: NetworkSyncOptions,
) -> Result<NetworkSyncReport> {
    sync_once_inner(config_dir, relay_override, timeout, None, options).await
}

pub async fn sync_once_with_fips(
    config_dir: &Path,
    relay_override: &[String],
    timeout: Duration,
    fips: Option<&FsFipsBlockSync>,
) -> Result<NetworkSyncReport> {
    sync_once_inner(
        config_dir,
        relay_override,
        timeout,
        fips,
        NetworkSyncOptions::default(),
    )
    .await
}

#[allow(clippy::too_many_lines)]
async fn sync_once_inner(
    config_dir: &Path,
    relay_override: &[String],
    timeout: Duration,
    fips: Option<&FsFipsBlockSync>,
    options: NetworkSyncOptions,
) -> Result<NetworkSyncReport> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let initial_state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; create or link a profile first"))?;
    let relays = pick_relays(&config, relay_override);
    let client = relay_sync::connect(&relays)
        .await
        .context("connecting to relays")?;
    let mut report = NetworkSyncReport {
        relays,
        blossom_servers: config.blossom_servers.clone(),
        files_root_event_outcome: "none".to_string(),
        ..NetworkSyncReport::default()
    };

    let receipt_events =
        relay_sync::fetch_device_approval_receipts(&client, &initial_state, timeout)
            .await
            .context("fetching device approval receipts")?;
    report.device_approval_receipts_seen = receipt_events.len();
    for event in &receipt_events {
        if matches!(
            relay_sync::apply_remote_device_approval_receipt_event(&mut config, event)
                .context("applying device approval receipt")?,
            relay_sync::NostrIdentityRosterOpApply::Applied
        ) {
            report.device_approval_receipts_applied += 1;
        }
    }

    let profile_id = config
        .profile
        .as_ref()
        .map(|state| state.profile_id)
        .ok_or_else(|| anyhow::anyhow!("profile disappeared during sync"))?;
    let profile_events = relay_sync::fetch_nostr_identity_roster_ops(&client, profile_id, timeout)
        .await
        .context("fetching NostrIdentity roster ops")?;
    report.profile_roster_ops_seen = profile_events.len();
    for event in &profile_events {
        if matches!(
            relay_sync::apply_remote_nostr_identity_roster_op_event(&mut config, event)
                .context("applying NostrIdentity roster op")?,
            relay_sync::NostrIdentityRosterOpApply::Applied
        ) {
            report.profile_roster_ops_applied += 1;
        }
    }
    let share_ids = config
        .shared_folders
        .iter()
        .map(|folder| folder.share_id)
        .collect::<Vec<_>>();
    for share_id in share_ids {
        let share_events = relay_sync::fetch_share_access_snapshots(&client, share_id, timeout)
            .await
            .with_context(|| format!("fetching share access snapshots for {share_id}"))?;
        report.share_access_snapshots_seen += share_events.len();
        for event in &share_events {
            if matches!(
                relay_sync::apply_remote_share_access_snapshot_event(&mut config, event)
                    .with_context(|| format!("applying share access snapshot for {share_id}"))?,
                relay_sync::ShareAccessSnapshotApply::Applied
            ) {
                report.share_access_snapshots_applied += 1;
            }
        }
    }

    let state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("profile disappeared during sync"))?;
    let authorized_app_keys = config
        .profile
        .as_ref()
        .map(authorized_app_key_pubkeys)
        .unwrap_or_default();
    let root_scope_id = state.root_scope_id();
    let drive_root_events = relay_sync::fetch_drive_roots(
        &client,
        &root_scope_id,
        PRIMARY_DRIVE_ID,
        &authorized_app_keys,
        timeout,
    )
    .await
    .context("fetching drive roots")?;
    let mut drive_root_events = drive_root_events;
    for folder in &config.shared_folders {
        let share_writers = crate::shared_folder_authorized_writer_pubkeys(folder);
        let mut share_events = relay_sync::fetch_drive_roots(
            &client,
            &folder.share_id.to_string(),
            PRIMARY_DRIVE_ID,
            &share_writers,
            timeout,
        )
        .await
        .with_context(|| format!("fetching share roots for {}", folder.share_id))?;
        drive_root_events.append(&mut share_events);
    }
    let drive_roots = apply_drive_root_events(config_dir, &mut config, &drive_root_events)
        .context("applying drive-root events")?;
    report.drive_root_events_seen = drive_roots.seen;
    report.drive_root_events_applied = drive_roots.applied;
    report.drive_root_events_skipped = drive_roots.skipped;
    let mut root_cids_to_download = drive_roots.root_cids_to_download;

    if let Some(account_state) = config.profile.clone().filter(ProfileState::can_write_roots) {
        match relay_sync::fetch_latest_files_root(
            &client,
            &account_state.app_key_pubkey,
            PRIMARY_DRIVE_ID,
            timeout,
        )
        .await
        {
            Ok(Some(ev)) => {
                report.files_root_event_seen = true;
                let account =
                    crate::Profile::load(account_state, config_dir).context("loading profile")?;
                let outcome = relay_sync::apply_remote_files_root_event(
                    &mut config,
                    &ev,
                    Some(account.app_key.keys()),
                )
                .context("applying files-root event")?;
                report.files_root_event_outcome = files_root_apply_label(&outcome).to_string();
                if matches!(outcome, relay_sync::FilesRootApply::Applied)
                    && let Some(root_ref) = config
                        .drive(PRIMARY_DRIVE_ID)
                        .and_then(|drive| drive.app_key_roots.get(&account.state.app_key_pubkey))
                {
                    push_unique(&mut root_cids_to_download, root_ref.root_cid.clone());
                }
            }
            Ok(None) => {}
            Err(error) => {
                report.files_root_event_outcome = "fetch_error".to_string();
                report.files_root_fetch_error = Some(format!("{error:#}"));
            }
        }
    }

    config.save(config_path_in(config_dir))?;
    download_roots(
        config_dir,
        &config,
        &root_cids_to_download,
        fips,
        options,
        &mut report,
    )
    .await;
    let downloaded_roots = report.fips_download.is_some() || report.blossom_download.is_some();
    let applied_remote_root =
        report.drive_root_events_applied > 0 || report.files_root_event_outcome == "applied";
    if downloaded_roots
        && should_materialize_after_sync(&config, &root_cids_to_download, applied_remote_root)
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

    relay_sync::shutdown_client(&client).await;
    Ok(report)
}

pub fn apply_drive_root_events(
    config_dir: &Path,
    config: &mut AppConfig,
    events: &[Event],
) -> Result<DriveRootEventApplyReport> {
    let device = AppKey::load(key_path_in(config_dir)).context("loading app key")?;
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
pub fn authorized_app_key_pubkeys(state: &ProfileState) -> Vec<String> {
    state.active_root_writer_app_key_pubkeys()
}

#[must_use]
pub fn drive_root_recipient_app_key_pubkeys(state: &ProfileState, drive: &Drive) -> Vec<String> {
    if let Some(app_keys) = current_app_key_actor_pubkeys(state) {
        return app_keys;
    }
    drive_root_writer_app_key_pubkeys(state, drive)
}

fn current_app_key_actor_pubkeys(state: &ProfileState) -> Option<Vec<String>> {
    let mut app_keys = state
        .current_app_keys_projection()?
        .app_actors
        .into_iter()
        .map(|actor| actor.pubkey)
        .collect::<Vec<_>>();
    app_keys.sort();
    app_keys.dedup();
    (!app_keys.is_empty()).then_some(app_keys)
}

#[must_use]
pub fn drive_root_writer_app_key_pubkeys(state: &ProfileState, drive: &Drive) -> Vec<String> {
    let mut app_keys = authorized_app_key_pubkeys(state);
    if !app_keys.is_empty() && (state.has_profile_roster_evidence() || state.app_keys.is_some()) {
        return app_keys;
    }

    app_keys.extend(drive.app_key_roots.keys().cloned());
    if state.can_write_roots() {
        app_keys.push(state.app_key_pubkey.clone());
    }
    app_keys.sort();
    app_keys.dedup();
    app_keys
}

#[must_use]
pub fn drive_root_app_key_can_write_roots(
    state: &ProfileState,
    drive: &Drive,
    app_key_pubkey: &str,
) -> bool {
    if state.can_write_roots_for_app_key(app_key_pubkey) {
        return true;
    }
    if state.has_profile_roster_evidence() || state.app_keys.is_some() {
        return false;
    }
    if app_key_pubkey == state.app_key_pubkey {
        return state.can_write_roots();
    }
    drive.app_key_roots.contains_key(app_key_pubkey)
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
        relay_sync::FilesRootApply::NotOurAppKey => "not_our_app_key",
        relay_sync::FilesRootApply::UnknownDrive => "unknown_drive",
        relay_sync::FilesRootApply::StaleTimestamp => "stale",
    }
}

fn root_cid_belongs_to_peer(config: &AppConfig, root_cid: &str) -> bool {
    let Some(account) = config.profile.as_ref() else {
        return false;
    };
    config.drive(PRIMARY_DRIVE_ID).is_some_and(|drive| {
        drive
            .active_app_key_roots(config.profile.as_ref())
            .into_iter()
            .any(|(device, root)| device != &account.app_key_pubkey && root.root_cid == root_cid)
    })
}

fn should_materialize_after_sync(
    config: &AppConfig,
    root_cid_strs: &[String],
    applied_remote_root: bool,
) -> bool {
    applied_remote_root
        || root_cid_strs
            .iter()
            .any(|root_cid| root_cid_belongs_to_peer(config, root_cid))
}

async fn download_roots(
    config_dir: &Path,
    config: &AppConfig,
    root_cid_strs: &[String],
    fips: Option<&FsFipsBlockSync>,
    options: NetworkSyncOptions,
    report: &mut NetworkSyncReport,
) {
    if root_cid_strs.is_empty() {
        return;
    }

    let fips_policy = fips_download_policy(config);
    match direct_fips_download_decision(fips.is_some(), options) {
        DirectFipsDownloadDecision::UseSupplied => {
            let fips = fips.expect("direct FIPS decision requires supplied handle");
            match download_roots_over_fips(fips, root_cid_strs, fips_policy).await {
                Ok(download) => report.fips_download = Some(download),
                Err(error) => report.fips_download_error = Some(format!("{error:#}")),
            }
        }
        DirectFipsDownloadDecision::StartTemporary => {
            let mut block_config = config.clone();
            block_config.relays = report.relays.clone();
            match start_fips_block_sync(config_dir, &block_config).await {
                Ok(fips) => match download_roots_over_fips(&fips, root_cid_strs, fips_policy).await
                {
                    Ok(download) => report.fips_download = Some(download),
                    Err(error) => report.fips_download_error = Some(format!("{error:#}")),
                },
                Err(error) => report.fips_download_error = Some(format!("{error:#}")),
            }
        }
        DirectFipsDownloadDecision::SkipTemporary => {}
    }

    if report.fips_download.is_none() && !config.blossom_servers.is_empty() {
        match download_roots_over_blossom(config_dir, config, root_cid_strs).await {
            Ok(download) => report.blossom_download = Some(download),
            Err(error) => report.blossom_download_error = Some(format!("{error:#}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DirectFipsDownloadDecision {
    UseSupplied,
    StartTemporary,
    SkipTemporary,
}

fn direct_fips_download_decision(
    has_supplied_fips: bool,
    options: NetworkSyncOptions,
) -> DirectFipsDownloadDecision {
    if has_supplied_fips {
        DirectFipsDownloadDecision::UseSupplied
    } else if options.start_direct_fips_download {
        DirectFipsDownloadDecision::StartTemporary
    } else {
        DirectFipsDownloadDecision::SkipTemporary
    }
}

async fn start_fips_block_sync(config_dir: &Path, config: &AppConfig) -> Result<FsFipsBlockSync> {
    let device = AppKey::load(key_path_in(config_dir)).context("loading app key")?;
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

    let device = AppKey::load(key_path_in(config_dir)).context("loading app key")?;
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

#[cfg(test)]
mod tests {
    use super::{
        DirectFipsDownloadDecision, NetworkSyncOptions, direct_fips_download_decision,
        drive_root_recipient_app_key_pubkeys, should_materialize_after_sync,
    };
    use crate::config::{AppConfig, Drive};
    use crate::profile::Profile;
    use nostr_sdk::Keys;
    use tempfile::tempdir;

    #[test]
    fn default_sync_options_preserve_existing_temporary_fips_download() {
        assert!(NetworkSyncOptions::default().start_direct_fips_download);
        assert_eq!(
            direct_fips_download_decision(false, NetworkSyncOptions::default()),
            DirectFipsDownloadDecision::StartTemporary
        );
    }

    #[test]
    fn sync_options_can_skip_only_temporary_direct_fips_download() {
        let options = NetworkSyncOptions::without_direct_fips_download();

        assert_eq!(
            direct_fips_download_decision(false, options),
            DirectFipsDownloadDecision::SkipTemporary
        );
        assert_eq!(
            direct_fips_download_decision(true, options),
            DirectFipsDownloadDecision::UseSupplied
        );
    }

    #[test]
    fn applied_remote_roots_request_materialization_even_without_peer_root() {
        let config = AppConfig::default();

        assert!(should_materialize_after_sync(&config, &[], true));
        assert!(!should_materialize_after_sync(&config, &[], false));
    }

    #[test]
    fn drive_root_recipients_use_full_cached_app_key_projection() {
        let dir = tempdir().unwrap();
        let mut profile = Profile::create(dir.path(), Some("Owner".into())).unwrap();
        let phone = Keys::generate().public_key().to_hex();
        let tablet = Keys::generate().public_key().to_hex();
        profile
            .approve_app_key(&phone, Some("Phone".into()))
            .unwrap();
        profile
            .approve_app_key(&tablet, Some("Tablet".into()))
            .unwrap();
        let expected = profile
            .state
            .current_app_keys_projection()
            .unwrap()
            .app_actors
            .iter()
            .map(|actor| actor.pubkey.clone())
            .collect::<Vec<_>>();

        let mut partial_state = profile.state.clone();
        partial_state.profile_roster_ops.truncate(1);
        partial_state.profile_roster_projection = None;
        assert_eq!(
            partial_state
                .current_app_keys_projection()
                .unwrap()
                .app_actors
                .len(),
            expected.len()
        );

        let drive = Drive::primary(partial_state.root_scope_id());
        let recipients = drive_root_recipient_app_key_pubkeys(&partial_state, &drive);

        assert_eq!(recipients, expected);
    }
}
