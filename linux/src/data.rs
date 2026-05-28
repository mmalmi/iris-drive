#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn drive_mount_text(json: &Value) -> String {
    mounted_dir(json)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "Not mounted".to_string())
}

pub(crate) fn mounted_dir(json: &Value) -> Option<PathBuf> {
    json.get("daemon")
        .and_then(|daemon| daemon.get("mount"))
        .and_then(|mount| mount.get("mountpoint"))
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
}

pub(crate) fn drive_name(json: &Value) -> String {
    json.get("drives")
        .and_then(Value::as_array)
        .and_then(|drives| {
            drives
                .iter()
                .find(|drive| drive.get("drive_id").and_then(Value::as_str) == Some("main"))
                .or_else(|| drives.first())
        })
        .and_then(|drive| find_string(drive, &["display_name", "name"]))
        .unwrap_or("My Drive")
        .to_string()
}

pub(crate) fn snapshot_value(json: &Value) -> String {
    snapshot_link(json)
        .map(short_text)
        .unwrap_or_else(|| "-".to_string())
}

pub(crate) fn snapshot_link(json: &Value) -> Option<&str> {
    let hashtree = json.get("hashtree").unwrap_or(&Value::Null);
    find_string(hashtree, &["snapshot_url", "permalink_url"])
}

pub(crate) fn account_json(json: &Value) -> &Value {
    json.get("account").unwrap_or(&Value::Null)
}

pub(crate) fn is_awaiting_link_approval(json: &Value) -> bool {
    let account = account_json(json);
    find_string(account, &["authorization_state"]) == Some("awaiting_approval")
        && find_bool(account, &["has_owner_signing_authority"]) == Some(false)
}

pub(crate) fn file_count_value(json: &Value) -> String {
    let hashtree = json.get("hashtree").unwrap_or(&Value::Null);
    find_number(hashtree, &["file_count", "top_level_entries"])
        .map(|value| value.to_string())
        .unwrap_or_else(|| "0".to_string())
}

pub(crate) fn storage_value(json: &Value) -> String {
    let hashtree = json.get("hashtree").unwrap_or(&Value::Null);
    find_number(hashtree, &["local_block_bytes"])
        .map(format_bytes)
        .unwrap_or_else(|| "0 B".to_string())
}

pub(crate) fn device_count_value(json: &Value) -> String {
    let network = json.get("network").unwrap_or(&Value::Null);
    let published = find_number(network, &["published_device_roots"]).unwrap_or(0);
    let authorized = find_number(network, &["authorized_device_count"]).unwrap_or(0);
    format!("{published}/{authorized}")
}

pub(crate) fn sidebar_online_value(json: &Value) -> String {
    let fips = json
        .get("network")
        .and_then(|network| network.get("fips"))
        .unwrap_or(&Value::Null);
    let online = find_number(fips, &["roster_connected_peer_count"]).unwrap_or(0);
    let expected = find_number(fips, &["roster_peer_count"]).unwrap_or(0);
    format!("{online}/{expected} online")
}

pub(crate) fn local_nhash_resolver_enabled(json: &Value) -> bool {
    json.get("settings")
        .and_then(|settings| find_bool(settings, &["local_nhash_resolver_enabled"]))
        .or_else(|| {
            json.get("hashtree")
                .and_then(|hashtree| hashtree.get("local_gateway"))
                .and_then(|gateway| find_bool(gateway, &["enabled"]))
        })
        .unwrap_or(true)
}

pub(crate) fn find_string<'a>(json: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| json.get(*key).and_then(Value::as_str))
}

pub(crate) fn find_number(json: &Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| json.get(*key).and_then(Value::as_u64))
}

pub(crate) fn find_bool(json: &Value, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .find_map(|key| json.get(*key).and_then(Value::as_bool))
}

pub(crate) fn short_value(value: Option<&str>) -> String {
    let Some(value) = value else {
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
