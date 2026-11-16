//! When this device measures what it holds and tells the profile's other
//! devices, as Iris Drive web does (`src/lib/deviceStorage.ts`):
//!
//! - it measures seldom: a while after the Drive changed and at most once an
//!   hour, once a day anyway, and soon after starting only when it has not
//!   reported for a day. A measurement counts again only the folders that
//!   changed, and each other folder at most once a day. It sends the numbers
//!   when they moved;
//! - about 15 seconds after it takes in another device's root it sends which
//!   versions it has taken in, with the last numbers and without measuring,
//!   as a device that only reads publishes nothing else that tells;
//! - when the profile's devices change, or a report arrives from a device it
//!   has not told, it sends again.
//!
//! It runs as a task of its own: measuring and relays never hold up anything
//! a person waits on. What it measured and heard is kept on disk at once
//! (`super::state`), so the apps show it without asking relays.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use nostr_sdk::{Client, Event};

use super::event::build_device_storage_event;
use super::measure::HoldingCache;
use super::report::{DeviceStorageReport, report_moved};
use super::state::{DeviceStorageState, SentReport, profile_devices, update_device_storage_state};
use super::unix_now;
use crate::config::AppConfig;
use crate::paths::{config_path_in, key_path_in};
use crate::{AppKey, Daemon, PRIMARY_DRIVE_ID, ProfileState};

/// Starting has enough to do.
const AT_START: Duration = Duration::from_mins(2);
/// Measured this long after the Drive changed…
const AFTER_CHANGE: Duration = Duration::from_mins(10);
/// …and at most this often.
const BACKGROUND_GAP: Duration = Duration::from_hours(1);
/// A folder counted longer ago than this is counted again, for content
/// fetched or removed since.
pub(super) const FOLDER_MAX_AGE_MS: u64 = 24 * 60 * 60 * 1000;
/// The versions taken in are sent this long after taking one in.
const SEEN_AFTER: Duration = Duration::from_secs(15);
/// A report is measured and sent again after this long even when nothing moved.
const RESEND_AFTER_SECS: u64 = 24 * 60 * 60;
/// A Drive not readable yet, as just after linking, is tried again after this long.
const NOT_READABLE_RETRY: Duration = Duration::from_mins(1);
/// A send that failed is tried again after this long.
const RETRY_AFTER: Duration = Duration::from_mins(1);
const TICK: Duration = Duration::from_secs(5);
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// Asks a running reporter to send again: a report arrived from a device it
/// has not told yet.
#[derive(Clone, Default)]
pub struct DeviceStorageNudge(Arc<tokio::sync::Notify>);

impl DeviceStorageNudge {
    pub fn resend(&self) {
        self.0.notify_one();
    }
}

/// The reporter running in this process, for an event handler to nudge.
static RUNNING: std::sync::Mutex<Option<DeviceStorageNudge>> = std::sync::Mutex::new(None);

/// Ask the reporter running in this process, if any, to send again.
pub fn nudge_running_reporter() {
    if let Some(nudge) = RUNNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
    {
        nudge.resend();
    }
}

pub struct DeviceStorageReporter {
    config_dir: PathBuf,
    cache: HoldingCache,
    account: String,
    roots: Option<String>,
    measure_at: Option<Instant>,
    measured_at: Option<Instant>,
    seen_at: Option<Instant>,
    resend: bool,
    retry_at: Option<Instant>,
}

impl DeviceStorageReporter {
    #[must_use]
    pub fn new(config_dir: &Path) -> Self {
        Self {
            config_dir: config_dir.to_path_buf(),
            cache: HoldingCache::default(),
            account: String::new(),
            roots: None,
            measure_at: Some(Instant::now() + AT_START),
            measured_at: None,
            seen_at: None,
            resend: false,
            retry_at: None,
        }
    }

    /// Measure on the next tick rather than after the start delay.
    #[cfg(test)]
    pub(crate) fn measure_now(&mut self) {
        self.measure_at = Some(Instant::now());
    }

    /// Run until `stop` says so, publishing through `client`.
    pub fn spawn(
        config_dir: &Path,
        client: Client,
        stop: Arc<std::sync::atomic::AtomicBool>,
    ) -> (DeviceStorageNudge, tokio::task::JoinHandle<()>) {
        let nudge = DeviceStorageNudge::default();
        let wake = nudge.0.clone();
        *RUNNING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(nudge.clone());
        let mut reporter = Self::new(config_dir);
        let task = tokio::spawn(async move {
            while !stop.load(std::sync::atomic::Ordering::Acquire) {
                tokio::select! {
                    () = tokio::time::sleep(TICK) => {}
                    () = wake.notified() => reporter.resend = true,
                }
                if let Err(error) = reporter.tick(&client).await {
                    tracing::warn!(error = %format!("{error:#}"), "device storage report failed");
                    reporter.retry_at = Some(Instant::now() + RETRY_AFTER);
                }
            }
        });
        (nudge, task)
    }

    /// Measure and send what is due.
    pub async fn tick(&mut self, client: &Client) -> Result<()> {
        if let Some(event) = self.due_event().await? {
            if client.relays().await.is_empty() {
                // Sent once relays are set.
                self.retry_at = Some(Instant::now() + RETRY_AFTER);
                return Ok(());
            }
            tokio::time::timeout(SEND_TIMEOUT, client.send_event(&event))
                .await
                .context("sending device storage report timed out")?
                .map_err(|error| anyhow::anyhow!("sending device storage report: {error}"))?;
        }
        Ok(())
    }

    /// The report to send now, if one is due. It is recorded as sent.
    pub async fn due_event(&mut self) -> Result<Option<Event>> {
        let config = AppConfig::load_or_default(config_path_in(&self.config_dir))?;
        let Some(state) = config
            .profile
            .as_ref()
            .filter(|state| state.is_authorized())
        else {
            return Ok(None);
        };
        let now = Instant::now();
        let account = format!("{}|{}", state.root_scope_id(), state.app_key_pubkey);
        let profile_id = state.root_scope_id();
        let stored = DeviceStorageState::load(&self.config_dir, &profile_id);
        let unix = unix_now();
        let overdue = stored
            .sent
            .as_ref()
            .is_none_or(|sent| unix.saturating_sub(sent.at) > RESEND_AFTER_SECS);
        if account != self.account {
            if !self.account.is_empty() {
                *self = Self::new(&self.config_dir);
            }
            self.account = account;
            // A new device needs a local result on its first tick.
            if stored.own.is_none() {
                self.measure_at = Some(now);
            } else if !overdue && self.measure_at.is_some_and(|at| at > now) {
                self.measure_at = None;
            }
        }
        let roots = roots_key(&config);
        if self.roots.as_ref().is_some_and(|known| *known != roots) {
            let later =
                (now + AFTER_CHANGE).max(self.measured_at.map_or(now, |at| at + BACKGROUND_GAP));
            self.measure_at = Some(self.measure_at.map_or(later, |at| at.min(later)));
            self.seen_at.get_or_insert(now + SEEN_AFTER);
        }
        self.roots = Some(roots);
        if overdue && self.measure_at.is_none() {
            self.measure_at = Some(now);
        }
        let devices = profile_devices(&config, state);

        let mut always = std::mem::take(&mut self.resend);
        if self.retry_at.is_some_and(|at| at <= now) {
            self.retry_at = None;
            always = true;
        }
        let report = if self.measure_at.is_some_and(|at| at <= now) {
            self.measure_at = None;
            let daemon = Daemon::open_with_config(&self.config_dir, config.clone());
            self.measure_at = Some(now + NOT_READABLE_RETRY);
            let daemon = daemon.context("opening local Drive for storage measurement")?;
            let (_, root, _) = daemon
                .provider_projection()
                .await
                .context("reading Drive for storage measurement")?;
            let report = super::local::measure_root(
                &daemon,
                &root.root_cid,
                state,
                &devices,
                &mut self.cache,
            )
            .await?;
            self.measured_at = Some(now);
            self.measure_at = None;
            Some(report)
        } else if self.seen_at.is_some_and(|at| at <= now) {
            self.seen_at = None;
            stored.own.clone().map(|own| DeviceStorageReport {
                seen: Some(seen_versions(&config, state, &devices)),
                ..own
            })
        } else {
            stored.own.clone()
        };
        let Some(report) = report else {
            return Ok(None);
        };
        let keys = AppKey::load(key_path_in(&self.config_dir)).context("loading app key")?;
        let event = update_device_storage_state(&self.config_dir, &profile_id, |stored| {
            stored.own = Some(report.clone());
            let due = always
                || stored.sent.as_ref().is_none_or(|sent| {
                    sent.recipients != devices
                        || unix.saturating_sub(sent.at) > RESEND_AFTER_SECS
                        || report_moved(&sent.report, &report)
                });
            if !due {
                return Ok(None);
            }
            let created_at = stored.next_created_at(unix);
            let event = build_device_storage_event(
                keys.keys(),
                &profile_id,
                &report,
                &devices,
                created_at,
            )?;
            stored.sent = Some(SentReport {
                at: unix,
                created_at,
                recipients: devices.clone(),
                report: report.clone(),
            });
            Ok::<_, crate::sealed_content::SealedContentError>(Some(event))
        })??;
        Ok(event)
    }
}

/// The newest version of each other device's Drive this one has taken in.
/// Its own versions the others learn from its roots.
#[must_use]
pub fn seen_versions(
    config: &AppConfig,
    state: &ProfileState,
    devices: &[String],
) -> BTreeMap<String, u64> {
    let Some(drive) = config.drive(PRIMARY_DRIVE_ID) else {
        return BTreeMap::new();
    };
    devices
        .iter()
        .filter(|device| **device != state.app_key_pubkey)
        .filter_map(|device| {
            drive
                .app_key_roots
                .get(device)
                .filter(|root| root.app_key_seq > 0 && !root.local_only)
                .map(|root| (device.clone(), root.app_key_seq))
        })
        .collect()
}

/// Changes whenever a device's root does.
fn roots_key(config: &AppConfig) -> String {
    config
        .drive(PRIMARY_DRIVE_ID)
        .map(|drive| {
            drive
                .app_key_roots
                .iter()
                .map(|(device, root)| format!("{device}:{}:{}", root.app_key_seq, root.root_cid))
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_default()
}
