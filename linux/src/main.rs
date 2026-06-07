use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::{Arc, OnceLock, mpsc};
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use iris_drive_app_core::{
    LinkInputClassification, NativeAppAction, NativeAppState, UiState, classify_link_input,
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
    approve_device_button: gtk::Button,
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
    local_nhash_resolver: gtk::CheckButton,
    recovery_phrase_button: gtk::Button,
    logout_button: gtk::Button,
    relay_entry: gtk::Entry,
    backup_entry: gtk::Entry,
    backup_label_entry: gtk::Entry,
    share_source_entry: gtk::Entry,
    share_name_entry: gtk::Entry,
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
    closed_to_tray: Cell<bool>,
    retired: Cell<bool>,
    quit_requested: Cell<bool>,
}

type AppRef = Rc<AppModel>;

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
    Logout,
    Quit,
}

fn main() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
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
        build_ui(app);
    });
    app.connect_open(|app, files, _hint| {
        if app.active_window().is_none() {
            build_ui(app);
        }
        for file in files {
            handle_launch_input(&file.uri());
        }
        if let Some(window) = app.active_window() {
            window.present();
        }
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
    }
}

fn apply_share_dialog_link(model: &AppRef, classification: &LinkInputClassification) {
    model.ui.stack.set_visible_child_name("shares");
    if !classification.is_valid || classification.share_source_path.trim().is_empty() {
        model.ui.notice.set_text(
            if classification.error.trim().is_empty() {
                "Share folder path is required."
            } else {
                classification.error.trim()
            },
        );
        return;
    }

    model
        .ui
        .share_source_entry
        .set_text(classification.share_source_path.trim());
    model
        .ui
        .share_name_entry
        .set_text(classification.share_display_name.trim());

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
