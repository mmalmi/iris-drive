//! The Storage page's devices, worded, from the reports this device keeps on
//! disk: its own last measurement and the newest report of each other
//! device. Nothing here waits for relays. The apps and `idrive storage` show
//! these as they are.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Serialize;

use super::state::{DeviceStorageState, profile_devices};
use super::view::{
    DeviceRowInput, age_label, folder_lines, holding_label, holds_all, permille, sort_rows,
    space_label, summary, sync_state,
};
use crate::app_key_summary::pubkey_npub;
use crate::config::AppConfig;
use crate::{PRIMARY_DRIVE_ID, ProfileState};

const CONNECTED_DETAIL: &str = "Linked directly to this device";
const NOT_CONNECTED_DETAIL: &str =
    "Not linked directly to this device now; changes still reach it through relays";
const SPACE_DETAIL: &str = "Everything Drive keeps on that device, and the most it may keep";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DeviceStorageView {
    pub summary: String,
    pub devices: Vec<DeviceStorageRowView>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[allow(clippy::struct_excessive_bools)]
pub struct DeviceStorageRowView {
    pub pubkey: String,
    pub npub: String,
    pub name: String,
    pub is_current: bool,
    pub app: String,
    /// `web`, `phone`, `computer` or `drive`.
    pub icon: String,
    pub has_report: bool,
    pub held_label: String,
    pub holds_all: bool,
    pub held_permille: u32,
    pub connected: bool,
    pub connection_label: String,
    pub connection_detail: String,
    pub sync_state: String,
    pub sync_label: String,
    pub sync_detail: String,
    pub space_label: String,
    pub space_detail: String,
    pub age_label: String,
    pub folders: Vec<DeviceStorageFolderView>,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DeviceStorageFolderView {
    pub label: String,
    pub depth: u32,
    pub detail: String,
    pub held_permille: u32,
}

fn icon(app: &str) -> &'static str {
    match app {
        "web" => "web",
        "ios" | "android" => "phone",
        "macos" | "windows" | "linux" => "computer",
        _ => "drive",
    }
}

/// The profile's devices as the Storage page lists them. `name` gives a
/// device's name by its hex key, and `connected` whether it is linked
/// directly to this one now.
#[must_use]
pub fn device_storage_view(
    config_dir: &Path,
    config: &AppConfig,
    state: &ProfileState,
    name: impl Fn(&str) -> Option<String>,
    connected: impl Fn(&str) -> bool,
    now: u64,
) -> DeviceStorageView {
    let devices = profile_devices(config, state);
    let stored = DeviceStorageState::load(config_dir, &state.root_scope_id());
    let mut rows = devices
        .iter()
        .map(|pubkey| {
            let current = *pubkey == state.app_key_pubkey;
            let heard = stored.heard.get(pubkey);
            DeviceRowInput {
                pubkey: pubkey.clone(),
                name: name(pubkey)
                    .map(|name| name.trim().to_owned())
                    .filter(|name| !name.is_empty())
                    .unwrap_or_else(|| short_npub(&pubkey_npub(pubkey))),
                current,
                connected: !current && connected(pubkey),
                report: if current {
                    stored.own.clone()
                } else {
                    heard.map(|heard| heard.report.clone())
                },
                heard_at: heard.map(|heard| heard.created_at),
            }
        })
        .collect::<Vec<_>>();
    sort_rows(&mut rows);
    let roots = config
        .drive(PRIMARY_DRIVE_ID)
        .map_or_else(Default::default, |drive| drive.app_key_roots.clone());
    let held = roots
        .iter()
        .map(|(device, root)| (device.clone(), root.app_key_seq))
        .collect::<BTreeMap<_, _>>();
    // A device whose newest root here is local bookkeeping, never sent, is
    // not compared: no other device can have taken that root in.
    let compared = devices
        .iter()
        .filter(|device| !roots.get(*device).is_some_and(|root| root.local_only))
        .cloned()
        .collect::<Vec<_>>();
    DeviceStorageView {
        summary: summary(&rows),
        devices: rows
            .iter()
            .map(|row| row_view(row, &held, &compared, now))
            .collect(),
    }
}

fn row_view(
    row: &DeviceRowInput,
    held: &BTreeMap<String, u64>,
    devices: &[String],
    now: u64,
) -> DeviceStorageRowView {
    let report = row.report.as_ref();
    let sync = (!row.current)
        .then(|| sync_state(&row.pubkey, report, held, devices))
        .flatten();
    let mut notes = Vec::new();
    let folders = report.map_or_else(Vec::new, |report| {
        let lines = folder_lines(report);
        if lines.is_empty() {
            notes.push("No files yet".to_owned());
        }
        if report.unread_folders.is_some() {
            notes.push("Some folders could not be read when it measured.".to_owned());
        }
        lines
            .iter()
            .map(|line| DeviceStorageFolderView {
                label: line.label.clone(),
                depth: u32::try_from(line.depth).unwrap_or(u32::MAX),
                detail: line.detail(),
                held_permille: permille(line.held, line.bytes),
            })
            .collect()
    });
    let (connection_label, connection_detail) = match (row.current, row.connected) {
        (true, _) => ("", ""),
        (false, true) => ("Connected", CONNECTED_DETAIL),
        (false, false) => ("Not connected", NOT_CONNECTED_DETAIL),
    };
    let age = report.map_or_else(String::new, |report| {
        let at = if row.current {
            report.measured_at
        } else {
            row.heard_at.unwrap_or(report.measured_at)
        };
        age_label(at, now)
    });
    DeviceStorageRowView {
        pubkey: row.pubkey.clone(),
        npub: pubkey_npub(&row.pubkey),
        name: row.name.clone(),
        is_current: row.current,
        app: report.map_or_else(Default::default, |report| report.app.clone()),
        icon: icon(report.map_or("", |report| report.app.as_str())).to_owned(),
        has_report: report.is_some(),
        held_label: report.map_or_else(|| "No report yet".to_owned(), holding_label),
        holds_all: report.is_some_and(|report| report.files > 0 && holds_all(report)),
        held_permille: report.map_or(0, |report| permille(report.held_bytes, report.file_bytes)),
        connected: row.connected,
        connection_label: connection_label.to_owned(),
        connection_detail: connection_detail.to_owned(),
        sync_state: sync.map_or_else(Default::default, |sync| sync.id().to_owned()),
        sync_label: sync.map_or_else(Default::default, |sync| sync.label().to_owned()),
        sync_detail: sync.map_or_else(Default::default, |sync| sync.detail().to_owned()),
        space_label: report.map_or_else(Default::default, space_label),
        space_detail: if report.is_some() {
            SPACE_DETAIL.to_owned()
        } else {
            String::new()
        },
        age_label: age,
        folders,
        notes,
    }
}

fn short_npub(npub: &str) -> String {
    let start = npub.chars().take(12).collect::<String>();
    format!("{start}…")
}
