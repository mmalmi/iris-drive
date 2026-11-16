/// Apply one Drive event that arrived from relays or, over the FIPS feed, from
/// one of the account's own devices. True when the current state changed and
/// should be announced to the other devices.
async fn apply_received_drive_event(
    event: &nostr_sdk::Event,
    subscription_policy: &iris_drive_core::relay_sync::RelayEventRetentionPolicy,
    client: &nostr_sdk::Client,
    config_dir: &Path,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    mount_refresh_tx: Option<tokio::sync::mpsc::Sender<&'static str>>,
    daemon_tasks: &DaemonTaskSet,
) -> Result<bool> {
    let is_device_approval_receipt =
        iris_drive_core::relay_sync::is_device_approval_receipt_event(event);
    let awaiting_approval = AppConfig::load_or_default_cached_profile(config_path_in(config_dir))?
        .profile
        .as_ref()
        .is_some_and(|profile| {
            profile.authorization_state == iris_drive_core::AppKeyAuthorizationState::AwaitingApproval
        });
    if should_defer_relay_roster_event_while_awaiting(
        event.kind.as_u16(),
        is_device_approval_receipt,
        awaiting_approval,
    ) {
        return Ok(false);
    }
    if !is_device_approval_receipt
        && !iris_drive_core::relay_sync::relay_event_matches_policy(subscription_policy, event)
    {
        return Ok(false);
    }
    match apply_one_event(client, config_dir, event, fips_blocks, mount_refresh_tx, daemon_tasks)
        .await
    {
        Ok(outcome) => Ok(outcome.should_announce_current_state()),
        Err(error) => {
            println!(
                "{}",
                json!({"event": "apply_error", "id": event.id.to_hex(), "error": error.to_string()})
            );
            Ok(false)
        }
    }
}
