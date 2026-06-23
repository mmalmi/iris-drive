#[allow(clippy::wildcard_imports)]
use super::*;
use iris_drive_core::fips_status::normalize_fips_status_value;

pub(crate) fn fips_network_diagnostics(config: &AppConfig, daemon_status: Option<&Value>) -> Value {
    let running = daemon_status
        .and_then(|status| status.get("running"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fresh = daemon_status
        .and_then(|status| status.get("fresh"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fips_status = daemon_status
        .and_then(|status| status.get("fips_block_sync"))
        .filter(|value| value.is_object());
    let error = daemon_status
        .and_then(|status| status.get("fips_block_sync_error"))
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or(Value::Null);
    normalize_fips_status_value(
        fips_status,
        running,
        fresh,
        error,
        &configured_fips_authorized_peer_npubs(config),
    )
}

fn configured_fips_authorized_peer_npubs(config: &AppConfig) -> Vec<String> {
    let Some(account) = config.profile.as_ref() else {
        return Vec::new();
    };
    let Some(snapshot) = account.current_app_keys_projection() else {
        return Vec::new();
    };

    snapshot
        .app_actors
        .iter()
        .filter(|device| device.pubkey != account.app_key_pubkey)
        .map(|device| pubkey_npub(&device.pubkey))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}
