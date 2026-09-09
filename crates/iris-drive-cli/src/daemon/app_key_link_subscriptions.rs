async fn refresh_app_key_link_relay_subscriptions_for_config(
    client: &nostr_sdk::Client,
    config_dir: &Path,
    config_cache: &mut AppConfigLoadCache,
    subscriptions: &mut iris_drive_core::relay_sync::AppKeyLinkRelaySubscriptionState,
) -> Result<Option<iris_drive_core::relay_sync::RelayEventRetentionPolicy>> {
    let config = load_app_config_cached(&config_path_in(config_dir), config_cache)?;
    let Some(state) = config.profile.as_ref() else {
        return Ok(None);
    };
    let share_ids = config
        .shared_folders
        .iter()
        .map(|folder| folder.share_id)
        .collect::<Vec<_>>();
    let mut filters = iris_drive_core::relay_sync::subscription_filters_for_shared_roots(
        &state.app_key_pubkey,
        &state.root_scope_id(),
        iris_drive_core::PRIMARY_DRIVE_ID,
        &share_ids,
    );
    if let Some(filter) =
        iris_drive_core::relay_sync::pending_device_approval_applied_ack_filter(state)?
    {
        filters.push(filter);
    }
    let policy = iris_drive_core::relay_sync::event_retention_policy(filters);
    iris_drive_core::relay_sync::refresh_app_key_link_relay_subscriptions(
        client,
        state,
        subscriptions,
    )
    .await?;
    Ok(Some(policy))
}

fn spawn_pending_device_approval_ack_replay(
    config_dir: &Path,
    relays: &[String],
    daemon_tasks: &DaemonTaskSet,
) {
    const TASK_KEY: &str = "pending_device_approval_ack_replay";
    if relays.is_empty() {
        return;
    }
    let Ok(config) = AppConfig::load_or_default_cached_profile(config_path_in(config_dir)) else {
        return;
    };
    if !config.profile.as_ref().is_some_and(|profile| {
        !profile.pending_device_approval_receipts.is_empty()
    }) {
        return;
    }
    let config_dir = config_dir.to_path_buf();
    let relays = relays.to_vec();
    let task_config_dir = config_dir.clone();
    let task = tokio::spawn(async move {
        emit_daemon_status_event(
            &task_config_dir,
            json!({"event": "nostr_identity_device_approval_applied_ack_replay_started"}),
        );
        loop {
            let Ok(config) = AppConfig::load_or_default_cached_profile(config_path_in(&task_config_dir)) else {
                return;
            };
            if !config.profile.as_ref().is_some_and(|profile| {
                !profile.pending_device_approval_receipts.is_empty()
            }) {
                return;
            }
            match iris_drive_core::sync_pending_device_approval_acks(
                &task_config_dir,
                &relays,
                std::time::Duration::from_secs(2),
            )
            .await
            {
                Ok(report) if report.device_approval_applied_acks_applied > 0 => {
                    emit_daemon_status_event(
                        &task_config_dir,
                        json!({
                            "event": "nostr_identity_device_approval_applied_ack_replay",
                            "seen": report.device_approval_applied_acks_seen,
                            "applied": report.device_approval_applied_acks_applied,
                        }),
                    );
                    return;
                }
                Ok(_) => {}
                Err(error) => emit_daemon_status_event(
                    &task_config_dir,
                    json!({
                        "event": "nostr_identity_device_approval_applied_ack_replay_error",
                        "error": format!("{error:#}"),
                    }),
                ),
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    let _ = daemon_tasks.push_keyed(TASK_KEY.to_string(), task);
}

fn should_defer_relay_roster_event_while_awaiting(
    kind: u16,
    is_device_approval_receipt: bool,
    awaiting_approval: bool,
) -> bool {
    kind == iris_drive_core::KIND_NOSTR_IDENTITY_ROSTER_OP
        && awaiting_approval
        && !is_device_approval_receipt
}

#[cfg(test)]
mod app_key_link_subscription_tests {
    use super::should_defer_relay_roster_event_while_awaiting;

    #[test]
    fn awaiting_devices_still_accept_approval_receipt_events() {
        assert!(!should_defer_relay_roster_event_while_awaiting(
            iris_drive_core::KIND_NOSTR_IDENTITY_ROSTER_OP,
            true,
            true,
        ));
        assert!(should_defer_relay_roster_event_while_awaiting(
            iris_drive_core::KIND_NOSTR_IDENTITY_ROSTER_OP,
            false,
            true,
        ));
    }
}
