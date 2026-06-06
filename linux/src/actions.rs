#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn add_relay(model: &AppRef) {
    let relay = model.ui.relay_entry.text().trim().to_string();
    if relay.is_empty() {
        return;
    }
    match dispatch_desktop_action(NativeAppAction::AddRelay { url: relay }) {
        Ok(_) => {
            model.ui.relay_entry.set_text("");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn add_backup_target(model: &AppRef) {
    let target = model.ui.backup_entry.text().trim().to_string();
    if target.is_empty() {
        return;
    }
    let label = model.ui.backup_label_entry.text().trim().to_string();

    match dispatch_desktop_action(NativeAppAction::AddBackupTarget { target, label }) {
        Ok(_) => {
            model.ui.backup_entry.set_text("");
            model.ui.backup_label_entry.set_text("");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn sync_backups(model: &AppRef) {
    match dispatch_desktop_action(NativeAppAction::SyncBackups {
        target: String::new(),
    }) {
        Ok(_) => {
            model.ui.notice.set_text("Backups synced");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn remove_backup_target(model: &AppRef, target: String) {
    match dispatch_desktop_action(NativeAppAction::RemoveBackupTarget { target }) {
        Ok(_) => refresh(model),
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn check_backup_target(model: &AppRef, target: String) {
    match dispatch_desktop_action(NativeAppAction::CheckBackups { target }) {
        Ok(_) => {
            model.ui.notice.set_text("Backup checked");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn check_backups(model: &AppRef) {
    match dispatch_desktop_action(NativeAppAction::CheckBackups {
        target: String::new(),
    }) {
        Ok(_) => {
            model.ui.notice.set_text("Backups checked");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn create_share(model: &AppRef) {
    let source_path = model.ui.share_source_entry.text().trim().to_string();
    if source_path.is_empty() {
        model.ui.notice.set_text("Folder path is required");
        return;
    }
    let display_name = model.ui.share_name_entry.text().trim().to_string();

    match dispatch_desktop_action(NativeAppAction::CreateShare {
        source_path,
        display_name,
    }) {
        Ok(_) => {
            model.ui.share_source_entry.set_text("");
            model.ui.share_name_entry.set_text("");
            model.ui.notice.set_text("Share created");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn accept_share_invite(model: &AppRef) {
    let invite = model.ui.share_invite_entry.text().trim().to_string();
    if invite.is_empty() {
        model.ui.notice.set_text("Share invite is required");
        return;
    }

    match dispatch_desktop_action(NativeAppAction::AcceptShareInvite { invite }) {
        Ok(_) => {
            model.ui.share_invite_entry.set_text("");
            model.ui.notice.set_text("Share accepted");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn copy_last_share_invite(model: &AppRef) {
    match desktop_state() {
        Ok(state) if !state.ui.last_share_invite.is_empty() => copy_text(
            model,
            &state.ui.last_share_invite,
            "Share invite copied",
        ),
        Ok(_) => model.ui.notice.set_text("No invite available"),
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn show_invite_share_member_dialog(
    model: &AppRef,
    share_id: String,
    share_name: String,
) {
    let dialog = gtk::Window::builder()
        .application(&model.application)
        .title("Invite to share")
        .modal(true)
        .default_width(460)
        .build();
    if let Some(parent) = model.application.active_window() {
        dialog.set_transient_for(Some(&parent));
    }

    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);

    let title = gtk::Label::new(Some(&format!("Invite to {share_name}")));
    title.add_css_class("title-2");
    title.set_xalign(0.0);
    body.append(&title);

    let profile_id = setup_entry("IrisProfile UUID");
    let app_key = setup_entry("Recipient AppKey");
    let npub_hint = setup_entry("Representative npub");
    let display_name = setup_entry("Name");
    let label = setup_entry("AppKey label");
    body.append(&profile_id);
    body.append(&app_key);
    body.append(&npub_hint);
    body.append(&display_name);
    body.append(&label);

    let role = gtk::DropDown::from_strings(&["reader", "editor", "admin"]);
    role.set_selected(0);
    body.append(&role);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = pill_button("Cancel");
    let invite = primary_button("Invite");
    buttons.append(&cancel);
    buttons.append(&invite);
    body.append(&buttons);

    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| dialog.close());
    }
    {
        let model = Rc::clone(model);
        let dialog = dialog.clone();
        let share_id = share_id.clone();
        invite.connect_clicked(move |_| {
            let profile_id = profile_id.text().trim().to_string();
            let app_key = app_key.text().trim().to_string();
            if profile_id.is_empty() || app_key.is_empty() {
                model
                    .ui
                    .notice
                    .set_text("IrisProfile UUID and AppKey are required");
                return;
            }
            let role = match role.selected() {
                1 => "editor",
                2 => "admin",
                _ => "reader",
            }
            .to_string();
            match dispatch_desktop_action(NativeAppAction::InviteShareMember {
                share_id: share_id.clone(),
                profile_id,
                app_key,
                role,
                representative_npub_hint: npub_hint.text().trim().to_string(),
                display_name: display_name.text().trim().to_string(),
                label: label.text().trim().to_string(),
            }) {
                Ok(state) => {
                    if state.ui.last_share_invite.is_empty() {
                        model.ui.notice.set_text("Share invite created");
                    } else {
                        copy_text(&model, &state.ui.last_share_invite, "Share invite copied");
                    }
                    refresh(&model);
                    dialog.close();
                }
                Err(error) => model.ui.notice.set_text(&error),
            }
        });
    }

    dialog.set_child(Some(&body));
    dialog.present();
}

pub(crate) fn repair_share_wraps(model: &AppRef, share_id: String) {
    match dispatch_desktop_action(NativeAppAction::RepairShareWraps { share_id }) {
        Ok(_) => {
            model.ui.notice.set_text("Share wraps repaired");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn add_share_shortcut(model: &AppRef, share_id: String, display_name: String) {
    let path = if display_name.trim().is_empty() {
        "Shared folder".to_string()
    } else {
        display_name.trim().to_string()
    };
    match dispatch_desktop_action(NativeAppAction::AddShareShortcut {
        share_id,
        path,
        parent: String::new(),
        target_path: String::new(),
    }) {
        Ok(_) => {
            model.ui.notice.set_text("Shortcut added");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn show_revoke_share_member_dialog(
    model: &AppRef,
    share_id: String,
    profile_id: String,
    display_name: String,
) {
    let dialog = gtk::Window::builder()
        .application(&model.application)
        .title("Revoke share member")
        .modal(true)
        .default_width(380)
        .build();
    if let Some(parent) = model.application.active_window() {
        dialog.set_transient_for(Some(&parent));
    }

    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);

    let title = gtk::Label::new(Some("Revoke access?"));
    title.add_css_class("title-2");
    title.set_xalign(0.0);
    body.append(&title);

    let message = gtk::Label::new(Some(&format!(
        "Revoke {display_name} from this share? Future key epochs will not be wrapped for this IrisProfile."
    )));
    message.add_css_class("iris-muted");
    message.set_xalign(0.0);
    message.set_wrap(true);
    body.append(&message);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = pill_button("Cancel");
    let revoke = pill_button("Revoke");
    revoke.add_css_class("destructive-action");
    buttons.append(&cancel);
    buttons.append(&revoke);
    body.append(&buttons);

    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| dialog.close());
    }
    {
        let model = Rc::clone(model);
        let dialog = dialog.clone();
        revoke.connect_clicked(move |_| {
            match dispatch_desktop_action(NativeAppAction::RevokeShareMember {
                share_id: share_id.clone(),
                profile_id: profile_id.clone(),
                reason: String::new(),
            }) {
                Ok(_) => {
                    model.ui.notice.set_text("Share member revoked");
                    refresh(&model);
                    dialog.close();
                }
                Err(error) => model.ui.notice.set_text(&error),
            }
        });
    }

    dialog.set_child(Some(&body));
    dialog.present();
}

pub(crate) fn set_local_nhash_resolver(model: &AppRef, enabled: bool) {
    let command = if enabled { "enable" } else { "disable" };
    match run_idrive(["nhash-resolver", command]) {
        Ok(()) => {
            restart_daemon(model);
            model.ui.notice.set_text(if enabled {
                "Local resolver enabled"
            } else {
                "Local resolver disabled"
            });
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn reset_relays(model: &AppRef) {
    match dispatch_desktop_action(NativeAppAction::ResetRelays) {
        Ok(_) => refresh(model),
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn reset_invite(model: &AppRef) {
    match dispatch_desktop_action(NativeAppAction::ResetInvite) {
        Ok(_) => {
            model.ui.notice.set_text("Invite reset");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn logout(model: &AppRef) {
    stop_daemon(model);
    match dispatch_desktop_action(NativeAppAction::Logout) {
        Ok(_) => {
            model.ui.notice.set_text("Logged out");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn show_add_device_dialog(model: &AppRef) {
    let dialog = gtk::Window::builder()
        .application(&model.application)
        .title("Add an AppKey")
        .modal(true)
        .default_width(420)
        .build();
    if let Some(parent) = model.application.active_window() {
        dialog.set_transient_for(Some(&parent));
    }

    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);

    let title = gtk::Label::new(Some("Add an AppKey"));
    title.add_css_class("title-2");
    title.set_xalign(0.0);
    body.append(&title);

    if let Ok(state) = desktop_state() {
        if let Some(requests) = profile(&state)
            .map(|account| account.inbound_app_key_link_requests.as_slice())
            .filter(|requests| !requests.is_empty())
        {
            let heading = gtk::Label::new(Some("AppKeys asking to join"));
            heading.add_css_class("iris-row-title");
            heading.set_xalign(0.0);
            body.append(&heading);
            for request in requests {
                let request_url = request.request_link.clone();
                let request_label = if request.label.is_empty() {
                    "New AppKey".to_string()
                } else {
                    request.label.clone()
                };
                let request_device = request.app_key_pubkey.clone();
                let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                row.set_valign(gtk::Align::Center);
                let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
                labels.set_hexpand(true);
                let title = gtk::Label::new(Some(&request_label));
                title.set_xalign(0.0);
                title.add_css_class("iris-row-title");
                labels.append(&title);
                let subtitle = gtk::Label::new(Some(&request_device));
                subtitle.set_xalign(0.0);
                subtitle.add_css_class("iris-row-subtitle");
                subtitle.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
                labels.append(&subtitle);
                row.append(&labels);
                let reject = pill_button("Reject");
                reject.add_css_class("destructive-action");
                let add_request = primary_button("Add");
                {
                    let model = Rc::clone(model);
                    let request_url = request_url.clone();
                    let dialog = dialog.clone();
                    reject.connect_clicked(move |_| match reject_device(&request_url) {
                        Ok(()) => {
                            model.ui.notice.set_text("AppKey request rejected");
                            refresh(&model);
                            dialog.close();
                        }
                        Err(error) => model.ui.notice.set_text(&error),
                    });
                }
                {
                    let model = Rc::clone(model);
                    let request_url = request_url.clone();
                    let request_label = request_label.clone();
                    let dialog = dialog.clone();
                    add_request.connect_clicked(move |_| {
                        if approve_device_values(&model, request_url.clone(), request_label.clone())
                        {
                            dialog.close();
                        }
                    });
                }
                row.append(&reject);
                row.append(&add_request);
                body.append(&row);
            }
        }
    }

    let help = gtk::Label::new(Some(
        "Paste the AppKey shown by the app install you want to approve.",
    ));
    help.add_css_class("iris-muted");
    help.set_xalign(0.0);
    help.set_wrap(true);
    body.append(&help);

    let device = setup_entry("AppKey");
    let label = setup_entry("Name (optional)");
    body.append(&device);
    body.append(&label);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = pill_button("Cancel");
    let add = primary_button("Add");
    buttons.append(&cancel);
    buttons.append(&add);
    body.append(&buttons);

    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| dialog.close());
    }
    {
        let model = Rc::clone(model);
        let dialog = dialog.clone();
        let device = device.clone();
        let label = label.clone();
        add.connect_clicked(move |_| {
            if approve_device_values(
                &model,
                device.text().trim().to_string(),
                label.text().trim().to_string(),
            ) {
                dialog.close();
            }
        });
    }
    {
        let add = add.clone();
        device.connect_activate(move |_| add.emit_clicked());
    }
    {
        let add = add.clone();
        label.connect_activate(move |_| add.emit_clicked());
    }

    dialog.set_child(Some(&body));
    dialog.present();
}

pub(crate) fn approve_device_values(model: &AppRef, device: String, label: String) -> bool {
    if device.trim().is_empty() {
        model.ui.notice.set_text("AppKey is required");
        return false;
    }

    let label = label.trim().to_string();

    match dispatch_desktop_action(NativeAppAction::ApproveDevice {
        request: device,
        label,
    }) {
        Ok(_) => {
            restart_daemon(model);
            model.ui.notice.set_text("AppKey approved");
            refresh(model);
            true
        }
        Err(error) => {
            model.ui.notice.set_text(&error);
            false
        }
    }
}

pub(crate) fn show_delete_device_dialog(model: &AppRef, device_npub: String, label: String) {
    let dialog = gtk::Window::builder()
        .application(&model.application)
        .title("Remove AppKey")
        .modal(true)
        .default_width(360)
        .build();
    if let Some(parent) = model.application.active_window() {
        dialog.set_transient_for(Some(&parent));
    }

    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);

    let title = gtk::Label::new(Some("Remove AppKey?"));
    title.add_css_class("title-2");
    title.set_xalign(0.0);
    body.append(&title);

    let message = gtk::Label::new(Some(&format!(
        "Remove {label} from Iris Drive? This removes its access to future syncs."
    )));
    message.add_css_class("iris-muted");
    message.set_xalign(0.0);
    message.set_wrap(true);
    body.append(&message);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = pill_button("Cancel");
    let delete = pill_button("Remove");
    delete.add_css_class("destructive-action");
    buttons.append(&cancel);
    buttons.append(&delete);
    body.append(&buttons);

    {
        let dialog = dialog.clone();
        cancel.connect_clicked(move |_| dialog.close());
    }
    {
        let model = Rc::clone(model);
        let dialog = dialog.clone();
        delete.connect_clicked(move |_| match delete_device(&device_npub) {
            Ok(()) => {
                restart_daemon(&model);
                model.ui.notice.set_text("AppKey removed");
                refresh(&model);
                dialog.close();
            }
            Err(error) => model.ui.notice.set_text(&error),
        });
    }

    dialog.set_child(Some(&body));
    dialog.present();
}

pub(crate) fn open_drive_folder(model: &AppRef) {
    if !ensure_daemon_running(model) {
        model.ui.notice.set_text("Could not start sync");
        return;
    }

    let Some(folder) = wait_for_mounted_dir(Duration::from_secs(3)) else {
        model.ui.notice.set_text("Drive mount unavailable");
        return;
    };
    open_path(&folder);
}

pub(crate) fn wait_for_mounted_dir(timeout: Duration) -> Option<PathBuf> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if let Ok(state) = desktop_state()
            && let Some(folder) = mounted_dir(&state)
            && wait_for_path(&folder, Duration::from_millis(100))
        {
            return Some(folder);
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let default_folder = default_drive_dir();
    if wait_for_path(&default_folder, Duration::from_millis(100)) {
        return Some(default_folder);
    }
    None
}

pub(crate) fn wait_for_path(path: &Path, timeout: Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    path.exists()
}

pub(crate) fn copy_snapshot_link(model: &AppRef) {
    match current_snapshot_link() {
        Ok(link) => copy_text(model, &link, "drive.iris.to link copied"),
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn copy_account_key(model: &AppRef, key: &str) {
    match current_account_value(key) {
        Ok(value) => {
            let message = if key == "app_key_npub" || key == "current_app_key_npub" {
                "AppKey copied"
            } else {
                "AppKey copied"
            };
            copy_text(model, &value, message);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn copy_text(model: &AppRef, value: &str, message: &str) {
    if let Some(display) = gtk::gdk::Display::default() {
        display.clipboard().set_text(value);
        model.ui.notice.set_text(message);
    } else {
        model.ui.notice.set_text("Clipboard unavailable");
    }
}

pub(crate) fn show_recovery_phrase_dialog(model: &AppRef) {
    let export =
        iris_drive_app_core::export_recovery_secret(app_config_dir().display().to_string());
    let dialog = gtk::Window::builder()
        .application(&model.application)
        .title("Recovery phrase")
        .modal(true)
        .default_width(420)
        .build();
    if let Some(parent) = model.application.active_window() {
        dialog.set_transient_for(Some(&parent));
    }

    let body = gtk::Box::new(gtk::Orientation::Vertical, 14);
    body.set_margin_top(18);
    body.set_margin_bottom(18);
    body.set_margin_start(18);
    body.set_margin_end(18);

    let title = gtk::Label::new(Some("Recovery phrase"));
    title.add_css_class("title-2");
    title.set_xalign(0.0);
    body.append(&title);

    if !export.error.trim().is_empty() {
        let error = gtk::Label::new(Some(&export.error));
        error.add_css_class("iris-muted");
        error.set_xalign(0.0);
        error.set_wrap(true);
        body.append(&error);
    } else {
        let word_index = Rc::new(Cell::new(0_usize));
        let word_label = gtk::Label::new(Some(&format!(
            "Word 1 of {}",
            RECOVERY_PHRASE_WORD_COUNT
        )));
        word_label.add_css_class("iris-field-name");
        word_label.set_xalign(0.0);
        body.append(&word_label);

        let word_value = gtk::Label::new(Some(export.words.first().map_or("", String::as_str)));
        word_value.add_css_class("title-1");
        word_value.set_halign(gtk::Align::Center);
        word_value.set_margin_top(4);
        word_value.set_margin_bottom(10);
        body.append(&word_value);

        let nav = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        nav.set_halign(gtk::Align::End);
        let back = pill_button("Back");
        let next = primary_button("Next");
        nav.append(&back);
        nav.append(&next);
        body.append(&nav);

        let copy_buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let copy_phrase = pill_button("Copy recovery phrase");
        let copy_key = pill_button("Copy secret key");
        copy_buttons.append(&copy_phrase);
        copy_buttons.append(&copy_key);
        body.append(&copy_buttons);

        let update = {
            let export = export.clone();
            let word_index = Rc::clone(&word_index);
            let word_label = word_label.clone();
            let word_value = word_value.clone();
            let back = back.clone();
            let next = next.clone();
            move || {
                let index = word_index.get().min(RECOVERY_PHRASE_WORD_COUNT - 1);
                word_label.set_text(&format!(
                    "Word {} of {}",
                    index + 1,
                    RECOVERY_PHRASE_WORD_COUNT
                ));
                word_value.set_text(export.words.get(index).map_or("", String::as_str));
                back.set_sensitive(index > 0);
                next.set_label(if index == RECOVERY_PHRASE_WORD_COUNT - 1 {
                    "Done"
                } else {
                    "Next"
                });
            }
        };
        update();

        {
            let word_index = Rc::clone(&word_index);
            let update = update.clone();
            back.connect_clicked(move |_| {
                word_index.set(word_index.get().saturating_sub(1));
                update();
            });
        }
        {
            let dialog = dialog.clone();
            let word_index = Rc::clone(&word_index);
            let update = update.clone();
            next.connect_clicked(move |_| {
                if word_index.get() >= RECOVERY_PHRASE_WORD_COUNT - 1 {
                    dialog.close();
                } else {
                    word_index.set((word_index.get() + 1).min(RECOVERY_PHRASE_WORD_COUNT - 1));
                    update();
                }
            });
        }
        {
            let model = Rc::clone(model);
            let phrase = export.recovery_phrase.clone();
            copy_phrase.connect_clicked(move |_| {
                copy_text(&model, &phrase, "Recovery phrase copied");
            });
        }
        {
            let model = Rc::clone(model);
            let secret_key = export.secret_key.clone();
            copy_key.connect_clicked(move |_| {
                copy_text(&model, &secret_key, "Secret key copied");
            });
        }
    }

    dialog.set_child(Some(&body));
    dialog.present();
}

pub(crate) fn open_snapshot_link(model: &AppRef) {
    match current_snapshot_link() {
        Ok(link) => open_uri(&link),
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn current_snapshot_link() -> Result<String, String> {
    let state = desktop_state()?;
    snapshot_link(&state)
        .map(str::to_string)
        .ok_or_else(|| "No snapshot available".to_string())
}

pub(crate) fn current_account_value(key: &str) -> Result<String, String> {
    let state = desktop_state()?;
    let account = profile(&state).ok_or_else(|| "No account key available".to_string())?;
    match key {
        "app_key_npub" | "current_app_key_npub" => Ok(account.current_app_key_npub.clone()),
        "device_npub" => Ok(account.current_app_key_npub.clone()),
        _ => Err("No account key available".to_string()),
    }
}
