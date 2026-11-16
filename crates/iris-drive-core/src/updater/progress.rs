//! What an update install is doing right now, in the words every app shows.
//!
//! The core reports finding, downloading and checking a release. The native
//! shells report the steps only they can take (installing, restarting, opening
//! an installer) through [`report_update_stage`], then render
//! [`update_progress`] as given: the text, a progress fraction and, after a
//! failure, a way to try again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use hashtree_updater::DownloadEvent;
use serde::{Deserialize, Serialize};

const MAX_REASON_CHARS: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStage {
    #[default]
    Idle,
    FindingRelease,
    Downloading,
    Verifying,
    Installing,
    Restarting,
    OpeningInstaller,
    InstallerOpened,
    Failed,
}

impl UpdateStage {
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(value.trim().to_owned())).ok()
    }

    const fn active(self) -> bool {
        !matches!(self, Self::Idle | Self::InstallerOpened | Self::Failed)
    }
}

/// One snapshot of an update install. `text`, `fraction`, `active`, `shown`
/// and `can_retry` are derived; shells render them and never word steps
/// themselves. `shown` replaces the update offer with this step, `fraction`
/// is a download's progress bar, and other active steps show a spinner.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateProgress {
    pub stage: UpdateStage,
    pub tag: String,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub error: Option<String>,
    pub text: String,
    pub fraction: Option<f64>,
    pub active: bool,
    pub shown: bool,
    pub can_retry: bool,
}

impl Default for UpdateProgress {
    fn default() -> Self {
        Self::new(UpdateStage::Idle, "")
    }
}

impl UpdateProgress {
    #[must_use]
    pub fn new(stage: UpdateStage, tag: &str) -> Self {
        Self::build(stage, tag, 0, None, None)
    }

    #[must_use]
    pub fn downloading(tag: &str, downloaded_bytes: u64, total_bytes: Option<u64>) -> Self {
        Self::build(
            UpdateStage::Downloading,
            tag,
            downloaded_bytes,
            total_bytes,
            None,
        )
    }

    #[must_use]
    pub fn failed(tag: &str, reason: &str) -> Self {
        Self::build(UpdateStage::Failed, tag, 0, None, Some(reason_text(reason)))
    }

    fn build(
        stage: UpdateStage,
        tag: &str,
        downloaded_bytes: u64,
        total_bytes: Option<u64>,
        error: Option<String>,
    ) -> Self {
        let tag = tag.trim().to_owned();
        let total_bytes = total_bytes.filter(|total| *total > 0);
        let fraction = (stage == UpdateStage::Downloading)
            .then_some(total_bytes)
            .flatten()
            .map(|total| {
                #[allow(clippy::cast_precision_loss)]
                let fraction = downloaded_bytes as f64 / total as f64;
                fraction.clamp(0.0, 1.0)
            });
        let text = describe(stage, &tag, downloaded_bytes, total_bytes, error.as_deref());
        Self {
            stage,
            tag,
            downloaded_bytes,
            total_bytes,
            error,
            text,
            fraction,
            active: stage.active(),
            shown: stage != UpdateStage::Idle,
            can_retry: stage == UpdateStage::Failed,
        }
    }

    /// The download progress for one updater event. Chunk events carry no
    /// size, so `known_total` is the length the start event announced.
    #[must_use]
    pub fn from_download_event(tag: &str, event: &DownloadEvent, known_total: Option<u64>) -> Self {
        match event {
            DownloadEvent::Started { content_length } => Self::downloading(tag, 0, *content_length),
            DownloadEvent::Progress { downloaded, .. } => {
                Self::downloading(tag, *downloaded, known_total)
            }
            DownloadEvent::Finished { total } => Self::downloading(tag, *total, Some(*total)),
        }
    }
}

fn describe(
    stage: UpdateStage,
    tag: &str,
    downloaded: u64,
    total: Option<u64>,
    error: Option<&str>,
) -> String {
    let release = if tag.is_empty() { "the update" } else { tag };
    match stage {
        UpdateStage::Idle => String::new(),
        UpdateStage::FindingRelease if tag.is_empty() => "Finding the latest release".to_owned(),
        UpdateStage::FindingRelease => format!("Finding release {tag}"),
        UpdateStage::Downloading => match total {
            Some(total) => format!(
                "Downloading {release}: {} of {}",
                megabytes(downloaded),
                megabytes(total)
            ),
            None => format!("Downloading {release}: {}", megabytes(downloaded)),
        },
        UpdateStage::Verifying => format!("Checking the {release} download"),
        UpdateStage::Installing => format!("Installing {release}"),
        UpdateStage::Restarting => "Restarting Iris Drive".to_owned(),
        UpdateStage::OpeningInstaller => format!("Opening the {release} installer"),
        UpdateStage::InstallerOpened => {
            format!("The {release} installer is open. Finish installing there.")
        }
        UpdateStage::Failed => format!(
            "Couldn't install {release}: {}",
            error.unwrap_or("something went wrong")
        ),
    }
}

fn megabytes(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss)]
    let value = bytes as f64 / 1_000_000.0;
    if value >= 10.0 {
        format!("{value:.0} MB")
    } else {
        format!("{value:.1} MB")
    }
}

/// The first line of an error, trimmed and bounded, without a trailing period.
fn reason_text(reason: &str) -> String {
    let line = reason
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("something went wrong");
    let mut text: String = line.chars().take(MAX_REASON_CHARS).collect();
    if line.chars().count() > MAX_REASON_CHARS {
        text.push('…');
    }
    text.trim_end_matches('.').to_owned()
}

static PROGRESS: LazyLock<Mutex<HashMap<PathBuf, UpdateProgress>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// The current update install for one profile; idle before the first press.
#[must_use]
pub fn update_progress(config_dir: &Path) -> UpdateProgress {
    PROGRESS
        .lock()
        .ok()
        .and_then(|progress| progress.get(config_dir).cloned())
        .unwrap_or_default()
}

pub fn report_update_progress(config_dir: &Path, progress: UpdateProgress) {
    if let Ok(mut map) = PROGRESS.lock() {
        map.insert(config_dir.to_owned(), progress);
    }
}

/// A step a shell took. Keeps the release tag already known; `reason` only
/// matters for [`UpdateStage::Failed`]. Shells that render later may ignore
/// the returned snapshot.
#[allow(clippy::must_use_candidate)]
pub fn report_update_stage(config_dir: &Path, stage: UpdateStage, reason: &str) -> UpdateProgress {
    let tag = update_progress(config_dir).tag;
    let progress = if stage == UpdateStage::Failed {
        UpdateProgress::failed(&tag, reason)
    } else {
        UpdateProgress::new(stage, &tag)
    };
    report_update_progress(config_dir, progress.clone());
    progress
}

#[cfg(test)]
mod tests;
