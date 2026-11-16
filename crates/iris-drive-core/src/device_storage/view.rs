//! What the Storage page says about each device, in words, the same as Iris
//! Drive web (`src/lib/deviceStorageView.ts`). The native shells only lay
//! these out.

use std::collections::BTreeMap;

use super::report::DeviceStorageReport;

/// Bytes as the web shows them: `512 B`, `1.5 KB`, `2.3 GB`.
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    #[allow(clippy::cast_precision_loss)]
    let value = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if value < KB * KB {
        format!("{:.1} KB", value / KB)
    } else if value < KB * KB * KB {
        format!("{:.1} MB", value / KB / KB)
    } else {
        format!("{:.1} GB", value / KB / KB / KB)
    }
}

fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// Whether the device holds every file of the Drive it measured.
#[must_use]
pub fn holds_all(report: &DeviceStorageReport) -> bool {
    report.unread_folders.unwrap_or(0) == 0
        && report.held_files >= report.files
        && report.held_bytes >= report.file_bytes
}

/// Locally stored content in the Drive this device measured.
#[must_use]
pub fn holding_label(report: &DeviceStorageReport) -> String {
    if report.files == 0 {
        return if report.unread_folders.unwrap_or(0) > 0 {
            "Drive size unknown"
        } else {
            "Drive is empty"
        }
        .to_owned();
    }
    if report.unread_folders.unwrap_or(0) > 0 {
        return format!(
            "{} stored · some folders unavailable",
            format_bytes(report.held_bytes)
        );
    }
    if holds_all(report) {
        return "All Drive files available offline".to_owned();
    }
    if report.held_bytes == 0 {
        return "No Drive data stored".to_owned();
    }
    let share = (u128::from(report.held_bytes) * 100) / u128::from(report.file_bytes.max(1));
    if share < 1 {
        format!(
            "{} stored · under 1% of Drive",
            format_bytes(report.held_bytes)
        )
    } else {
        format!(
            "{} stored · {share}% of Drive",
            format_bytes(report.held_bytes)
        )
    }
}

/// The room a device uses: "Uses 2.3 GB of 5.0 GB allowed".
#[must_use]
pub fn space_label(report: &DeviceStorageReport) -> String {
    match report.capacity_bytes.filter(|capacity| *capacity > 0) {
        Some(capacity) => format!(
            "Uses {} of {} allowed",
            format_bytes(report.stored_bytes),
            format_bytes(capacity)
        ),
        None => format!("Uses {}", format_bytes(report.stored_bytes)),
    }
}

#[must_use]
pub fn age_label(at: u64, now: u64) -> String {
    let minutes = now.saturating_sub(at) / 60;
    if minutes < 2 {
        return "Updated just now".to_owned();
    }
    if minutes < 60 {
        return format!("Updated {minutes} min ago");
    }
    let hours = minutes / 60;
    if hours < 48 {
        return format!("Updated {hours} h ago");
    }
    format!("Updated {} days ago", hours / 24)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncState {
    InSync,
    Behind,
    Ahead,
    Apart,
}

impl SyncState {
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::InSync => "in-sync",
            Self::Behind => "behind",
            Self::Ahead => "ahead",
            Self::Apart => "apart",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::InSync => "In sync",
            Self::Behind => "Missing recent changes",
            Self::Ahead => "Has changes not here yet",
            Self::Apart => "Not in sync",
        }
    }

    /// What it means, for a pointer resting on it.
    #[must_use]
    pub const fn detail(self) -> &'static str {
        match self {
            Self::InSync => "Has the same version of your Drive as this device",
            Self::Behind => "Has not taken in all the changes this device has yet",
            Self::Ahead => "Has changes from another device that have not reached this one yet",
            Self::Apart => "Each has changes the other has not taken in yet",
        }
    }
}

/// Whether `device` had taken in the same version of every other device's
/// Drive as this one holds, as it last reported; `None` when it has not said.
#[must_use]
pub fn sync_state(
    device: &str,
    report: Option<&DeviceStorageReport>,
    held: &BTreeMap<String, u64>,
    devices: &[String],
) -> Option<SyncState> {
    let seen = report?.seen.as_ref()?;
    let mut behind = false;
    let mut ahead = false;
    for other in devices.iter().filter(|other| *other != device) {
        let theirs = seen.get(other).copied().unwrap_or(0);
        let ours = held.get(other).copied().unwrap_or(0);
        behind |= theirs < ours;
        ahead |= theirs > ours;
    }
    Some(match (behind, ahead) {
        (true, true) => SyncState::Apart,
        (true, false) => SyncState::Behind,
        (false, true) => SyncState::Ahead,
        (false, false) => SyncState::InSync,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderLine {
    pub label: String,
    pub depth: usize,
    pub bytes: u64,
    pub held: u64,
}

impl FolderLine {
    /// "All 1.2 GB" or "300.0 MB of 1.2 GB".
    #[must_use]
    pub fn detail(&self) -> String {
        if self.held >= self.bytes {
            format!("All {}", format_bytes(self.bytes))
        } else {
            format!(
                "{} of {}",
                format_bytes(self.held),
                format_bytes(self.bytes)
            )
        }
    }
}

/// The folders of a report as lines to show, each under its parent, then the
/// files in no folder and, as one line, the folders too small to list.
#[must_use]
pub fn folder_lines(report: &DeviceStorageReport) -> Vec<FolderLine> {
    let folders = report.folders.as_deref().unwrap_or_default();
    let mut lines = folders
        .iter()
        .map(|folder| FolderLine {
            label: folder
                .path
                .last()
                .filter(|name| !name.is_empty())
                .cloned()
                .unwrap_or_else(|| "Folder".to_owned()),
            depth: folder.path.len().saturating_sub(1),
            bytes: folder.bytes,
            held: folder.held,
        })
        .collect::<Vec<_>>();
    let top = folders.iter().filter(|folder| folder.path.len() == 1);
    let (top_bytes, top_held) = top.fold((0_u64, 0_u64), |(bytes, held), folder| {
        (bytes + folder.bytes, held + folder.held)
    });
    let loose = report.loose.clone().unwrap_or_default();
    if loose.files > 0 {
        lines.push(FolderLine {
            label: if loose.files == 1 {
                "1 file not in a folder".to_owned()
            } else {
                format!("{} files not in a folder", thousands(loose.files))
            },
            depth: 0,
            bytes: loose.bytes,
            held: loose.held,
        });
    }
    let rest_bytes =
        i128::from(report.file_bytes) - i128::from(loose.bytes) - i128::from(top_bytes);
    let rest_held = i128::from(report.held_bytes) - i128::from(loose.held) - i128::from(top_held);
    if rest_bytes > 0 && report.loose.is_some() {
        let bytes = u64::try_from(rest_bytes).unwrap_or(u64::MAX);
        lines.push(FolderLine {
            label: if folders.is_empty() {
                "Folders"
            } else {
                "Other folders"
            }
            .to_owned(),
            depth: 0,
            bytes,
            held: u64::try_from(rest_held.clamp(0, rest_bytes)).unwrap_or(0),
        });
    }
    lines
}

/// One device as the Storage page lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRowInput {
    pub pubkey: String,
    pub name: String,
    pub current: bool,
    /// Linked directly to this device now.
    pub connected: bool,
    pub report: Option<DeviceStorageReport>,
    /// When it last sent its report.
    pub heard_at: Option<u64>,
}

/// The size of the Drive: as this device measured it, or else the newest report's.
#[must_use]
pub fn drive_size(rows: &[DeviceRowInput]) -> Option<&DeviceStorageReport> {
    if let Some(own) = rows
        .iter()
        .find(|row| row.current)
        .and_then(|row| row.report.as_ref())
        .filter(|report| report.unread_folders.unwrap_or(0) == 0)
    {
        return Some(own);
    }
    rows.iter()
        .filter_map(|row| row.report.as_ref())
        .filter(|report| report.unread_folders.unwrap_or(0) == 0)
        .max_by_key(|report| report.measured_at)
}

/// Which devices hold all of the Drive, in a sentence.
#[must_use]
pub fn holders_sentence(rows: &[DeviceRowInput]) -> String {
    let holders = rows
        .iter()
        .filter(|row| {
            row.report
                .as_ref()
                .is_some_and(|report| report.files > 0 && holds_all(report))
        })
        .collect::<Vec<_>>();
    match holders.as_slice() {
        [] => String::new(),
        [one] if one.current => "Every Drive file is available offline on this device.".to_owned(),
        [one] => format!("{} has every Drive file available offline.", one.name),
        many => format!(
            "Every Drive file is available offline on {} devices.",
            many.len()
        ),
    }
}

/// The line above the devices.
#[must_use]
pub fn summary(rows: &[DeviceRowInput]) -> String {
    match drive_size(rows) {
        Some(drive) => format!(
            "Your Drive is {} in {} {}. {}",
            format_bytes(drive.file_bytes),
            thousands(drive.files),
            if drive.files == 1 { "file" } else { "files" },
            holders_sentence(rows)
        )
        .trim_end()
        .to_owned(),
        None => String::new(),
    }
}

/// The rows in the order the page lists them: this device, then those that
/// reported, holding most first, then by name.
pub fn sort_rows(rows: &mut [DeviceRowInput]) {
    rows.sort_by(|a, b| {
        b.current
            .cmp(&a.current)
            .then(b.report.is_some().cmp(&a.report.is_some()))
            .then(
                b.report
                    .as_ref()
                    .map_or(0, |report| report.held_bytes)
                    .cmp(&a.report.as_ref().map_or(0, |report| report.held_bytes)),
            )
            .then_with(|| a.name.cmp(&b.name))
    });
}

/// How full a bar is, in thousandths of `of`.
#[must_use]
pub fn permille(part: u64, of: u64) -> u32 {
    let value = (u128::from(part) * 1000) / u128::from(of.max(1));
    u32::try_from(value.min(1000)).unwrap_or(1000)
}
