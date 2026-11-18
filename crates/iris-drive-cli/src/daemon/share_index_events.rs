/// The profile's shared folders as another of its devices lists them: what
/// it shared, joined, moved or left applies here too.
async fn apply_share_index_relay_event(
    config_dir: &Path,
    event: &nostr_sdk::Event,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
    daemon_tasks: &DaemonTaskSet,
) -> Result<EventApplyOutcome> {
    use iris_drive_core::share_index::{self, ShareIndexApply};
    let outcome = match share_index::apply_share_index_event(config_dir, event).await {
        Ok(outcome) => outcome,
        Err(error) => {
            emit_daemon_status_event(
                config_dir,
                json!({"event": "share_index_error", "id": event.id.to_hex(), "error": format!("{error:#}")}),
            );
            return Ok(EventApplyOutcome::Unchanged);
        }
    };
    if outcome != ShareIndexApply::Applied {
        return Ok(EventApplyOutcome::Unchanged);
    }
    emit_daemon_status_event(
        config_dir,
        json!({"event": "share_index", "outcome": "applied", "author": pubkey_npub(&event.pubkey.to_hex())}),
    );
    // A folder that moved or left My Drive changes what the provider shows.
    let config = AppConfig::load_or_default_cached_profile(config_path_in(config_dir))?;
    enqueue_root_apply_followup(
        config_dir.to_path_buf(),
        config,
        None,
        fips_blocks,
        true,
        "share_index_projection",
        mount_refresh,
        daemon_tasks,
    );
    Ok(EventApplyOutcome::Changed)
}

/// What this device's shared folders and the profile's devices looked like
/// when the list was last reconciled.
fn share_index_reconcile_key(config_dir: &Path) -> Vec<Option<(u64, std::time::SystemTime)>> {
    [
        config_path_in(config_dir),
        config_dir.join(iris_drive_core::share_index::SHARE_INDEX_FILE),
    ]
    .iter()
    .map(|path| {
        let metadata = std::fs::metadata(path).ok()?;
        Some((metadata.len(), metadata.modified().ok()?))
    })
    .collect()
}

/// Keep the profile's other devices told of this device's shared folders, and
/// let devices linked to the profile into the folders this device answers
/// for. Runs for as long as Drive does: whenever the config or the list
/// changes, and once at startup so relays that lost the list get it again.
///
/// The returned channel says when directly connected devices have something
/// new to hear.
fn start_share_index_reconciler(
    client: &nostr_sdk::Client,
    config_dir: &Path,
    daemon_tasks: &DaemonTaskSet,
) -> tokio::sync::mpsc::UnboundedReceiver<()> {
    let (changed_tx, changed_rx) = tokio::sync::mpsc::unbounded_channel();
    let client = client.clone();
    let config_dir = config_dir.to_path_buf();
    daemon_tasks.push(tokio::spawn(async move {
        let mut reconciled = None;
        let mut timer =
            tokio::time::interval(std::time::Duration::from_millis(APP_KEY_LINK_TICK_MILLIS));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            timer.tick().await;
            let key = share_index_reconcile_key(&config_dir);
            if reconciled.as_ref() == Some(&key) {
                continue;
            }
            let first = reconciled.is_none();
            match iris_drive_core::share_index::reconcile_share_index(&config_dir, &client, first)
                .await
            {
                Ok(report) => {
                    // Its own writes run it once more, to no effect.
                    reconciled = Some(key);
                    if report.changed() {
                        // Devices reached directly, without relays, hear it too.
                        let _ = changed_tx.send(());
                    }
                    if !report.admitted.is_empty() {
                        emit_daemon_status_event(
                            &config_dir,
                            json!({
                                "event": "share_devices_admitted",
                                "share_ids": report.admitted.iter().map(ToString::to_string).collect::<Vec<_>>(),
                            }),
                        );
                    }
                }
                Err(error) => {
                    emit_daemon_status_event(
                        &config_dir,
                        json!({"event": "share_index_reconcile_error", "error": format!("{error:#}")}),
                    );
                    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                }
            }
        }
    }));
    changed_rx
}

fn start_shared_file_returns(
    client: &nostr_sdk::Client,
    config_dir: &Path,
    daemon_tasks: &DaemonTaskSet,
) {
    let client = client.clone();
    let config_dir = config_dir.to_path_buf();
    daemon_tasks.push(tokio::spawn(async move {
        let mut timer = tokio::time::interval(std::time::Duration::from_secs(30));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            timer.tick().await;
            if let Err(error) = iris_drive_core::shared_owner_return::sync_shared_file_returns(
                &client, &config_dir, std::time::Duration::from_secs(6),
            ).await {
                tracing::debug!(%error, "shared file return sync will retry");
            }
        }
    }));
}
