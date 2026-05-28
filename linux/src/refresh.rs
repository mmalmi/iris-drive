#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn refresh(model: &AppRef) {
    match run_idrive_json(["status"]) {
        Ok(json) => {
            let initialized = json
                .get("initialized")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let awaiting_link_approval = initialized && is_awaiting_link_approval(&json);
            let sync_running = initialized && ensure_daemon_running(model, &json);
            set_view_mode(model, initialized && !awaiting_link_approval, sync_running);
            if !initialized {
                render_setup(model);
                return;
            }
            if awaiting_link_approval {
                render_awaiting_approval(model, &json, sync_running);
                return;
            }
            model.ui.drive_title.set_text(&drive_name(&json));
            model
                .ui
                .drive_message
                .set_text(if sync_running { "Running" } else { "Stopped" });
            model
                .ui
                .status_pill
                .set_text(if sync_running { "Running" } else { "Stopped" });
            model
                .ui
                .status
                .set_text(if sync_running { "Syncing" } else { "Ready" });
            model.ui.folder.set_text(&drive_mount_text(&json));
            let account = account_json(&json);
            let owner_npub = find_string(account, &["owner_npub"]);
            let device_npub = find_string(account, &["device_npub"]);
            let authorization = find_string(account, &["authorization_state"]).unwrap_or("-");
            model.ui.owner.set_text(&short_value(owner_npub));
            model.ui.device.set_text(&short_value(device_npub));
            model.ui.account_owner.set_text(owner_npub.unwrap_or("-"));
            model.ui.account_device.set_text(device_npub.unwrap_or("-"));
            model.ui.account_authorization.set_text(authorization);
            model
                .ui
                .approve_box
                .set_visible(find_bool(account, &["has_owner_signing_authority"]).unwrap_or(false));
            model.ui.snapshot.set_text(&snapshot_value(&json));
            model.ui.files.set_text(&file_count_value(&json));
            model.ui.storage.set_text(&storage_value(&json));
            model.ui.devices.set_text(&device_count_value(&json));
            model
                .ui
                .sidebar_online
                .set_text(&sidebar_online_value(&json));
            model.settings_refreshing.set(true);
            model
                .ui
                .local_nhash_resolver
                .set_active(local_nhash_resolver_enabled(&json));
            model.settings_refreshing.set(false);
            let has_snapshot = snapshot_link(&json).is_some();
            model.ui.copy_snapshot_button.set_sensitive(has_snapshot);
            model.ui.open_snapshot_button.set_sensitive(has_snapshot);
            render_drives(&model.ui.drives, &json);
            render_peers(model, &json);
            render_backups(&model.ui.backups, &json);
            render_network(&model.ui.fips, &model.ui.relays, &model.ui.blossom, &json);
        }
        Err(error) => {
            set_view_mode(model, true, daemon_is_running(model));
            model.ui.drive_title.set_text("My Drive");
            model.ui.drive_message.set_text("Unavailable");
            model.ui.status_pill.set_text("Stopped");
            model.ui.status.set_text("Unavailable");
            model
                .ui
                .folder
                .set_text(&default_drive_dir().display().to_string());
            model.ui.owner.set_text("-");
            model.ui.device.set_text("-");
            model.ui.account_owner.set_text("-");
            model.ui.account_device.set_text("-");
            model.ui.account_authorization.set_text("-");
            model.ui.approve_box.set_visible(false);
            model.ui.snapshot.set_text("-");
            model.ui.files.set_text("0");
            model.ui.storage.set_text("0 B");
            model.ui.devices.set_text("0/0");
            model.ui.sidebar_online.set_text("0/0 online");
            model.ui.copy_snapshot_button.set_sensitive(false);
            model.ui.open_snapshot_button.set_sensitive(false);
            model.ui.notice.set_text(&error);
            clear_list(&model.ui.drives);
            clear_list(&model.ui.peers);
            clear_list(&model.ui.backups);
            clear_list(&model.ui.relays);
            clear_list(&model.ui.blossom);
        }
    }
}

pub(crate) fn set_view_mode(model: &AppRef, initialized: bool, sync_running: bool) {
    model.ui.sidebar.set_visible(initialized);
    model.ui.setup.set_visible(!initialized);
    model.ui.main_view.set_visible(initialized);
    model.ui.main.set_visible(initialized);
    model.ui.init_button.set_visible(false);
    model.ui.folder_button.set_visible(initialized);
    model.ui.copy_snapshot_button.set_visible(initialized);
    model.ui.open_snapshot_button.set_visible(initialized);
    model.ui.start_button.set_visible(initialized);
    model
        .ui
        .start_button
        .set_sensitive(initialized && !sync_running);
    model.ui.stop_button.set_visible(initialized);
    model
        .ui
        .stop_button
        .set_sensitive(initialized && sync_running);
}
