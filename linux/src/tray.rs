#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn connect_tray(model: &AppRef, window: &adw::ApplicationWindow) {
    let (sender, receiver) = mpsc::channel();
    let tray = install_tray(sender);
    let available = tray.is_some();

    model.tray_available.set(available);
    if available && let Some(app) = window.application() {
        TRAY_APP_HOLD.with(|hold| {
            *hold.borrow_mut() = Some(app.hold());
        });
    }
    model.ui.tray_on_close.set_sensitive(available);
    model
        .ui
        .tray_on_close
        .set_active(read_close_to_tray_on_close());
    *model.tray.borrow_mut() = tray;

    if !available {
        return;
    }

    let model = Rc::clone(model);
    let window = window.clone();
    glib::timeout_add_local(Duration::from_millis(150), move || {
        if model.retired.get() {
            return glib::ControlFlow::Break;
        }
        while let Ok(command) = receiver.try_recv() {
            handle_tray_command(&model, &window, command);
        }
        glib::ControlFlow::Continue
    });
}

pub(crate) fn handle_tray_command(
    model: &AppRef,
    window: &adw::ApplicationWindow,
    command: TrayCommand,
) {
    match command {
        TrayCommand::Show => show_window(model, window),
        TrayCommand::OpenDriveFolder => open_drive_folder(model),
        TrayCommand::StartSync => {
            start_daemon(model);
            refresh(model);
        }
        TrayCommand::StopSync => {
            stop_daemon(model);
            refresh(model);
        }
        TrayCommand::Quit => quit_application(model, window),
    }
}

pub(crate) fn show_window(model: &AppRef, window: &adw::ApplicationWindow) {
    if !visible_app_window_exists().unwrap_or_else(|| {
        !model.closed_to_tray.get()
            && window.application().is_some()
            && window.is_mapped()
            && window.is_realized()
            && window.surface().is_some()
    }) {
        model.retired.set(true);
        shutdown_tray(model);
        build_ui(&model.application);
        return;
    }

    window.set_visible(true);
    window.unminimize();
    window.present();
    refresh(model);
}

pub(crate) fn visible_app_window_exists() -> Option<bool> {
    std::env::var_os("DISPLAY")?;

    let status = Command::new("xdotool")
        .args(["search", "--onlyvisible", "--name", "^Iris Drive$"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?;
    Some(status.success())
}

pub(crate) fn quit_application(model: &AppRef, window: &adw::ApplicationWindow) {
    model.quit_requested.set(true);
    window.set_hide_on_close(false);
    shutdown_tray(model);
    stop_daemon(model);

    if let Some(app) = window.application() {
        TRAY_APP_HOLD.with(|hold| {
            hold.borrow_mut().take();
        });
        app.quit();
    } else {
        window.close();
    }
}

pub(crate) fn shutdown_tray(model: &AppRef) {
    let tray = model.tray.borrow_mut().take();
    model.tray_available.set(false);
    model.ui.tray_on_close.set_sensitive(false);
    if let Some(tray) = tray {
        shutdown_tray_handle(tray);
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn shutdown_tray_handle(tray: TrayServiceHandle) {
    tray.shutdown().wait();
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn shutdown_tray_handle(_tray: TrayServiceHandle) {}

#[cfg(target_os = "linux")]
pub(crate) fn install_tray(sender: mpsc::Sender<TrayCommand>) -> Option<TrayServiceHandle> {
    use ksni::blocking::TrayMethods;

    let tray = IrisDriveTray { sender };
    match tray.spawn() {
        Ok(handle) => Some(handle),
        Err(error) => {
            eprintln!("Could not start system tray: {error}");
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) fn install_tray(_sender: mpsc::Sender<TrayCommand>) -> Option<TrayServiceHandle> {
    None
}

#[cfg(target_os = "linux")]
pub(crate) struct IrisDriveTray {
    sender: mpsc::Sender<TrayCommand>,
}

#[cfg(target_os = "linux")]
impl IrisDriveTray {
    fn send(&self, command: TrayCommand) {
        let _ = self.sender.send(command);
    }
}

#[cfg(target_os = "linux")]
impl ksni::Tray for IrisDriveTray {
    fn id(&self) -> String {
        APP_ID.to_string()
    }

    fn title(&self) -> String {
        "Iris Drive".to_string()
    }

    fn icon_name(&self) -> String {
        "iris-drive".to_string()
    }

    fn icon_theme_path(&self) -> String {
        if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
            return PathBuf::from(path).join("icons").display().to_string();
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join(".local/share/icons")
                .display()
                .to_string();
        }
        String::new()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        ksni::ToolTip {
            icon_name: "iris-drive".to_string(),
            title: "Iris Drive".to_string(),
            description: "Sync active".to_string(),
            ..Default::default()
        }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.send(TrayCommand::Show);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        vec![
            tray_menu_item(
                &self.sender,
                TrayCommand::Show,
                "Show Iris Drive",
                "window-new",
            ),
            tray_menu_item(
                &self.sender,
                TrayCommand::OpenDriveFolder,
                "Open Drive Folder",
                "folder-open",
            ),
            ksni::MenuItem::Separator,
            tray_menu_item(
                &self.sender,
                TrayCommand::StartSync,
                "Start Sync",
                "media-playback-start",
            ),
            tray_menu_item(
                &self.sender,
                TrayCommand::StopSync,
                "Stop Sync",
                "media-playback-stop",
            ),
            ksni::MenuItem::Separator,
            tray_menu_item(&self.sender, TrayCommand::Quit, "Quit", "application-exit"),
        ]
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn tray_menu_item(
    sender: &mpsc::Sender<TrayCommand>,
    command: TrayCommand,
    label: &str,
    icon_name: &str,
) -> ksni::MenuItem<IrisDriveTray> {
    let sender = sender.clone();
    ksni::menu::StandardItem {
        label: label.to_string(),
        icon_name: icon_name.to_string(),
        activate: Box::new(move |_| {
            let _ = sender.send(command);
        }),
        ..Default::default()
    }
    .into()
}
