use super::{
    APP_KEY_APPROVAL_APPLIED_ACK_APP_TOPIC, AppConfig, Event, JsonUtil, Mutex,
    NATIVE_SYNC_RELAY_TIMEOUT_SECS, Path, config_path_in, key_path_in, normalize_pubkey,
    pubkey_npub, unix_now_seconds,
};

pub(super) fn native_device_approval_ack_is_ready(
    config_dir: &Path,
    config: &AppConfig,
) -> Result<bool, String> {
    let Some(state) = config.profile.as_ref() else {
        return Ok(false);
    };
    let Some(pending) = state.outbound_app_key_link_request.as_ref() else {
        return Ok(false);
    };
    let receipt_count = pending.approval_receipt_event.len();
    if receipt_count == 0 {
        return Ok(false);
    }
    let device = iris_drive_core::AppKey::load(key_path_in(config_dir))
        .map_err(|error| format!("loading app key for approval readiness: {error}"))?;
    let acknowledgements =
        iris_drive_core::app_key_link_transport::device_approval_applied_ack_events(
            state,
            device.keys(),
            unix_now_seconds(),
        )
        .map_err(|error| format!("checking device approval readiness: {error}"))?;
    Ok(acknowledgements.len() == receipt_count)
}

pub(super) async fn handle_native_device_approval_receipt(
    config_dir: &Path,
    config_mutation: &Mutex<()>,
    relay_client: &nostr_sdk::Client,
    sync: &iris_drive_core::FsFipsBlockSync,
    message: &iris_drive_core::FipsAppMessage,
) -> Result<bool, String> {
    let event_json = std::str::from_utf8(&message.data)
        .map_err(|error| format!("device approval receipt is not UTF-8: {error}"))?;
    let event = Event::from_json(event_json)
        .map_err(|error| format!("parsing device approval receipt event: {error}"))?;
    if !iris_drive_core::relay_sync::is_device_approval_receipt_event(&event) {
        return Err("FIPS device approval receipt has the wrong event type".to_string());
    }
    let config_guard = config_mutation
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    let outcome = iris_drive_core::relay_sync::apply_remote_device_approval_receipt_event(
        &mut config,
        &event,
    )
    .map_err(|error| format!("applying FIPS device approval receipt: {error}"))?;
    if matches!(
        outcome,
        iris_drive_core::relay_sync::NostrIdentityRosterOpApply::NotOurProfile
            | iris_drive_core::relay_sync::NostrIdentityRosterOpApply::ApprovalReceiptRequired
    ) {
        return Ok(true);
    }
    config
        .save(config_path_in(config_dir))
        .map_err(|error| format!("saving device approval receipt: {error}"))?;
    drop(config_guard);
    sync.refresh_authorized_peers_from_config_dir(config_dir)
        .await;
    send_native_device_approval_applied_ack(config_dir, relay_client, sync).await?;
    Ok(true)
}

pub(super) fn handle_native_device_approval_applied_ack(
    config_dir: &Path,
    config_mutation: &Mutex<()>,
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
    let _config_mutation = config_mutation
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
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
) -> Result<bool, String> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading applied approval state: {error}"))?;
    let state = config
        .profile
        .as_ref()
        .ok_or_else(|| "profile disappeared after applying approval".to_string())?;
    let Some(pending) = state.outbound_app_key_link_request.as_ref() else {
        return Ok(false);
    };
    let receipt_count = pending.approval_receipt_event.len();
    if receipt_count == 0 {
        return Ok(false);
    }
    let device = iris_drive_core::AppKey::load(key_path_in(config_dir))
        .map_err(|error| format!("loading app key for approval ACK: {error}"))?;
    let acknowledgements =
        iris_drive_core::app_key_link_transport::device_approval_applied_ack_events(
            state,
            device.keys(),
            unix_now_seconds(),
        )
        .map_err(|error| format!("building device approval applied ACKs: {error}"))?;
    let all_receipts_ready = acknowledgements.len() == receipt_count;
    for acknowledgement in acknowledgements {
        send_native_device_approval_ack(relay_client, sync, acknowledgement).await?;
    }
    Ok(all_receipts_ready)
}

async fn send_native_device_approval_ack(
    relay_client: &nostr_sdk::Client,
    sync: &iris_drive_core::FsFipsBlockSync,
    ack: Event,
) -> Result<(), String> {
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
    if fips_result.is_ok() {
        let relay_client = relay_client.clone();
        tokio::spawn(async move {
            match tokio::time::timeout(
                std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
                iris_drive_core::relay_sync::publish_device_approval_applied_ack(
                    &relay_client,
                    &ack,
                ),
            )
            .await
            {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => tracing::warn!(
                    error = %error,
                    "publishing redundant device approval applied ACK failed"
                ),
                Err(_) => {
                    tracing::warn!("publishing redundant device approval applied ACK timed out");
                }
            }
        });
        return Ok(());
    }

    let fips_error = fips_result.unwrap_err();
    tracing::warn!(
        error = %fips_error,
        "sending device approval applied ACK over FIPS failed"
    );
    match tokio::time::timeout(
        std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
        iris_drive_core::relay_sync::publish_device_approval_applied_ack(relay_client, &ack),
    )
    .await
    {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => Err(format!(
            "sending device approval applied ACK failed over FIPS ({fips_error}) and relays ({error})"
        )),
        Err(_) => Err(format!(
            "sending device approval applied ACK failed over FIPS ({fips_error}) and relays timed out"
        )),
    }
}
