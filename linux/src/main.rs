use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::rc::Rc;
use std::sync::mpsc;
use std::time::Duration;

use adw::prelude::*;
use gtk::{gio, glib};
use serde_json::Value;

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

thread_local! {
    static TRAY_APP_HOLD: RefCell<Option<gio::ApplicationHoldGuard>> = const { RefCell::new(None) };
}

#[derive(Clone)]
struct Ui {
    sidebar: gtk::Box,
    setup: gtk::Box,
    sidebar_online: gtk::Label,
    main_view: gtk::ScrolledWindow,
    main: gtk::Box,
    drive_title: gtk::Label,
    drive_message: gtk::Label,
    status_pill: gtk::Label,
    status: gtk::Label,
    folder: gtk::Label,
    owner: gtk::Label,
    device: gtk::Label,
    snapshot: gtk::Label,
    files: gtk::Label,
    storage: gtk::Label,
    devices: gtk::Label,
    account_owner: gtk::Label,
    account_device: gtk::Label,
    account_authorization: gtk::Label,
    approve_box: gtk::Box,
    approve_device_button: gtk::Button,
    reset_invite_button: gtk::Button,
    notice: gtk::Label,
    drives: gtk::ListBox,
    peers: gtk::ListBox,
    backups: gtk::ListBox,
    fips: gtk::ListBox,
    relays: gtk::ListBox,
    blossom: gtk::ListBox,
    tray_on_close: gtk::CheckButton,
    local_nhash_resolver: gtk::CheckButton,
    logout_button: gtk::Button,
    relay_entry: gtk::Entry,
    backup_entry: gtk::Entry,
    backup_label_entry: gtk::Entry,
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
    Restore,
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
    let _app_lock = match AppInstanceLock::acquire() {
        Ok(lock) => lock,
        Err(error) => {
            eprintln!("{error}");
            return glib::ExitCode::SUCCESS;
        }
    };

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_startup(|_| {
        gtk::Window::set_default_icon_name("iris-drive");
        install_css();
    });
    app.connect_activate(build_ui);
    app.run()
}
