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

pub(crate) fn reset_relays(model: &AppRef) {
    match run_idrive(["relays", "reset"]) {
        Ok(()) => refresh(model),
        Err(error) => model.ui.notice.set_text(&error),
    }
}

pub(crate) fn approve_device(model: &AppRef) {
    let device = model.ui.approve_device_entry.text().trim().to_string();
    if device.is_empty() {
        model.ui.notice.set_text("Device key is required");
        return;
    }

    let mut args = vec!["approve".to_string(), device];
    let label = model.ui.approve_label_entry.text().trim().to_string();
    if !label.is_empty() {
        args.push("--label".to_string());
        args.push(label);
    }

    match run_idrive_owned(&args) {
        Ok(()) => {
            model.ui.approve_device_entry.set_text("");
            model.ui.approve_label_entry.set_text("");
            restart_daemon(model);
            model.ui.notice.set_text("Device approved");
            refresh(model);
        }
        Err(error) => model.ui.notice.set_text(&error),
    }
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
        Ok(link) => copy_text(model, &link, "Snapshot copied"),
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
