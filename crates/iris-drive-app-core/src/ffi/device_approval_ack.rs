use super::*;

pub(super) fn handle_native_device_approval_applied_ack(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
) -> Result<bool, String> {
    let event_json = std::str::from_utf8(&message.data)
        .map_err(|error| format!("device approval applied ACK is not UTF-8: {error}"))?;
    let event = Event::from_json(event_json)
        .map_err(|error| format!("parsing device approval applied ACK event: {error}"))?;
    let signer = event.pubkey.to_hex();
    if normalize_pubkey(&message.peer_id).ok().as_deref() != Some(signer.as_str()) {
        return Err("device approval applied ACK peer does not match signer".to_string());
    }
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    let changed = match config.profile.as_mut() {
        Some(state) => {
            iris_drive_core::app_key_link_transport::apply_device_approval_applied_ack_event(
                state, &event,
            )
            .map_err(|error| format!("applying device approval applied ACK: {error}"))?
        }
        None => false,
    };
    if changed {
        config
            .save(config_path_in(config_dir))
            .map_err(|error| format!("saving device approval applied ACK: {error}"))?;
    }
    Ok(true)
}

pub(super) async fn send_native_device_approval_applied_ack(
    config_dir: &Path,
    relay_client: &nostr_sdk::Client,
    sync: &iris_drive_core::FsFipsBlockSync,
    approval_event: &Event,
) -> Result<(), String> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading applied approval state: {error}"))?;
    let state = config
        .profile
        .as_ref()
        .ok_or_else(|| "profile disappeared after applying approval".to_string())?;
    let device = iris_drive_core::AppKey::load(key_path_in(config_dir))
        .map_err(|error| format!("loading app key for approval ACK: {error}"))?;
    let ack = iris_drive_core::app_key_link_transport::device_approval_applied_ack_event(
        state,
        device.keys(),
        approval_event,
        unix_now_seconds(),
    )
    .map_err(|error| format!("building device approval applied ACK: {error}"))?;
    let parsed =
        iris_drive_core::nostr_identity::parse_nostr_identity_device_approval_applied_ack_event(
            &ack,
        )
        .map_err(|error| format!("parsing device approval applied ACK: {error}"))?;
    let fips_result = sync
        .send_app_message(
            &pubkey_npub(&parsed.approved_by_pubkey),
            APP_KEY_APPROVAL_APPLIED_ACK_APP_TOPIC,
            ack.as_json().into_bytes(),
        )
        .await;
    let relay_result = tokio::time::timeout(
        std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
        iris_drive_core::relay_sync::publish_device_approval_applied_ack(relay_client, &ack),
    )
    .await;
    if fips_result.is_err() && !matches!(relay_result, Ok(Ok(_))) {
        return Err(format!(
            "sending device approval applied ACK failed over FIPS ({}) and relays",
            fips_result.unwrap_err()
        ));
    }
    if let Err(error) = fips_result {
        tracing::warn!(error = %error, "sending device approval applied ACK over FIPS failed");
    }
    match relay_result {
        Ok(Ok(_)) => {}
        Ok(Err(error)) => {
            tracing::warn!(error = %error, "publishing device approval applied ACK failed")
        }
        Err(_) => tracing::warn!("publishing device approval applied ACK timed out"),
    }
    Ok(())
}
