/// Follow the folders this device shares, joins and hands out links to.
/// Returns the filters to retain, so their events reach `apply_one_event`.
async fn refresh_share_join_subscriptions(
    client: &nostr_sdk::Client,
    config_dir: &Path,
    config: &AppConfig,
    subscriptions: &mut iris_drive_core::share_join::ShareRelaySubscriptionState,
    daemon_tasks: &DaemonTaskSet,
) -> Result<Vec<nostr_sdk::Filter>> {
    let store = iris_drive_core::share_join::ShareJoinStore::load(config_dir)?;
    let filters = iris_drive_core::share_join::share_relay_filters(config, &store);
    let changed =
        iris_drive_core::share_join::refresh_share_relay_subscriptions(client, &filters, subscriptions)
            .await?;
    if changed && !store.pending.is_empty() {
        // Ask again whenever what we wait on changes; relays keep only the latest request.
        let client = client.clone();
        let dir = config_dir.to_path_buf();
        daemon_tasks.push_keyed(
            "share-join-requests".to_owned(),
            tokio::spawn(async move {
                if let Err(error) =
                    iris_drive_core::share_join::publish_pending_share_join_requests(&client, &dir).await
                {
                    emit_daemon_status_event(
                        &dir,
                        json!({"event": "share_join_request_error", "error": format!("{error:#}")}),
                    );
                }
            }),
        );
    }
    Ok(filters)
}

/// Requests made with this device's links, and access lists answering the
/// joins it asked for. `None` when the event is something else.
async fn apply_share_join_relay_event(
    _client: &nostr_sdk::Client,
    config_dir: &Path,
    event: &nostr_sdk::Event,
    daemon_tasks: &DaemonTaskSet,
) -> Result<Option<EventApplyOutcome>> {
    use iris_drive_core::share_join::{self, ShareJoinEventOutcome};
    let store = share_join::ShareJoinStore::load(config_dir)?;
    if !share_join::is_share_join_event(event, &store) {
        return Ok(None);
    }
    let outcome = match share_join::apply_share_join_event(config_dir, event).await {
        Ok(outcome) => outcome,
        Err(error) => {
            emit_daemon_status_event(
                config_dir,
                json!({"event": "share_join_error", "id": event.id.to_hex(), "error": format!("{error:#}")}),
            );
            return Ok(Some(EventApplyOutcome::Unchanged));
        }
    };
    let (label, share_id) = match outcome {
        ShareJoinEventOutcome::Approved(share_id) => ("approved", share_id),
        ShareJoinEventOutcome::Joined(share_id) => ("joined", share_id),
        ShareJoinEventOutcome::Ignored => return Ok(Some(EventApplyOutcome::Unchanged)),
    };
    emit_daemon_status_event(
        config_dir,
        json!({"event": "share_join", "outcome": label, "share_id": share_id.to_string()}),
    );
    if matches!(outcome, ShareJoinEventOutcome::Approved(_)) {
        let dir = config_dir.to_path_buf();
        daemon_tasks.push_keyed(
            format!("share-join-publish:{share_id}"),
            tokio::spawn(async move {
                if let Err(error) = share_join::publish_approved_share(&dir, share_id).await {
                    emit_daemon_status_event(
                        &dir,
                        json!({"event": "share_join_publish_error", "share_id": share_id.to_string(), "error": format!("{error:#}")}),
                    );
                }
            }),
        );
    }
    Ok(Some(EventApplyOutcome::Changed))
}
