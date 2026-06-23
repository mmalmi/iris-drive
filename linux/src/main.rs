use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

use adw::prelude::*;
use gtk::{gio, glib};
use iris_drive_app_core::{
    LinkInputClassification, NativeAppAction, NativeAppState, UiState, UpdateAutoCheckPolicy,
    classify_link_input,
};

mod actions;
mod daemon_control;
mod data;
mod platform;
mod refresh;
mod render;
mod setup;
mod tray;
mod ui;
mod updater;
mod widgets;

use actions::*;
use daemon_control::*;
use data::*;
use platform::*;
use refresh::*;
use render::*;
use setup::*;
use tray::*;
use ui::*;
use widgets::*;

const APP_ID: &str = "to.iris.drive";
const RECOVERY_PHRASE_WORD_COUNT: usize = 12;

thread_local! {
    static TRAY_APP_HOLD: RefCell<Option<gio::ApplicationHoldGuard>> = const { RefCell::new(None) };
    static APP_INSTANCE_LOCK: RefCell<Option<AppInstanceLock>> = const { RefCell::new(None) };
    static ACTIVE_MODEL: RefCell<Option<AppRef>> = const { RefCell::new(None) };
    static PENDING_LAUNCH_INPUTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone)]
struct Ui {
    sidebar: gtk::Box,
    update_bar: gtk::Box,
    update_label: gtk::Label,
    update_auto_check: gtk::CheckButton,
    update_auto_install: gtk::CheckButton,
    update_install_button: gtk::Button,
    update_check_button: gtk::Button,
    update_status: gtk::Label,
    setup: gtk::Box,
    stack: gtk::Stack,
    sidebar_online: gtk::Label,
    main_view: gtk::ScrolledWindow,
    main: gtk::Box,
    drive_title: gtk::Label,
    drive_message: gtk::Label,
    status_pill: gtk::Label,
    status: gtk::Label,
    folder: gtk::Label,
    app_key: gtk::Label,
    device: gtk::Label,
    snapshot: gtk::Label,
    files: gtk::Label,
    storage: gtk::Label,
    devices: gtk::Label,
    account_app_key: gtk::Label,
    account_device: gtk::Label,
    account_authorization: gtk::Label,
    approve_box: gtk::Box,
    add_device_expander: gtk::Expander,
    add_device_invite: gtk::Label,
    copy_invite_button: gtk::Button,
    add_device_requests: gtk::ListBox,
    add_device_entry: gtk::Entry,
    add_device_label_entry: gtk::Entry,
    add_device_submit_button: gtk::Button,
    add_recovery_key_button: gtk::Button,
    reset_invite_button: gtk::Button,
    notice: gtk::Label,
    drives: gtk::ListBox,
    peers: gtk::ListBox,
    backups: gtk::ListBox,
    shares: gtk::ListBox,
    fips: gtk::ListBox,
    relays: gtk::ListBox,
    blossom: gtk::ListBox,
    tray_on_close: gtk::CheckButton,
    launch_on_startup: gtk::CheckButton,
    local_nhash_resolver: gtk::CheckButton,
    open_sites_portal_button: gtk::Button,
    caldav_url: gtk::Label,
    caldav_server_path: gtk::Label,
    caldav_port: gtk::Label,
    copy_caldav_url_button: gtk::Button,
    recovery_phrase_button: gtk::Button,
    logout_button: gtk::Button,
    relay_entry: gtk::Entry,
    backup_entry: gtk::Entry,
    backup_label_entry: gtk::Entry,
    share_source_entry: gtk::Entry,
    share_invite_entry: gtk::Entry,
    create_share_button: gtk::Button,
    accept_share_invite_button: gtk::Button,
    last_share_invite: gtk::Label,
    copy_last_share_invite_button: gtk::Button,
    copy_share_identity_button: gtk::Button,
    init_button: gtk::Button,
    folder_button: gtk::Button,
    copy_snapshot_button: gtk::Button,
    open_snapshot_button: gtk::Button,
    start_button: gtk::Button,
    stop_button: gtk::Button,
}

struct AppModel {
    application: adw::Application,
    ui: Ui,
    daemon: RefCell<Option<Child>>,
    setup_screen: RefCell<SetupScreen>,
    setup_username: RefCell<String>,
    setup_recovery_words: RefCell<Vec<String>>,
    setup_recovery_word_index: Cell<usize>,
    tray: RefCell<Option<TrayServiceHandle>>,
    tray_available: Cell<bool>,
    settings_refreshing: Cell<bool>,
    update: RefCell<updater::UpdateState>,
    update_policy: RefCell<UpdateAutoCheckPolicy>,
    update_sender: mpsc::Sender<updater::UpdateEvent>,
    update_receiver: RefCell<mpsc::Receiver<updater::UpdateEvent>>,
    backup_check_sender: mpsc::Sender<BackupCheckEvent>,
    backup_check_receiver: RefCell<mpsc::Receiver<BackupCheckEvent>>,
    backup_checking: Cell<bool>,
    tray_sync_running: Arc<AtomicBool>,
    closed_to_tray: Cell<bool>,
    launch_on_startup_synced: Cell<Option<bool>>,
    retired: Cell<bool>,
    quit_requested: Cell<bool>,
}

type AppRef = Rc<AppModel>;

enum BackupCheckEvent {
    Progress { checked: usize, total: usize },
    Finished(Result<String, String>),
}

#[cfg(target_os = "linux")]
type TrayServiceHandle = ksni::blocking::Handle<IrisDriveTray>;

#[cfg(not(target_os = "linux"))]
type TrayServiceHandle = ();

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SetupScreen {
    Welcome,
    Create,
    CreatePhoto,
    RestoreOptions,
    RestorePhrase,
    RestoreSecretKey,
    Link,
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TrayCommand {
    Show,
    OpenDriveFolder,
    StartSync,
    StopSync,
    Quit,
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN | gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(|app| {
        match AppInstanceLock::acquire() {
            Ok(lock) => {
                APP_INSTANCE_LOCK.with(|slot| {
                    *slot.borrow_mut() = Some(lock);
                });
            }
            Err(error) => {
                eprintln!("{error}");
                app.quit();
                return;
            }
        }
        gtk::Window::set_default_icon_name("iris-drive");
        install_css();
    });
    app.connect_activate(|app| {
        if let Some(window) = app.active_window() {
            window.present();
            return;
        }
        build_ui(app, true);
    });
    app.connect_open(|app, files, _hint| {
        if app.active_window().is_none() {
            build_ui(app, true);
        }
        for file in files {
            handle_launch_input(&file.uri());
        }
        if let Some(window) = app.active_window() {
            window.present();
        }
    });
    app.connect_command_line(|app, command_line| {
        let mut present = true;
        for arg in command_line.arguments() {
            let arg = arg.to_string_lossy();
            if arg == HIDDEN_LAUNCH_ARG {
                present = false;
                continue;
            }
            if arg.starts_with("iris-drive://") || arg.starts_with("https://drive.iris.to/") {
                handle_launch_input(&arg);
                present = true;
            }
        }
        if let Some(window) = app.active_window() {
            if present {
                window.present();
            }
        } else {
            build_ui(app, present);
        }
        glib::ExitCode::SUCCESS.into()
    });
    app.run()
}

fn register_active_model(model: &AppRef) {
    ACTIVE_MODEL.with(|slot| {
        *slot.borrow_mut() = Some(Rc::clone(model));
    });
}

fn drain_pending_launch_inputs(model: &AppRef) {
    let pending = PENDING_LAUNCH_INPUTS.with(|slot| slot.take());
    for input in pending {
        apply_launch_input(model, &input);
    }
}

fn handle_launch_input(input: &str) {
    let handled = ACTIVE_MODEL.with(|slot| {
        if let Some(model) = slot.borrow().as_ref() {
            apply_launch_input(model, input);
            true
        } else {
            false
        }
    });
    if !handled {
        PENDING_LAUNCH_INPUTS.with(|slot| slot.borrow_mut().push(input.to_owned()));
    }
}

fn apply_launch_input(model: &AppRef, input: &str) {
    let classification = classify_link_input(input.to_owned());
    if classification.kind == "share_dialog" {
        apply_share_dialog_link(model, &classification);
    } else if classification.kind == "nhash_file" || classification.kind == "mutable_file" {
        open_content_link(model, &classification);
    } else if classification.kind == "invite" {
        apply_invite_link(model, input, &classification);
    }
}

fn apply_invite_link(model: &AppRef, input: &str, classification: &LinkInputClassification) {
    if !classification.is_valid {
        model.ui.notice.set_text(if classification.error.trim().is_empty() {
            "Could not open invite link"
        } else {
            classification.error.trim()
        });
        return;
    }
    match relink_device(input) {
        Ok(()) => refresh(model),
        Err(error) => model.ui.notice.set_text(&error),
    }
}

fn open_content_link(model: &AppRef, classification: &LinkInputClassification) {
    let display_name = classification.open_display_name.trim();
    let label = if display_name.is_empty() {
        "file"
    } else {
        display_name
    };
    if !classification.is_valid || classification.local_open_url.trim().is_empty() {
        model.ui.notice.set_text(if classification.error.trim().is_empty() {
            "Could not open content link"
        } else {
            classification.error.trim()
        });
        return;
    }
    let Some(window) = model.application.active_window() else {
        model.ui.notice.set_text(&format!("Opening {label}"));
        open_uri(classification.local_open_url.trim());
        return;
    };
    let dialog = adw::AlertDialog::builder()
        .heading(format!("Open {label}?"))
        .body("Open it now or save a copy to Iris Drive.")
        .close_response("cancel")
        .default_response("open")
        .build();
    dialog.add_responses(&[
        ("cancel", "Cancel"),
        ("save", "Save to Drive"),
        ("open", "Open"),
    ]);
    let model = model.clone();
    let classification = classification.clone();
    let label = label.to_owned();
    dialog.choose(&window, None::<&gio::Cancellable>, move |response| {
        if response == "open" {
            model.ui.notice.set_text(&format!("Opening {label}"));
            open_uri(classification.local_open_url.trim());
        } else if response == "save" {
            save_content_link(&model, &classification, &label);
        }
    });
}

fn save_content_link(model: &AppRef, classification: &LinkInputClassification, label: &str) {
    let link = classification.normalized_input.trim();
    if link.is_empty() {
        model.ui.notice.set_text(&format!("Could not save {label}"));
        return;
    }
    model.ui.notice.set_text(&format!("Saving {label}"));
    match dispatch_desktop_action(NativeAppAction::ImportContentLink {
        link: link.to_owned(),
    }) {
        Ok(_state) => {
            model.ui.notice.set_text(&format!("Saved {label} to Iris Drive"));
            refresh(model);
        }
        Err(error) => {
            model.ui.notice.set_text(&error);
        }
    }
}

fn apply_share_dialog_link(model: &AppRef, classification: &LinkInputClassification) {
    model.ui.stack.set_visible_child_name("shares");
    if !classification.is_valid || classification.share_source_path.trim().is_empty() {
        model
            .ui
            .notice
            .set_text(if classification.error.trim().is_empty() {
                "Share folder path is required."
            } else {
                classification.error.trim()
            });
        return;
    }

    model
        .ui
        .share_source_entry
        .set_text(classification.share_source_path.trim());

    let recipient = first_non_empty([
        classification.share_recipient_display_name.as_str(),
        classification.share_recipient_npub_hint.as_str(),
        classification.share_recipient_profile_id.as_str(),
    ]);
    if recipient.is_empty() {
        model.ui.notice.set_text("Share folder selected");
    } else {
        model
            .ui
            .notice
            .set_text(&format!("Share folder selected for {recipient}"));
    }
}

fn first_non_empty(values: [&str; 3]) -> &str {
    values
        .into_iter()
        .map(str::trim)
        .find(|value| !value.is_empty())
        .unwrap_or("")
}

fn render_update_state(model: &AppRef) {
    let update = model.update.borrow();
    model.ui.update_bar.set_visible(update.available);
    model.ui.update_label.set_text(&update_stripe_text(
        &update.version,
        env!("CARGO_PKG_VERSION"),
    ));
    model.ui.update_install_button.set_sensitive(
        update.available && update.asset.is_some() && !update.checking && !update.downloading,
    );
    model
        .ui
        .update_check_button
        .set_sensitive(!update.checking && !update.downloading);
    model.ui.update_status.set_text(&update.status);
    model.settings_refreshing.set(true);
    model.ui.update_auto_check.set_active(update.auto_check);
    model.ui.update_auto_install.set_active(update.auto_install);
    model.settings_refreshing.set(false);
}

fn set_auto_check_updates(model: &AppRef, enabled: bool) {
    {
        let mut update = model.update.borrow_mut();
        update.auto_check = enabled;
    }
    write_auto_check_updates(enabled);
    render_update_state(model);
    if enabled {
        check_updates_if_due(model);
    }
}

fn set_auto_install_updates(model: &AppRef, enabled: bool) {
    let should_download = {
        let mut update = model.update.borrow_mut();
        update.auto_install = enabled;
        enabled && update.available && update.asset.is_some()
    };
    write_auto_install_updates(enabled);
    render_update_state(model);
    if should_download {
        download_update(model);
    }
}

fn check_updates(model: &AppRef, manual: bool) {
    let (sender, config_dir) = {
        let mut update = model.update.borrow_mut();
        if update.checking || update.downloading {
            return;
        }
        if manual {
            model
                .update_policy
                .borrow_mut()
                .note_manual_check_started(Instant::now());
            update.status = "Checking for updates".to_string();
        }
        update.checking = true;
        (model.update_sender.clone(), app_config_dir())
    };
    render_update_state(model);
    updater::check(
        env!("CARGO_PKG_VERSION").to_string(),
        config_dir,
        manual,
        sender,
    );
}

fn check_updates_if_due(model: &AppRef) {
    let due = {
        let update = model.update.borrow();
        let enabled = update.auto_check;
        drop(update);
        model
            .update_policy
            .borrow_mut()
            .should_start_check(enabled, Instant::now())
    };
    if due {
        check_updates(model, false);
    }
}

fn download_update(model: &AppRef) {
    let (asset, sender, config_dir) = {
        let mut update = model.update.borrow_mut();
        if update.checking || update.downloading {
            return;
        }
        let Some(asset) = update.asset.clone() else {
            update.status = "No Linux update asset found".to_string();
            render_update_state(model);
            return;
        };
        update.downloading = true;
        update.status = format!("Downloading {}", update.version);
        (asset, model.update_sender.clone(), app_config_dir())
    };
    render_update_state(model);
    updater::download(
        env!("CARGO_PKG_VERSION").to_string(),
        config_dir,
        asset,
        sender,
    );
}

fn drain_update_events(model: &AppRef) {
    let events = {
        let receiver = model.update_receiver.borrow();
        receiver.try_iter().collect::<Vec<_>>()
    };
    if events.is_empty() {
        return;
    }

    let mut auto_download = false;
    {
        let mut update = model.update.borrow_mut();
        for event in events {
            match event {
                updater::UpdateEvent::Checked { manual, result } => {
                    update.checking = false;
                    match result {
                        Ok(check) => {
                            update.available = check.newer;
                            update.version = check.tag.clone();
                            update.asset = if check.newer { check.asset } else { None };
                            if check.newer {
                                update.status = if update.asset.is_some() {
                                    format!("Update {} available", check.tag)
                                } else {
                                    format!(
                                        "Update {} found without a Linux desktop asset",
                                        check.tag
                                    )
                                };
                                auto_download = update.auto_install && update.asset.is_some();
                            } else if manual {
                                update.status = "Up to date".to_string();
                            } else {
                                update.status.clear();
                            }
                        }
                        Err(error) => {
                            if manual {
                                update.status = error;
                            } else {
                                update.status.clear();
                            }
                        }
                    }
                }
                updater::UpdateEvent::Downloaded(result) => {
                    update.downloading = false;
                    match result {
                        Ok(path) => {
                            update.status = format!(
                                "Downloaded {}",
                                path.file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or("update")
                            );
                        }
                        Err(error) => {
                            update.status = error;
                        }
                    }
                }
            }
        }
    }

    if auto_download {
        download_update(model);
    } else {
        render_update_state(model);
    }
}

fn drain_backup_check_events(model: &AppRef) {
    let events = {
        let receiver = model.backup_check_receiver.borrow();
        receiver.try_iter().collect::<Vec<_>>()
    };
    if events.is_empty() {
        return;
    }

    for event in events {
        match event {
            BackupCheckEvent::Progress { checked, total } => {
                model
                    .ui
                    .notice
                    .set_text(&format!("Checking {checked} of {total}"));
            }
            BackupCheckEvent::Finished(result) => {
                model.backup_checking.set(false);
                match result {
                    Ok(message) => model.ui.notice.set_text(&message),
                    Err(error) => model.ui.notice.set_text(&error),
                }
                refresh(model);
            }
        }
    }
}

fn update_stripe_text(version: &str, current: &str) -> String {
    let version = version.trim();
    let current = current.trim();
    if current.is_empty() {
        format!("Update available: {version}")
    } else {
        format!("Update available: {version} (you're on {current})")
    }
}

fn update_preferences_path() -> PathBuf {
    app_config_dir().join("linux-update-preferences")
}

fn read_auto_check_updates() -> bool {
    read_update_preference("auto_check").unwrap_or(true)
}

fn write_auto_check_updates(enabled: bool) {
    write_update_preference("auto_check", enabled);
}

fn read_auto_install_updates() -> bool {
    read_update_preference("auto_install").unwrap_or(false)
}

fn write_auto_install_updates(enabled: bool) {
    write_update_preference("auto_install", enabled);
}

fn read_update_preference(name: &str) -> Option<bool> {
    let contents = std::fs::read_to_string(update_preferences_path()).ok()?;
    contents.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        if key.trim() == name {
            Some(value.trim() == "true")
        } else {
            None
        }
    })
}

fn write_update_preference(name: &str, enabled: bool) {
    let path = update_preferences_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut preferences = std::collections::BTreeMap::new();
    if let Ok(contents) = std::fs::read_to_string(&path) {
        for line in contents.lines() {
            if let Some((key, value)) = line.split_once('=') {
                preferences.insert(key.trim().to_string(), value.trim().to_string());
            }
        }
    }
    preferences.insert(name.to_string(), enabled.to_string());
    let contents = preferences
        .into_iter()
        .map(|(key, value)| format!("{key}={value}\n"))
        .collect::<String>();
    let _ = std::fs::write(path, contents);
}

fn update_poll_interval_secs() -> u64 {
    std::env::var("IRIS_DRIVE_UPDATE_POLL_SECONDS")
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .filter(|seconds| *seconds > 0)
        .unwrap_or(6 * 60 * 60)
}
