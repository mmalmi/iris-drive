#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
use std::path::Path;

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
use iris_drive_core::fips_status::normalize_fips_status_value;
#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
use iris_drive_core::{AppConfig, AppKeyAuthorizationState};
#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
use serde_json::Value;
#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
use serde_json::json;

use super::NativeAppRuntime;

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) const NATIVE_FIPS_STATUS_STABLE_WRITE_MIN_SECS: u64 = 15;

impl NativeAppRuntime {
    #[allow(clippy::unused_self)]
    pub(super) fn reconcile_app_key_link_exchange(&mut self) {
        #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
        {
            let Ok(config) = self.load_config() else {
                self.stop_app_key_link_exchange();
                return;
            };
            if !native_app_key_link_exchange_should_run(&config, self.state.ui.sync.running) {
                self.stop_app_key_link_exchange();
                write_native_fips_paused(Path::new(&self.data_dir));
                return;
            }
            self.app_key_link_exchange_stop
                .store(false, std::sync::atomic::Ordering::Release);
            if self
                .app_key_link_exchange_running
                .swap(true, std::sync::atomic::Ordering::AcqRel)
            {
                return;
            }

            let data_dir = self.data_dir.clone();
            let running = self.app_key_link_exchange_running.clone();
            let stop = self.app_key_link_exchange_stop.clone();
            std::thread::spawn(move || {
                if let Err(error) = super::run_app_key_link_exchange(&data_dir, stop) {
                    tracing::warn!(error = %error, "native app-key-link FIPS exchange stopped");
                }
                running.store(false, std::sync::atomic::Ordering::Release);
            });
        }
    }

    #[allow(clippy::unused_self)]
    pub(super) fn stop_app_key_link_exchange(&mut self) {
        #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
        {
            self.app_key_link_exchange_stop
                .store(true, std::sync::atomic::Ordering::Release);
        }
    }
}

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) fn native_app_key_link_exchange_should_run(
    config: &AppConfig,
    sync_running: bool,
) -> bool {
    match config
        .profile
        .as_ref()
        .map(|profile| profile.authorization_state)
    {
        Some(AppKeyAuthorizationState::Authorized) => sync_running,
        Some(AppKeyAuthorizationState::AwaitingApproval) => true,
        _ => false,
    }
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
pub(super) fn write_native_fips_error(config_dir: &Path, error: &str) {
    let raw = json!({
        "running": false,
        "updated_at": super::unix_now_seconds(),
        "online_devices": [],
        "online_peers": [],
        "direct_devices": [],
        "direct_peers": [],
        "connected_peers": [],
        "mesh_devices": [],
        "mesh_peers": [],
        "peer_statuses": [],
        "error": error,
    });
    let value = normalize_fips_status_value(
        Some(&raw),
        false,
        false,
        Value::String(error.to_owned()),
        &[],
    );
    if let Err(write_error) = write_native_fips_status_value(config_dir, &value) {
        tracing::warn!(error = %write_error, "writing native FIPS error failed");
    }
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
pub(super) fn write_native_fips_paused(config_dir: &Path) {
    let raw = json!({
        "running": false,
        "updated_at": super::unix_now_seconds(),
        "online_devices": [],
        "online_peers": [],
        "direct_devices": [],
        "direct_peers": [],
        "connected_peers": [],
        "mesh_devices": [],
        "mesh_peers": [],
        "peer_statuses": [],
        "error": Value::Null,
    });
    let value = normalize_fips_status_value(Some(&raw), false, false, Value::Null, &[]);
    if let Err(error) = write_native_fips_status_value(config_dir, &value) {
        tracing::warn!(error = %error, "writing native FIPS paused status failed");
    }
}

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) fn write_native_fips_status_value(
    config_dir: &Path,
    value: &serde_json::Value,
) -> Result<(), String> {
    let path = super::native_fips_status_path(config_dir);
    if !native_fips_status_write_is_due(&path, value, super::unix_now_seconds()) {
        return Ok(());
    }
    let data =
        serde_json::to_vec(value).map_err(|error| format!("encoding FIPS status: {error}"))?;
    std::fs::write(&path, data).map_err(|error| format!("writing {}: {error}", path.display()))
}

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) fn native_fips_status_write_is_due(path: &Path, value: &Value, now: u64) -> bool {
    let Ok(existing_data) = std::fs::read(path) else {
        return true;
    };
    let Ok(existing_value) = serde_json::from_slice::<Value>(&existing_data) else {
        return true;
    };
    if existing_value == *value {
        return false;
    }
    if native_fips_status_stable_value(&existing_value) != native_fips_status_stable_value(value) {
        return true;
    }
    let existing_updated_at = existing_value
        .get("updated_at")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    existing_updated_at == 0
        || now.saturating_sub(existing_updated_at) >= NATIVE_FIPS_STATUS_STABLE_WRITE_MIN_SECS
}

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
fn native_fips_status_stable_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| !native_fips_status_volatile_key(key))
                .map(|(key, value)| (key.clone(), native_fips_status_stable_value(value)))
                .collect(),
        ),
        Value::Array(values) => {
            Value::Array(values.iter().map(native_fips_status_stable_value).collect())
        }
        value => value.clone(),
    }
}

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
fn native_fips_status_volatile_key(key: &str) -> bool {
    matches!(
        key,
        "bytes_recv"
            | "bytes_sent"
            | "connection_label"
            | "packets_recv"
            | "packets_sent"
            | "srtt_ms"
            | "updated_at"
    )
}
