//! What a device reports: how much of the Drive it holds, how much room it
//! has and which versions of the other devices' Drives it took in. The same
//! fields as Iris Drive web's `DeviceStorageReport`
//! (`src/drive/deviceStorageReport.ts`), checked the same way on reading.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

const MAX_FOLDERS: usize = 64;
const MAX_NAME: usize = 255;
const MAX_DEPTH: usize = 8;
const MAX_DEVICES: usize = 256;
const MAX_APP: usize = 32;
/// The largest whole number a JavaScript number holds exactly.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// Files and how much of them the device holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStorageFiles {
    pub files: u64,
    pub bytes: u64,
    pub held: u64,
}

/// A folder that makes up a real share of the Drive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStorageFolder {
    /// Names from the top of the Drive down.
    pub path: Vec<String>,
    /// Size of the files in it, subfolders included.
    pub bytes: u64,
    /// How much of that the device holds.
    pub held: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceStorageReport {
    pub v: u8,
    /// Unix seconds.
    pub measured_at: u64,
    /// `web`, `macos`, `ios`, `android`, `windows` or `linux`.
    pub app: String,
    /// The Drive version measured, as the hex hash of its root.
    pub root: String,
    pub files: u64,
    /// Size of all files in the Drive.
    pub file_bytes: u64,
    /// Files whose whole content the device holds.
    pub held_files: u64,
    pub held_bytes: u64,
    /// Everything the app stores on the device, the Drive and more.
    pub stored_bytes: u64,
    /// The most the app may store there: stored plus free disk, or a limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capacity_bytes: Option<u64>,
    /// Folders that could not be read when measuring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unread_folders: Option<u64>,
    /// The newest version of each other device's Drive taken in, by the
    /// device's key: the `app_key_seq` of its root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seen: Option<BTreeMap<String, u64>>,
    /// Files at the top of the Drive, in no folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loose: Option<DeviceStorageFiles>,
    /// The folders that make up a real share of the Drive, a folder before
    /// its subfolders, larger ones first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folders: Option<Vec<DeviceStorageFolder>>,
}

/// The app kind this build reports.
#[must_use]
pub const fn current_app() -> &'static str {
    if cfg!(target_os = "ios") {
        "ios"
    } else if cfg!(target_os = "android") {
        "android"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    }
}

fn count(value: Option<&Value>) -> Option<u64> {
    value
        .and_then(Value::as_u64)
        .filter(|value| *value <= MAX_SAFE_INTEGER)
}

fn is_hex_key(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn truncate(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

/// The report with every field checked; `None` when it is not one. Bad
/// optional fields are left out, and a device never holds more than there is.
#[must_use]
pub fn read_device_storage_report(value: &Value) -> Option<DeviceStorageReport> {
    let raw = value.as_object()?;
    if raw.get("v").and_then(Value::as_u64) != Some(1) {
        return None;
    }
    let number = |name: &str| count(raw.get(name));
    let root = raw
        .get("root")
        .and_then(Value::as_str)
        .filter(|root| is_hex_key(root))?;
    let files = number("files")?;
    let file_bytes = number("file_bytes")?;
    let mut report = DeviceStorageReport {
        v: 1,
        measured_at: number("measured_at")?,
        app: raw
            .get("app")
            .and_then(Value::as_str)
            .map(|app| truncate(app, MAX_APP))
            .unwrap_or_default(),
        root: root.to_owned(),
        files,
        file_bytes,
        held_files: number("held_files")?.min(files),
        held_bytes: number("held_bytes")?.min(file_bytes),
        stored_bytes: number("stored_bytes")?,
        capacity_bytes: number("capacity_bytes"),
        unread_folders: number("unread_folders").filter(|unread| *unread > 0),
        seen: None,
        loose: None,
        folders: None,
    };
    if let Some(seen) = raw.get("seen").and_then(Value::as_object) {
        report.seen = Some(
            seen.iter()
                .take(MAX_DEVICES)
                .filter(|(device, _)| is_hex_key(device))
                .filter_map(|(device, seq)| Some((device.clone(), count(Some(seq))?)))
                .collect(),
        );
    }
    if let Some(loose) = raw.get("loose").and_then(Value::as_object)
        && let (Some(files), Some(bytes), Some(held)) = (
            count(loose.get("files")),
            count(loose.get("bytes")),
            count(loose.get("held")),
        )
    {
        report.loose = Some(DeviceStorageFiles {
            files,
            bytes,
            held: held.min(bytes),
        });
    }
    if let Some(folders) = raw.get("folders").and_then(Value::as_array) {
        report.folders = Some(
            folders
                .iter()
                .take(MAX_FOLDERS)
                .filter_map(read_folder)
                .collect(),
        );
    }
    Some(report)
}

fn read_folder(value: &Value) -> Option<DeviceStorageFolder> {
    let folder = value.as_object()?;
    let path = folder.get("path")?.as_array()?;
    if path.is_empty() || path.len() > MAX_DEPTH {
        return None;
    }
    let path = path
        .iter()
        .map(|name| name.as_str().map(|name| truncate(name, MAX_NAME)))
        .collect::<Option<Vec<_>>>()?;
    let bytes = count(folder.get("bytes"))?;
    let held = count(folder.get("held"))?;
    Some(DeviceStorageFolder {
        path,
        bytes,
        held: held.min(bytes),
    })
}

/// Whether `next` tells the other devices something `last` did not.
#[must_use]
pub fn report_moved(last: &DeviceStorageReport, next: &DeviceStorageReport) -> bool {
    #[allow(clippy::cast_precision_loss)]
    let moved =
        |a: u64, b: u64| a.abs_diff(b) as f64 > (1024.0 * 1024.0_f64).max(0.01 * a.max(b) as f64);
    let paths = |report: &DeviceStorageReport| {
        report
            .folders
            .iter()
            .flatten()
            .map(|folder| folder.path.join("\0"))
            .collect::<Vec<_>>()
    };
    if last.files != next.files || last.held_files != next.held_files || paths(last) != paths(next)
    {
        return true;
    }
    if moved(last.file_bytes, next.file_bytes)
        || moved(last.held_bytes, next.held_bytes)
        || moved(last.stored_bytes, next.stored_bytes)
        || moved(
            last.capacity_bytes.unwrap_or_default(),
            next.capacity_bytes.unwrap_or_default(),
        )
    {
        return true;
    }
    if last.seen.clone().unwrap_or_default() != next.seen.clone().unwrap_or_default() {
        return true;
    }
    let last_folders = last.folders.as_deref().unwrap_or_default();
    next.folders
        .iter()
        .flatten()
        .enumerate()
        .any(|(index, folder)| {
            moved(
                folder.held,
                last_folders.get(index).map_or(0, |last| last.held),
            )
        })
}
