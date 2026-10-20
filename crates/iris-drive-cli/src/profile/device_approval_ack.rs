use super::{
    APP_KEY_APPROVAL_APPLIED_ACK_APP_TOPIC, AppConfig, ConfigMutationLock, Context,
    FsFipsBlockSync, JsonUtil, Path, Result, config_path_in, key_path_in, normalize_pubkey,
    pubkey_npub, unix_now_millis, unix_now_seconds,
};

pub(super) async fn handle_device_approval_applied_ack_app_message(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
) -> Result<bool> {
    let received_at_ms = unix_now_millis();
    let event_json =
        std::str::from_utf8(&message.data).context("device approval applied ACK is not UTF-8")?;
    let event = nostr_sdk::Event::from_json(event_json)
        .context("parsing device approval applied ACK event")?;
    if !iris_drive_core::relay_sync::is_device_approval_applied_ack_event(&event) {
        return Err(anyhow::anyhow!(
            "FIPS device approval applied ACK has the wrong event type"
        ));
    }
    let signer = event.pubkey.to_hex();
    if normalize_pubkey(&message.peer_id).ok().as_deref() != Some(signer.as_str()) {
        return Err(anyhow::anyhow!(
            "FIPS device approval applied ACK peer does not match signer"
        ));
    }
    let lock_started_at = std::time::Instant::now();
    let _config_lock = ConfigMutationLock::acquire(config_dir).await?;
    let config_lock_wait_ms = lock_started_at.elapsed().as_millis();
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let pending_before = config
        .profile
        .as_ref()
        .map_or(0, |state| state.pending_device_approval_receipts.len());
    let changed = match config.profile.as_mut() {
        Some(state) => {
            iris_drive_core::app_key_link_transport::apply_device_approval_applied_ack_event(
                state, &event,
            )?
        }
        None => false,
    };
    if changed {
        config.save(config_path_in(config_dir))?;
    }
    let pending_after = config
        .profile
        .as_ref()
        .map_or(0, |state| state.pending_device_approval_receipts.len());
    println!(
        "{}",
        serde_json::json!({
            "event": "fips_device_approval_applied_ack",
            "event_id": event.id.to_hex(),
            "event_created_at": event.created_at.as_secs(),
            "received_at_ms": received_at_ms,
            "config_lock_wait_ms": config_lock_wait_ms,
            "persisted_at_ms": unix_now_millis(),
            "pending_before": pending_before,
            "pending_after": pending_after,
            "outcome": if changed { "applied" } else { "ignored" },
        })
    );
    Ok(true)
}

pub(super) async fn handle_device_approval_receipt_app_message(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<bool> {
    let event_json =
        std::str::from_utf8(&message.data).context("device approval receipt is not UTF-8")?;
    let event =
        nostr_sdk::Event::from_json(event_json).context("parsing device approval receipt event")?;
    if !iris_drive_core::relay_sync::is_device_approval_receipt_event(&event) {
        return Err(anyhow::anyhow!(
            "FIPS device approval receipt has the wrong event type"
        ));
    }
    let config_lock = ConfigMutationLock::acquire(config_dir).await?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let outcome = iris_drive_core::relay_sync::apply_remote_device_approval_receipt_event(
        &mut config,
        &event,
    )?;
    if matches!(
        outcome,
        iris_drive_core::relay_sync::NostrIdentityRosterOpApply::NotOurProfile
            | iris_drive_core::relay_sync::NostrIdentityRosterOpApply::ApprovalReceiptRequired
    ) {
        return Ok(true);
    }
    config.save(config_path_in(config_dir))?;
    drop(config_lock);
    if let Some(sync) = fips_blocks {
        sync.refresh_authorized_peers_from_config_dir(config_dir)
            .await;
    }
    send_device_approval_applied_ack_if_ready(config_dir, &config, fips_blocks).await?;
    Ok(true)
}

pub(super) async fn send_device_approval_applied_ack_if_ready(
    config_dir: &Path,
    config: &AppConfig,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<bool> {
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
    let device = iris_drive_core::identity::AppKey::load(key_path_in(config_dir))
        .context("loading app key for approval ACK")?;
    let acknowledgements =
        iris_drive_core::app_key_link_transport::device_approval_applied_ack_events(
            state,
            device.keys(),
            unix_now_seconds(),
        )?;
    let all_receipts_ready = acknowledgements.len() == receipt_count;
    let Some(sync) = fips_blocks else {
        return Ok(false);
    };
    for acknowledgement in acknowledgements {
        let parsed = iris_drive_core::nostr_identity::parse_nostr_identity_device_approval_applied_ack_event(&acknowledgement)?;
        sync.send_app_message(
            &pubkey_npub(&parsed.approved_by_pubkey),
            APP_KEY_APPROVAL_APPLIED_ACK_APP_TOPIC,
            acknowledgement.as_json().into_bytes(),
        )
        .await?;
    }
    Ok(all_receipts_ready)
}
