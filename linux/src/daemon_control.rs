#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn start_daemon(model: &AppRef) {
    if desktop_state().is_ok_and(|state| is_revoked(&state)) {
        stop_daemon(model);
        model.ui.notice.set_text("Device removed");
        return;
    }
    if ensure_daemon_running(model) {
        model.ui.notice.set_text("Sync is already on");
        return;
    }
    model.ui.notice.set_text("Could not start sync");
}

pub(crate) fn restart_daemon(model: &AppRef) {
    stop_daemon(model);
    start_daemon(model);
    refresh(model);
}

pub(crate) fn ensure_daemon_running(model: &AppRef) -> bool {
    if daemon_is_running(model) || daemon_lock_is_running() {
        return true;
    }

    match spawn_daemon() {
        Ok(child) => {
            *model.daemon.borrow_mut() = Some(child);
            model.ui.notice.set_text("Sync resumed");
            true
        }
        Err(error) => {
            model
                .ui
                .notice
                .set_text(&format!("Could not start sync: {error}"));
            false
        }
    }
}

pub(crate) fn spawn_daemon() -> Result<Child, std::io::Error> {
    Command::new(idrive_path())
        .arg("daemon")
        .arg("--watch-interval")
        .arg("2")
        .arg("--watch-debounce-ms")
        .arg("100")
        .arg("--mount")
        .arg("--mountpoint")
        .arg(default_drive_dir())
        .env("IRIS_DRIVE_PARENT_PID", std::process::id().to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
}

pub(crate) fn daemon_is_running(model: &AppRef) -> bool {
    let mut daemon = model.daemon.borrow_mut();
    let Some(child) = daemon.as_mut() else {
        return false;
    };
    match child.try_wait() {
        Ok(None) => true,
        Ok(Some(_)) | Err(_) => {
            *daemon = None;
            false
        }
    }
}

pub(crate) fn daemon_lock_is_running() -> bool {
    daemon_lock_pid().is_some_and(process_is_running)
}

pub(crate) fn daemon_lock_pid() -> Option<u32> {
    let Ok(contents) = std::fs::read_to_string(app_config_dir().join("daemon.lock")) else {
        return None;
    };
    contents.trim().parse::<u32>().ok()
}

pub(crate) fn process_is_running(pid: u32) -> bool {
    Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn terminate_process(pid: u32) -> bool {
    if pid == std::process::id() || !process_is_running(pid) {
        return false;
    }

    let _ = Command::new("/bin/kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
    for _ in 0..15 {
        if !process_is_running(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let _ = Command::new("/bin/kill")
        .arg("-KILL")
        .arg(pid.to_string())
        .status();
    !process_is_running(pid)
}

pub(crate) fn terminate_child(child: &mut Child) -> bool {
    if child.try_wait().ok().flatten().is_some() {
        return false;
    }

    let pid = child.id();
    let _ = Command::new("/bin/kill")
        .arg("-TERM")
        .arg(pid.to_string())
        .status();
    for _ in 0..15 {
        if child.try_wait().ok().flatten().is_some() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let _ = child.kill();
    let _ = child.wait();
    true
}

pub(crate) struct AppInstanceLock {
    path: PathBuf,
}

impl AppInstanceLock {
    pub(crate) fn acquire() -> Result<Self, String> {
        let dir = app_config_dir();
        std::fs::create_dir_all(&dir)
            .map_err(|error| format!("Could not create config dir {}: {error}", dir.display()))?;
        let path = dir.join("app.lock");

        match Self::try_create(&path) {
            Ok(lock) => Ok(lock),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                Self::replace_stale_or_error(&path)
            }
            Err(error) => Err(format!(
                "Could not create app lock {}: {error}",
                path.display()
            )),
        }
    }

    fn replace_stale_or_error(path: &Path) -> Result<Self, String> {
        if let Ok(contents) = std::fs::read_to_string(path)
            && let Ok(pid) = contents.trim().parse::<u32>()
            && process_is_running(pid)
        {
            return Err("Iris Drive is already running.".to_string());
        }

        let _ = std::fs::remove_file(path);
        Self::try_create(path).map_err(|error| {
            format!(
                "Could not replace stale app lock {}: {error}",
                path.display()
            )
        })
    }

    fn try_create(path: &Path) -> std::io::Result<Self> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        writeln!(file, "{}", std::process::id())?;
        Ok(Self {
            path: path.to_path_buf(),
        })
    }
}

impl Drop for AppInstanceLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) fn app_config_dir() -> PathBuf {
    if let Some(path) = std::env::var_os("IRIS_DRIVE_CONFIG_DIR") {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(path).join("iris-drive");
    }
    if let Some(path) = std::env::var_os("HOME") {
        return PathBuf::from(path).join(".config/iris-drive");
    }
    PathBuf::from(".").join(".config/iris-drive")
}

pub(crate) fn close_to_tray_config_path() -> PathBuf {
    app_config_dir().join("linux-close-to-tray-on-close")
}

pub(crate) fn read_close_to_tray_on_close() -> bool {
    std::fs::read_to_string(close_to_tray_config_path())
        .map(|value| value.trim() != "false")
        .unwrap_or(true)
}

pub(crate) fn write_close_to_tray_on_close(enabled: bool) {
    let path = close_to_tray_config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(path, if enabled { "true\n" } else { "false\n" });
}

pub(crate) fn stop_daemon(model: &AppRef) {
    let stopped = stop_daemon_processes(model);

    if stopped {
        model.ui.notice.set_text("Sync paused");
    }
    refresh(model);
}

pub(crate) fn stop_daemon_processes(model: &AppRef) -> bool {
    let lock_pid = daemon_lock_pid();
    let mut stopped = false;
    let mut child_pid = None;

    if let Some(mut child) = model.daemon.borrow_mut().take() {
        let pid = child.id();
        child_pid = Some(pid);
        stopped |= terminate_child(&mut child);
    }

    if let Some(pid) = lock_pid
        && Some(pid) != child_pid
    {
        stopped |= terminate_process(pid);
    }

    stopped
}

pub(crate) fn desktop_state() -> Result<NativeAppState, String> {
    let state = desktop_core().refresh();
    if state.error.trim().is_empty() {
        Ok(state)
    } else {
        Err(state.error)
    }
}

pub(crate) fn dispatch_desktop_action(action: NativeAppAction) -> Result<NativeAppState, String> {
    let state = desktop_core().dispatch(action);
    if !state.error.trim().is_empty() {
        return Err(state.error);
    }
    Ok(state)
}

fn desktop_core() -> Arc<iris_drive_app_core::FfiApp> {
    static APP: OnceLock<Arc<iris_drive_app_core::FfiApp>> = OnceLock::new();
    APP.get_or_init(|| {
        iris_drive_app_core::FfiApp::new(
            app_config_dir().display().to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        )
    })
    .clone()
}

pub(crate) fn run_idrive<const N: usize>(args: [&str; N]) -> Result<(), String> {
    let output = Command::new(idrive_path())
        .args(args)
        .output()
        .map_err(|error| format!("idrive failed to start: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

pub(crate) fn run_idrive_output<const N: usize>(args: [&str; N]) -> Result<String, String> {
    let output = Command::new(idrive_path())
        .args(args)
        .output()
        .map_err(|error| format!("idrive failed to start: {error}"))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

pub(crate) fn run_idrive_owned(args: &[String]) -> Result<(), String> {
    let output = Command::new(idrive_path())
        .args(args)
        .output()
        .map_err(|error| format!("idrive failed to start: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
    }
}

pub(crate) fn idrive_path() -> PathBuf {
    if let Ok(path) = std::env::var("IRIS_DRIVE_CLI") {
        return PathBuf::from(path);
    }

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for candidate in [
        manifest.join("../target/debug/idrive"),
        manifest.join("../target/release/idrive"),
        manifest.join("../../target/debug/idrive"),
        manifest.join("../../target/release/idrive"),
    ] {
        if candidate.exists() {
            return candidate;
        }
    }
    PathBuf::from("idrive")
}

pub(crate) fn default_drive_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Iris Drive")
}
