async fn refresh_app_key_link_relay_subscriptions_for_config(
    client: &nostr_sdk::Client,
    config_dir: &Path,
    subscriptions: &mut iris_drive_core::relay_sync::AppKeyLinkRelaySubscriptionState,
) -> Result<Option<iris_drive_core::relay_sync::RelayEventRetentionPolicy>> {
    let config = AppConfig::load_or_default_cached_profile(config_path_in(config_dir))?;
    let Some(state) = config.profile.as_ref() else {
        return Ok(None);
    };
    let share_ids = config
        .shared_folders
        .iter()
        .map(|folder| folder.share_id)
        .collect::<Vec<_>>();
    let policy = iris_drive_core::relay_sync::event_retention_policy(
        iris_drive_core::relay_sync::subscription_filters_for_shared_roots(
            &state.app_key_pubkey,
            &state.root_scope_id(),
            iris_drive_core::PRIMARY_DRIVE_ID,
            &share_ids,
        ),
    );
    iris_drive_core::relay_sync::refresh_app_key_link_relay_subscriptions(
        client,
        state,
        subscriptions,
    )
    .await?;
    Ok(Some(policy))
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
