#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn drive_mount_text(state: &NativeAppState) -> String {
    mounted_dir(state)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "Not mounted".to_string())
}

pub(crate) fn mounted_dir(state: &NativeAppState) -> Option<PathBuf> {
    (state.ui.setup_complete && !state.ui.revoked).then(default_drive_dir)
}

pub(crate) fn drive_name(state: &NativeAppState) -> String {
    state
        .ui
        .roots
        .first()
        .map(|root| root.name.as_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("My Drive")
        .to_string()
}

pub(crate) fn snapshot_value(state: &NativeAppState) -> String {
    snapshot_link(state)
        .map(short_text)
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn snapshot_link(state: &NativeAppState) -> Option<&str> {
    (!state.ui.snapshot_link.is_empty()).then_some(state.ui.snapshot_link.as_str())
}

pub(crate) fn profile(state: &NativeAppState) -> Option<&iris_drive_app_core::state::UiProfile> {
    state.ui.profile.as_ref()
}

pub(crate) fn setup_label_value(state: &NativeAppState) -> &str {
    if state.ui.setup_label.is_empty() {
        "-"
    } else {
        &state.ui.setup_label
    }
}

pub(crate) fn is_awaiting_link_approval(state: &NativeAppState) -> bool {
    state.ui.awaiting_approval
}

pub(crate) fn is_revoked(state: &NativeAppState) -> bool {
    state.ui.revoked
}

pub(crate) fn file_count_value(state: &NativeAppState) -> String {
    state.ui.file_count.to_string()
}

pub(crate) fn storage_value(state: &NativeAppState) -> String {
    format_bytes(state.ui.visible_file_bytes)
}

pub(crate) fn app_key_count_value(state: &NativeAppState) -> String {
    format!(
        "{}/{}",
        state.ui.online_app_key_count, state.ui.authorized_app_key_count
    )
}

pub(crate) fn sidebar_online_value(state: &NativeAppState) -> String {
    format!("{} online", app_key_count_value(state))
}

pub(crate) fn primary_status_label_value(state: &NativeAppState) -> &str {
    if state.ui.primary_status_label.is_empty() {
        "Ready"
    } else {
        &state.ui.primary_status_label
    }
}

pub(crate) fn local_nhash_resolver_enabled(_state: &NativeAppState) -> bool {
    true
}

pub(crate) fn short_value(value: Option<&str>) -> String {
    let Some(value) = value.filter(|value| !value.is_empty()) else {
        return "-".to_string();
    };
    short_text(value)
}

pub(crate) fn short_text(value: &str) -> String {
    if value.chars().count() <= 32 {
        return value.to_string();
    }
    let start = value.chars().take(14).collect::<String>();
    let end = value
        .chars()
        .rev()
        .take(10)
        .collect::<String>()
        .chars()
        .rev()
        .collect::<String>();
    format!("{start}...{end}")
}

pub(crate) fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
