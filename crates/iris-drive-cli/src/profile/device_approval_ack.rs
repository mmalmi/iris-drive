use super::{
    APP_KEY_APPROVAL_APPLIED_ACK_APP_TOPIC, AppConfig, ConfigMutationLock, Context,
    FsFipsBlockSync, JsonUtil, Path, Result, config_path_in, key_path_in, normalize_pubkey,
    pubkey_npub, unix_now_seconds,
};

pub(super) async fn handle_device_approval_applied_ack_app_message(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
) -> Result<bool> {
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
    let _config_lock = ConfigMutationLock::acquire(config_dir).await?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
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
    let device = iris_drive_core::identity::AppKey::load(key_path_in(config_dir))
        .context("loading app key for approval ACK")?;
    let ack = iris_drive_core::app_key_link_transport::device_approval_applied_ack_event(
        config
            .profile
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("profile disappeared"))?,
        device.keys(),
        &event,
        unix_now_seconds(),
    )?;
    if let Some(sync) = fips_blocks {
        let parsed = iris_drive_core::nostr_identity::parse_nostr_identity_device_approval_applied_ack_event(&ack)?;
        sync.send_app_message(
            &pubkey_npub(&parsed.approved_by_pubkey),
            APP_KEY_APPROVAL_APPLIED_ACK_APP_TOPIC,
            ack.as_json().into_bytes(),
        )
        .await?;
    }
    Ok(true)
}
