#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn start_daemon(model: &AppRef) {
    let status = run_idrive_json(["status"]).unwrap_or(Value::Null);
    if ensure_daemon_running(model, &status) {
        model.ui.notice.set_text("Sync already running");
        return;
    }
    model.ui.notice.set_text("Could not start sync");
}

pub(crate) fn restart_daemon(model: &AppRef) {
    stop_daemon(model);
    start_daemon(model);
    refresh(model);
}

pub(crate) fn ensure_daemon_running(model: &AppRef, status: &Value) -> bool {
    if daemon_is_running(model) || daemon_lock_is_running(status) {
        return true;
    }

    match spawn_daemon() {
        Ok(child) => {
            *model.daemon.borrow_mut() = Some(child);
            model.ui.notice.set_text("Sync started");
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

pub(crate) fn daemon_lock_is_running(status: &Value) -> bool {
    let Some(config_dir) = status.get("config_dir").and_then(Value::as_str) else {
        return false;
    };
    let Ok(contents) = std::fs::read_to_string(PathBuf::from(config_dir).join("daemon.lock"))
    else {
        return false;
    };
    let Ok(pid) = contents.trim().parse::<u32>() else {
        return false;
    };
    process_is_running(pid)
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
    let Some(mut child) = model.daemon.borrow_mut().take() else {
        return;
    };
    let _ = child.kill();
    let _ = child.wait();
    model.ui.notice.set_text("Sync stopped");
    refresh(model);
}

pub(crate) fn run_idrive_json<const N: usize>(args: [&str; N]) -> Result<Value, String> {
    let output = Command::new(idrive_path())
        .args(args)
        .output()
        .map_err(|error| format!("idrive failed to start: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    serde_json::from_slice(&output.stdout).map_err(|error| format!("Invalid status JSON: {error}"))
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
