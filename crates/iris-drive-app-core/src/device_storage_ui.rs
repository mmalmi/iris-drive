//! The Storage page's devices for the shells, worded by the core
//! (`iris_drive_core::device_storage::rows`), named and marked connected as
//! the device list shows them.

use std::path::Path;

use iris_drive_core::app_key_summary::pubkey_npub;
use iris_drive_core::device_storage::rows::device_storage_view;
use iris_drive_core::{AppConfig, ProfileState};

use crate::state::{UiAppActor, UiDeviceStorage, UiDeviceStorageFolder, UiDeviceStorageRow};

#[must_use]
pub(crate) fn device_storage_ui(
    data_dir: &Path,
    config: &AppConfig,
    state: &ProfileState,
    actors: &[UiAppActor],
    now: u64,
) -> UiDeviceStorage {
    let actor = |pubkey: &str| {
        let npub = pubkey_npub(pubkey);
        actors
            .iter()
            .find(|actor| actor.actor_kind == "device" && actor.pubkey == npub)
            .cloned()
    };
    let view = device_storage_view(
        data_dir,
        config,
        state,
        |pubkey| actor(pubkey).map(|actor| actor.display_label),
        |pubkey| actor(pubkey).is_some_and(|actor| actor.connection_state == "direct"),
        now,
    );
    UiDeviceStorage {
        summary: view.summary,
        devices: view
            .devices
            .into_iter()
            .map(|row| UiDeviceStorageRow {
                pubkey: row.pubkey,
                npub: row.npub,
                name: row.name,
                is_current: row.is_current,
                app: row.app,
                icon: row.icon,
                has_report: row.has_report,
                held_label: row.held_label,
                holds_all: row.holds_all,
                held_permille: row.held_permille,
                connected: row.connected,
                connection_label: row.connection_label,
                connection_detail: row.connection_detail,
                sync_state: row.sync_state,
                sync_label: row.sync_label,
                sync_detail: row.sync_detail,
                space_label: row.space_label,
                space_detail: row.space_detail,
                age_label: row.age_label,
                folders: row
                    .folders
                    .into_iter()
                    .map(|folder| UiDeviceStorageFolder {
                        label: folder.label,
                        depth: folder.depth,
                        detail: folder.detail,
                        held_permille: folder.held_permille,
                    })
                    .collect(),
                notes: row.notes,
            })
            .collect(),
    }
}
