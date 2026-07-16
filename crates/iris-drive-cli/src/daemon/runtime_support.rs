struct DaemonCommandStartup {
    runtime: tokio::runtime::Runtime,
    config: AppConfig,
    state: ProfileState,
    relays: Vec<String>,
    filters: Vec<nostr_sdk::Filter>,
    subscription_policy: iris_drive_core::relay_sync::RelayEventRetentionPolicy,
    embedded_hashtree_requested: bool,
    embedded_hashtree: Option<EmbeddedHashtreeHost>,
    embedded_hashtree_status: Value,
}

fn prepare_daemon_command(
    config_dir: &Path,
    relay_override: &[String],
    enable_gateway: bool,
) -> Result<DaemonCommandStartup> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_stack_size(DAEMON_TOKIO_WORKER_STACK_BYTES)
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .profile
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    if state.authorization_state == iris_drive_core::AppKeyAuthorizationState::Revoked {
        write_runtime_daemon_status(
            config_dir,
            json!({
                "event": "revoked",
                "error": "device removed",
            }),
        );
        return Err(anyhow::anyhow!(
            "this device has been removed from Iris Drive; link it again or log out"
        ));
    }
    let relays = pick_relays(&config, relay_override);
    let root_scope_id = state.root_scope_id();
    let share_ids = config
        .shared_folders
        .iter()
        .map(|folder| folder.share_id)
        .collect::<Vec<_>>();
    let filters = iris_drive_core::relay_sync::subscription_filters_for_shared_roots(
        &state.app_key_pubkey,
        &root_scope_id,
        iris_drive_core::PRIMARY_DRIVE_ID,
        &share_ids,
    );
    if filters.is_empty() {
        return Err(anyhow::anyhow!("no filters to subscribe to"));
    }
    let subscription_policy =
        iris_drive_core::relay_sync::event_retention_policy(filters.clone());
    let embedded_hashtree_requested = enable_gateway && config.local_nhash_resolver_enabled;
    let (embedded_hashtree, embedded_hashtree_status) = if embedded_hashtree_requested {
        match EmbeddedHashtreeHost::start(config_dir, &config) {
            Ok(host) => {
                let status = host.status_payload();
                (Some(host), status)
            }
            Err(error) => {
                let error = format!("{error:#}");
                println!(
                    "{}",
                    json!({
                        "event": "embedded_hashtree_unavailable",
                        "error": error,
                    })
                );
                (None, json!({"running": false, "error": error}))
            }
        }
    } else {
        let disabled_by = if enable_gateway { "settings" } else { "cli" };
        (
            None,
            json!({
                "running": false,
                "disabled_by": disabled_by,
            }),
        )
    };

    Ok(DaemonCommandStartup {
        runtime,
        config,
        state,
        relays,
        filters,
        subscription_policy,
        embedded_hashtree_requested,
        embedded_hashtree,
        embedded_hashtree_status,
    })
}

#[allow(clippy::too_many_arguments)]
async fn handle_direct_app_message_event(
    recv: Option<
        Result<
            iris_drive_core::FipsAppMessage,
            tokio::sync::broadcast::error::RecvError,
        >,
    >,
    direct_app_message_rx: &mut Option<
        tokio::sync::broadcast::Receiver<iris_drive_core::FipsAppMessage>,
    >,
    config_dir: &Path,
    acked_app_key_link_rosters: &mut BTreeSet<String>,
    direct_roots: &mut DirectRootExchange,
    client: &nostr_sdk::Client,
    fips_blocks: Option<&Arc<FsFipsBlockSync>>,
    mount_refresh_tx: Option<&tokio::sync::mpsc::Sender<&'static str>>,
    daemon_tasks: &DaemonTaskSet,
) -> bool {
    use tokio::sync::broadcast::error::{RecvError, TryRecvError};

    let Some(result) = recv else {
        *direct_app_message_rx = None;
        println!("{}", json!({"event": "direct_root_app_closed"}));
        return false;
    };
    let message = match result {
        Ok(message) => message,
        Err(RecvError::Lagged(n)) => {
            println!(
                "{}",
                json!({"event": "direct_root_app_lagged", "skipped": n})
            );
            return false;
        }
        Err(RecvError::Closed) => {
            *direct_app_message_rx = None;
            println!("{}", json!({"event": "direct_root_app_closed"}));
            return false;
        }
    };

    let should_coalesce_direct_roots =
        message.topic == iris_drive_core::DIRECT_ROOT_APP_TOPIC;
    let mut messages = vec![message];
    let mut receiver_closed = false;
    if should_coalesce_direct_roots {
        tokio::time::sleep(std::time::Duration::from_millis(
            DIRECT_ROOT_RECEIVE_COALESCE_MS,
        ))
        .await;
    }
    if let Some(rx) = direct_app_message_rx.as_mut() {
        while messages.len() < DIRECT_APP_MESSAGE_DRAIN_LIMIT {
            match rx.try_recv() {
                Ok(message) => messages.push(message),
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Lagged(n)) => println!(
                    "{}",
                    json!({"event": "direct_root_app_lagged", "skipped": n})
                ),
                Err(TryRecvError::Closed) => {
                    receiver_closed = true;
                    break;
                }
            }
        }
    }
    let received_messages = messages.len();
    let (messages, coalesced_roots) =
        iris_drive_core::coalesce_direct_root_app_messages(messages);
    if coalesced_roots > 0 {
        println!(
            "{}",
            json!({
                "event": "direct_root_app_coalesced",
                "received_messages": received_messages,
                "applied_messages": messages.len(),
                "skipped_roots": coalesced_roots,
            })
        );
    }

    let mut announce_pending = false;
    for message in messages {
        let wakes_direct_roots_after_app_key_link =
            message.topic == crate::profile::APP_KEY_LINK_ROSTER_APP_TOPIC
                || message.topic == crate::profile::APP_KEY_APPROVAL_RECEIPT_APP_TOPIC;
        match handle_app_key_link_app_message(
            config_dir,
            &message,
            fips_blocks.map(AsRef::as_ref),
            acked_app_key_link_rosters,
        )
        .await
        {
            Ok(true) => {
                if wakes_direct_roots_after_app_key_link && fips_blocks.is_some() {
                    announce_pending = true;
                    enqueue_pending_root_sync_followups(
                        config_dir,
                        fips_blocks.cloned(),
                        mount_refresh_tx.cloned(),
                        daemon_tasks,
                        "app_key_link_authorization_message",
                    );
                }
                continue;
            }
            Ok(false) => {}
            Err(error) => {
                println!(
                    "{}",
                    json!({"event": "app_key_link_request_receive_error", "error": format!("{error:#}")})
                );
                continue;
            }
        }
        if let Some(sync) = fips_blocks {
            match direct_roots
                .handle_app_message(
                    client,
                    config_dir,
                    sync.clone(),
                    mount_refresh_tx.cloned(),
                    daemon_tasks,
                    message,
                )
                .await
            {
                Ok(true) => announce_pending = true,
                Ok(false) => {}
                Err(error) => println!(
                    "{}",
                    json!({"event": "direct_root_app_error", "error": format!("{error:#}")})
                ),
            }
        }
    }
    if should_coalesce_direct_roots {
        enqueue_pending_root_sync_followups(
            config_dir,
            fips_blocks.cloned(),
            mount_refresh_tx.cloned(),
            daemon_tasks,
            "direct_root_app_message",
        );
    }
    if receiver_closed {
        *direct_app_message_rx = None;
        println!("{}", json!({"event": "direct_root_app_closed"}));
    }
    announce_pending
}

fn handle_nostr_pubsub_event(
    message: &iris_drive_core::FipsNostrPubsubEvent,
    update_announcements: &mut iris_drive_core::UpdateAnnouncementExchange,
    config_dir: &Path,
) {
    match update_announcements.handle_nostr_event(config_dir, message) {
        Ok(true) => println!(
            "{}",
            json!({
                "event": "update_announcement_received",
                "origin": message.origin_peer_id,
            })
        ),
        Ok(false) => {}
        Err(error) => println!(
            "{}",
            json!({"event": "update_announcement_error", "trigger": "nostr_pubsub_event", "error": error})
        ),
    }
}

async fn shutdown_fips_block_sync(fips_blocks: Option<Arc<FsFipsBlockSync>>) {
    let Some(sync) = fips_blocks else {
        return;
    };
    match Arc::try_unwrap(sync) {
        Ok(sync) => {
            if let Err(error) = sync.shutdown().await {
                println!(
                    "{}",
                    json!({"event": "fips_block_sync_shutdown_error", "error": format!("{error:#}")})
                );
            }
        }
        Err(sync) => {
            if let Err(error) = sync.shutdown_endpoint().await {
                println!(
                    "{}",
                    json!({"event": "fips_block_sync_shutdown_error", "error": format!("{error:#}")})
                );
            }
            println!(
                "{}",
                json!({"event": "fips_block_sync_shutdown_retained", "owners": Arc::strong_count(&sync)})
            );
            std::mem::forget(sync);
        }
    }
}
