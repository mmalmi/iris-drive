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
        .title("Add a device")
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

    let title = gtk::Label::new(Some("Add a device"));
    title.add_css_class("title-2");
    title.set_xalign(0.0);
    body.append(&title);

    if let Ok(state) = desktop_state() {
        if let Some(requests) = account(&state)
            .map(|account| account.inbound_device_link_requests.as_slice())
            .filter(|requests| !requests.is_empty())
        {
            let heading = gtk::Label::new(Some("Devices asking to join"));
            heading.add_css_class("iris-row-title");
            heading.set_xalign(0.0);
            body.append(&heading);
            for request in requests {
                let request_url = request.request_link.clone();
                let request_label = if request.label.is_empty() {
                    "New device".to_string()
                } else {
                    request.label.clone()
                };
                let request_device = request.device_pubkey.clone();
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
                            model.ui.notice.set_text("Device request rejected");
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
                        if approve_device_values(&model, request_url.clone(), request_label.clone()) {
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
        "Paste the Device ID shown on the other device when you link it manually.",
    ));
    help.add_css_class("iris-muted");
    help.set_xalign(0.0);
    help.set_wrap(true);
    body.append(&help);

    let device = setup_entry("Device ID");
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
        model.ui.notice.set_text("Device key is required");
        return false;
    }

    let label = label.trim().to_string();

    match dispatch_desktop_action(NativeAppAction::ApproveDevice {
        request: device,
        label,
    }) {
        Ok(_) => {
            restart_daemon(model);
            model.ui.notice.set_text("Device approved");
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
        .title("Remove device")
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

    let title = gtk::Label::new(Some("Remove device?"));
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
                model.ui.notice.set_text("Device removed");
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
            let message = if key == "owner_npub" {
                "Owner key copied"
            } else {
                "Device key copied"
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
    let account = account(&state).ok_or_else(|| "No account key available".to_string())?;
    match key {
        "owner_npub" => Ok(account.owner_pubkey.clone()),
        "device_npub" => Ok(account.device_pubkey.clone()),
        _ => Err("No account key available".to_string()),
    }
}
