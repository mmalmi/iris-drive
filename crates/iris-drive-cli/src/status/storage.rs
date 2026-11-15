#[allow(clippy::wildcard_imports)]
use super::*;

use iris_drive_core::device_storage::rows::device_storage_view;

/// Print, as JSON, each of the profile's devices as the Storage page shows
/// them: how much of the Drive it holds, its room, and whether it is in sync.
/// Read from what this device keeps; relays are not asked.
pub(crate) fn cmd_storage(config_dir: &Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))
        .with_context(|| format!("reading config at {}", config_path_in(config_dir).display()))?;
    let Some(state) = config.profile.as_ref() else {
        println!("{}", json!({"summary": "", "devices": []}));
        return Ok(());
    };
    let daemon_status = load_daemon_status(config_dir);
    let peers = peer_statuses(config_dir, &config, daemon_status.as_ref());
    let peer = |pubkey: &str| {
        peers
            .iter()
            .find(|peer| peer.get("app_key_pubkey").and_then(Value::as_str) == Some(pubkey))
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs());
    let view = device_storage_view(
        config_dir,
        &config,
        state,
        |pubkey| {
            peer(pubkey)
                .and_then(|peer| peer.get("display_label"))
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        },
        |pubkey| {
            peer(pubkey)
                .and_then(|peer| peer.get("fips_direct_online"))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        },
        now,
    );
    println!("{}", serde_json::to_string_pretty(&view)?);
    Ok(())
}
