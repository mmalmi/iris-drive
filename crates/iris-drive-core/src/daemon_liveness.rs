use std::path::{Path, PathBuf};

pub const DAEMON_LOCK_FILE_NAME: &str = "daemon.lock";
pub const DAEMON_STATUS_FILE_NAME: &str = "daemon-status.json";
const DAEMON_STATUS_FRESH_SECS: u64 = 15;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaemonLiveness {
    pub pid: Option<u32>,
    pub running: bool,
}

#[must_use]
pub fn daemon_lock_path(config_dir: &Path) -> PathBuf {
    config_dir.join(DAEMON_LOCK_FILE_NAME)
}

#[must_use]
pub fn daemon_lock_pid(config_dir: &Path) -> Option<u32> {
    std::fs::read_to_string(daemon_lock_path(config_dir))
        .ok()
        .and_then(|contents| contents.trim().parse::<u32>().ok())
}

#[must_use]
pub fn daemon_liveness(config_dir: &Path) -> DaemonLiveness {
    let pid = daemon_lock_pid(config_dir);
    DaemonLiveness {
        pid,
        running: pid.is_some_and(process_is_running),
    }
}

pub fn ensure_daemon_available_for_provider_mutation(
    config_dir: &Path,
) -> anyhow::Result<DaemonLiveness> {
    let liveness = daemon_liveness(config_dir);
    if liveness.running {
        return Ok(liveness);
    }
    if let Some(status_liveness) = fresh_daemon_status_liveness(config_dir) {
        return Ok(status_liveness);
    }
    anyhow::bail!(
        "Iris Drive daemon is unavailable; provider changes cannot be saved while sync is offline. Open Iris Drive or start the background service and retry."
    );
}

fn daemon_status_path(config_dir: &Path) -> PathBuf {
    config_dir.join(DAEMON_STATUS_FILE_NAME)
}

fn fresh_daemon_status_liveness(config_dir: &Path) -> Option<DaemonLiveness> {
    let data = std::fs::read(daemon_status_path(config_dir)).ok()?;
    let status: serde_json::Value = serde_json::from_slice(&data).ok()?;
    let running = status
        .get("running")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !running {
        return None;
    }
    let fresh_flag = status
        .get("fresh")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(true);
    let updated_at = status
        .get("updated_at")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default();
    if !fresh_flag || unix_now_seconds().saturating_sub(updated_at) > DAEMON_STATUS_FRESH_SECS {
        return None;
    }
    let pid = status
        .get("pid")
        .and_then(serde_json::Value::as_u64)
        .and_then(|pid| u32::try_from(pid).ok())
        .or_else(|| daemon_lock_pid(config_dir));
    Some(DaemonLiveness { pid, running: true })
}

fn unix_now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(unix)]
#[must_use]
pub fn process_is_running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    std::process::Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(windows)]
#[must_use]
pub fn process_is_running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    let filter = format!("PID eq {pid}");
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().any(|line| {
        let mut fields = line.split(',');
        let _image = fields.next();
        fields
            .next()
            .map(|value| value.trim_matches('"').trim() == pid.to_string())
            .unwrap_or(false)
    })
}

#[cfg(not(any(unix, windows)))]
#[must_use]
pub fn process_is_running(pid: u32) -> bool {
    pid == std::process::id()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daemon_liveness_distinguishes_current_process_from_stale_pid() {
        let dir = tempfile::tempdir().unwrap();

        std::fs::write(
            daemon_lock_path(dir.path()),
            format!("{}\n", std::process::id()),
        )
        .unwrap();
        let current = daemon_liveness(dir.path());
        assert_eq!(current.pid, Some(std::process::id()));
        assert!(current.running);

        std::fs::write(daemon_lock_path(dir.path()), "99999999\n").unwrap();
        let stale = daemon_liveness(dir.path());
        assert_eq!(stale.pid, Some(99_999_999));
        assert!(!stale.running);
    }

    #[test]
    fn provider_mutation_error_names_offline_daemon() {
        let dir = tempfile::tempdir().unwrap();

        let error = ensure_daemon_available_for_provider_mutation(dir.path()).unwrap_err();

        assert!(error.to_string().contains("daemon is unavailable"));
    }

    #[test]
    fn provider_mutation_accepts_fresh_daemon_status_when_pid_probe_fails() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(daemon_lock_path(dir.path()), "99999999\n").unwrap();
        std::fs::write(
            dir.path().join("daemon-status.json"),
            format!(
                r#"{{"pid":99999999,"running":true,"fresh":true,"updated_at":{}}}"#,
                super::unix_now_seconds()
            ),
        )
        .unwrap();

        let liveness = ensure_daemon_available_for_provider_mutation(dir.path()).unwrap();

        assert_eq!(liveness.pid, Some(99_999_999));
        assert!(liveness.running);
    }
}
