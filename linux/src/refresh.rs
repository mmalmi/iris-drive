#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn refresh(model: &AppRef) {
    match desktop_state() {
        Ok(state) => {
            let initialized = state.ui.profile.is_some();
            let revoked = initialized && is_revoked(&state);
            let awaiting_link_approval =
                initialized && !revoked && is_awaiting_link_approval(&state);
            let sync_running = initialized && !revoked && ensure_daemon_running(model);
            set_view_mode(
                model,
                initialized && !awaiting_link_approval && !revoked,
                sync_running,
            );
            if !initialized {
                render_setup(model);
                return;
            }
            if revoked {
                stop_daemon_processes(model);
                render_revoked_device(model, &state);
                return;
            }
            if awaiting_link_approval {
                render_awaiting_approval(model, &state, sync_running);
                return;
            }
            model.ui.drive_title.set_text(&drive_name(&state));
            let primary_status_label = primary_status_label_value(&state);
            model.ui.drive_message.set_text(primary_status_label);
            model.ui.status_pill.set_text(primary_status_label);
            model.ui.status.set_text(primary_status_label);
            model.ui.folder.set_text(&drive_mount_text(&state));
            let account = profile(&state);
            let app_key_npub = account.map(|account| account.current_app_key_npub.as_str());
            let device_npub = account.map(|account| account.current_app_key_npub.as_str());
            model.ui.app_key.set_text(&short_value(app_key_npub));
            model.ui.device.set_text(&short_value(device_npub));
            model
                .ui
                .account_app_key
                .set_text(app_key_npub.unwrap_or("-"));
            model.ui.account_device.set_text(device_npub.unwrap_or("-"));
            model
                .ui
                .account_authorization
                .set_text(setup_label_value(&state));
            model
                .ui
                .approve_box
                .set_visible(account.is_some_and(|account| account.can_admin_profile));
            model.ui.snapshot.set_text(&snapshot_value(&state));
            model.ui.files.set_text(&file_count_value(&state));
            model.ui.storage.set_text(&storage_value(&state));
            model.ui.devices.set_text(&app_key_count_value(&state));
            model
                .ui
                .sidebar_online
                .set_text(&sidebar_online_value(&state));
            model.settings_refreshing.set(true);
            model
                .ui
                .local_nhash_resolver
                .set_active(local_nhash_resolver_enabled(&state));
            model.settings_refreshing.set(false);
            model
                .ui
                .recovery_phrase_button
                .set_visible(account.is_some_and(|account| account.can_export_recovery_phrase));
            let has_snapshot = snapshot_link(&state).is_some();
            model.ui.copy_snapshot_button.set_sensitive(has_snapshot);
            model.ui.open_snapshot_button.set_sensitive(has_snapshot);
            model
                .ui
                .copy_share_identity_button
                .set_sensitive(state.ui.profile.is_some());
            if state.ui.last_share_invite.is_empty() {
                model.ui.last_share_invite.set_text("-");
                model.ui.copy_last_share_invite_button.set_sensitive(false);
            } else {
                model
                    .ui
                    .last_share_invite
                    .set_text(&state.ui.last_share_invite);
                model.ui.copy_last_share_invite_button.set_sensitive(true);
            }
            render_drives(&model.ui.drives, &state);
            render_peers(model, &state);
            render_backups(model, &state);
            render_shares(model, &state);
            render_network(&model.ui.fips, &model.ui.relays, &model.ui.blossom, &state);
        }
        Err(error) => {
            set_view_mode(model, true, daemon_is_running(model));
            model.ui.drive_title.set_text("My Drive");
            model.ui.drive_message.set_text("Unavailable");
            model.ui.status_pill.set_text("Paused");
            model.ui.status.set_text("Unavailable");
            model
                .ui
                .folder
                .set_text(&default_drive_dir().display().to_string());
            model.ui.app_key.set_text("-");
            model.ui.device.set_text("-");
            model.ui.account_app_key.set_text("-");
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
            model.ui.last_share_invite.set_text("-");
            model.ui.copy_last_share_invite_button.set_sensitive(false);
            model.ui.recovery_phrase_button.set_visible(false);
            model.ui.notice.set_text(&error);
            clear_list(&model.ui.drives);
            clear_list(&model.ui.peers);
            clear_list(&model.ui.backups);
            clear_list(&model.ui.shares);
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
    model
        .ui
        .start_button
        .set_visible(initialized && !sync_running);
    model
        .ui
        .start_button
        .set_sensitive(initialized && !sync_running);
    model
        .ui
        .stop_button
        .set_visible(initialized && sync_running);
    model
        .ui
        .stop_button
        .set_sensitive(initialized && sync_running);
}
