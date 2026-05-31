#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn add_relay(model: &AppRef) {
    let relay = model.ui.relay_entry.text().trim().to_string();
    if relay.is_empty() {
        return;
    }
    match run_idrive_owned(&["relays".to_string(), "add".to_string(), relay]) {
        Ok(()) => {
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
    let mut args = vec!["backups".to_string(), "add".to_string(), target];
    let label = model.ui.backup_label_entry.text().trim().to_string();
    if !label.is_empty() {
        args.push("--label".to_string());
        args.push(label);
    }

    match run_idrive_owned(&args) {
        Ok(()) => {
            model.ui.backup_entry.set_text("");
            model.ui.backup_label_entry.set_text("");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn sync_backups(model: &AppRef) {
    match run_idrive(["backups", "sync"]) {
        Ok(()) => {
            model.ui.notice.set_text("Backups synced");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn check_backups(model: &AppRef) {
    match run_idrive(["backups", "check"]) {
        Ok(()) => {
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
    match run_idrive(["relays", "reset"]) {
        Ok(()) => refresh(model),
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn reset_invite(model: &AppRef) {
    match run_idrive(["devices", "reset-invite"]) {
        Ok(()) => {
            model.ui.notice.set_text("Invite reset");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn logout(model: &AppRef) {
    stop_daemon(model);
    match run_idrive(["logout"]) {
        Ok(()) => {
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

    let mut args = vec!["approve".to_string(), device];
    let label = label.trim().to_string();
    if !label.is_empty() {
        args.push("--label".to_string());
        args.push(label);
    }

    match run_idrive_owned(&args) {
        Ok(()) => {
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
        .title("Delete device")
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

    let title = gtk::Label::new(Some("Delete device?"));
    title.add_css_class("title-2");
    title.set_xalign(0.0);
    body.append(&title);

    let message = gtk::Label::new(Some(&format!(
        "Delete {label} from Iris Drive? This removes its access to future syncs."
    )));
    message.add_css_class("iris-muted");
    message.set_xalign(0.0);
    message.set_wrap(true);
    body.append(&message);

    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    buttons.set_halign(gtk::Align::End);
    let cancel = pill_button("Cancel");
    let delete = pill_button("Delete");
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
                model.ui.notice.set_text("Device deleted");
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
    let status = run_idrive_json(["status"]).unwrap_or(Value::Null);
    if !ensure_daemon_running(model, &status) {
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
        if let Ok(status) = run_idrive_json(["status"])
            && let Some(folder) = mounted_dir(&status)
            && wait_for_path(&folder, Duration::from_millis(100))
        {
            return Some(folder);
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let fallback = default_drive_dir();
    if wait_for_path(&fallback, Duration::from_millis(100)) {
        return Some(fallback);
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
    let json = run_idrive_json(["status"])?;
    snapshot_link(&json)
        .map(str::to_string)
        .ok_or_else(|| "No snapshot available".to_string())
}

pub(crate) fn current_account_value(key: &str) -> Result<String, String> {
    let json = run_idrive_json(["status"])?;
    let account = account_json(&json);
    find_string(account, &[key])
        .map(str::to_string)
        .ok_or_else(|| "No account key available".to_string())
}
