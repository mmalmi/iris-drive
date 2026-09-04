use super::{
    APP_KEY_APPROVAL_APPLIED_ACK_APP_TOPIC, AppConfig, AppKeyLinkAuditEvent, Event, Mutex,
    NATIVE_SYNC_RELAY_TIMEOUT_SECS, Path, append_app_key_link_audit, config_path_in, key_path_in,
    normalize_pubkey, pubkey_npub, unix_now_seconds,
};
use nostr_sdk::JsonUtil;

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
    let mut received = AppKeyLinkAuditEvent::new("approval_receipt_received");
    received.source = Some("fips".to_owned());
    received.event_id = Some(event.id.to_hex());
    let _ = append_app_key_link_audit(config_dir, received);
    let disk_mutation = iris_drive_core::config_lock::ConfigMutationLock::acquire(config_dir)
        .await
        .map_err(|error| format!("locking device approval receipt: {error}"))?;
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
    let ready = super::native_device_approval_ack_is_ready(config_dir, &config)?;
    let mut persisted = AppKeyLinkAuditEvent::new("approval_receipt_persisted");
    persisted.source = Some("fips".to_owned());
    persisted.event_id = Some(event.id.to_hex());
    persisted.outcome = Some(format!("{outcome:?}"));
    persisted.authorization_state = config
        .profile
        .as_ref()
        .map(|state| format!("{:?}", state.authorization_state));
    persisted.receipt_count = config
        .profile
        .as_ref()
        .and_then(|state| state.outbound_app_key_link_request.as_ref())
        .map(|pending| pending.approval_receipt_event.len());
    persisted.ready = Some(ready);
    let _ = append_app_key_link_audit(config_dir, persisted);
    drop(config_guard);
    drop(disk_mutation);
    sync.refresh_authorized_peers_from_config_dir(config_dir)
        .await;
    send_native_device_approval_applied_ack(config_dir, relay_client, sync).await?;
    Ok(true)
}

pub(super) async fn handle_native_device_approval_applied_ack(
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
    let _disk_mutation = iris_drive_core::config_lock::ConfigMutationLock::acquire(config_dir)
        .await
        .map_err(|error| format!("locking device approval applied ACK: {error}"))?;
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
        match iris_drive_core::app_key_link_transport::device_approval_applied_ack_events(
            state,
            device.keys(),
            unix_now_seconds(),
        ) {
            Ok(acknowledgements) => acknowledgements,
            Err(error) => {
                let mut audit = AppKeyLinkAuditEvent::new("approval_ack_build");
                audit.receipt_count = Some(receipt_count);
                audit.success = Some(false);
                audit.error_class = Some("build_failed".to_owned());
                let _ = append_app_key_link_audit(config_dir, audit);
                return Err(format!("building device approval applied ACKs: {error}"));
            }
        };
    let all_receipts_ready = acknowledgements.len() == receipt_count;
    let mut built = AppKeyLinkAuditEvent::new("approval_ack_build");
    built.receipt_count = Some(receipt_count);
    built.ack_count = Some(acknowledgements.len());
    built.ready = Some(all_receipts_ready);
    built.success = Some(true);
    let _ = append_app_key_link_audit(config_dir, built);
    for acknowledgement in acknowledgements {
        send_native_device_approval_ack(config_dir, relay_client, sync, acknowledgement).await?;
    }
    Ok(all_receipts_ready)
}

async fn send_native_device_approval_ack(
    config_dir: &Path,
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
    let mut fips_audit = AppKeyLinkAuditEvent::new("approval_ack_publish");
    fips_audit.event_id = Some(ack.id.to_hex());
    fips_audit.transport = Some("fips".to_owned());
    fips_audit.success = Some(fips_result.is_ok());
    if fips_result.is_err() {
        fips_audit.error_class = Some("send_failed".to_owned());
    }
    let _ = append_app_key_link_audit(config_dir, fips_audit);
    if fips_result.is_ok() {
        let relay_client = relay_client.clone();
        let audit_dir = config_dir.to_path_buf();
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
                Ok(Ok(_)) => {
                    let mut audit = AppKeyLinkAuditEvent::new("approval_ack_publish");
                    audit.event_id = Some(ack.id.to_hex());
                    audit.transport = Some("relay_redundant".to_owned());
                    audit.success = Some(true);
                    let _ = append_app_key_link_audit(&audit_dir, audit);
                }
                Ok(Err(error)) => {
                    let mut audit = AppKeyLinkAuditEvent::new("approval_ack_publish");
                    audit.event_id = Some(ack.id.to_hex());
                    audit.transport = Some("relay_redundant".to_owned());
                    audit.success = Some(false);
                    audit.error_class = Some("publish_failed".to_owned());
                    let _ = append_app_key_link_audit(&audit_dir, audit);
                    tracing::warn!(error = %error, "publishing redundant device approval applied ACK failed");
                }
                Err(_) => {
                    let mut audit = AppKeyLinkAuditEvent::new("approval_ack_publish");
                    audit.event_id = Some(ack.id.to_hex());
                    audit.transport = Some("relay_redundant".to_owned());
                    audit.success = Some(false);
                    audit.error_class = Some("timeout".to_owned());
                    let _ = append_app_key_link_audit(&audit_dir, audit);
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
    let relay_result = tokio::time::timeout(
        std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
        iris_drive_core::relay_sync::publish_device_approval_applied_ack(relay_client, &ack),
    )
    .await;
    let mut relay_audit = AppKeyLinkAuditEvent::new("approval_ack_publish");
    relay_audit.event_id = Some(ack.id.to_hex());
    relay_audit.transport = Some("relay_fallback".to_owned());
    relay_audit.success = Some(matches!(relay_result, Ok(Ok(_))));
    relay_audit.error_class = match &relay_result {
        Ok(Ok(_)) => None,
        Ok(Err(_)) => Some("publish_failed".to_owned()),
        Err(_) => Some("timeout".to_owned()),
    };
    let _ = append_app_key_link_audit(config_dir, relay_audit);
    match relay_result {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(error)) => Err(format!(
            "sending device approval applied ACK failed over FIPS ({fips_error}) and relays ({error})"
        )),
        Err(_) => Err(format!(
            "sending device approval applied ACK failed over FIPS ({fips_error}) and relays timed out"
        )),
    }
}
