#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum EventApplyOutcome {
    Changed,
    Unchanged,
    RetryablePrerequisiteMissing,
}

const ROOT_APPLY_FOLLOWUP_COALESCE_MS: u64 = 750;
const ROOT_APPLY_STATE_REPLY_SETTLE_MS: u64 = 500;
const DIRECT_ROOT_STATE_REQUEST_MIN_INTERVAL_SECS: u64 = 30;

static DIRECT_ROOT_STATE_REQUEST_THROTTLE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::BTreeMap<String, std::time::Instant>>,
> = std::sync::OnceLock::new();

impl EventApplyOutcome {
    pub(crate) const fn should_cache_direct_root_frame(self) -> bool {
        !matches!(self, Self::RetryablePrerequisiteMissing)
    }

    pub(crate) const fn should_announce_current_state(self) -> bool {
        matches!(self, Self::Changed)
    }
}

pub(crate) fn drive_root_apply_outcome_is_retryable(
    outcome: &iris_drive_core::relay_sync::DriveRootApply,
) -> bool {
    matches!(
        outcome,
        iris_drive_core::relay_sync::DriveRootApply::NotOurScope
            | iris_drive_core::relay_sync::DriveRootApply::UnknownDrive
            | iris_drive_core::relay_sync::DriveRootApply::UnauthorizedAppKey
            | iris_drive_core::relay_sync::DriveRootApply::KeyUnavailable
    )
}

#[allow(
    clippy::needless_return,
    clippy::redundant_else,
    clippy::too_many_lines
)]
pub(crate) async fn apply_one_event(
    _client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    event: &nostr_sdk::Event,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
    daemon_tasks: &DaemonTaskSet,
) -> Result<EventApplyOutcome> {
    use iris_drive_core::relay_sync;
    let config_lock = ConfigMutationLock::acquire(config_dir).await?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let kind = event.kind.as_u16();
    if iris_drive_core::nostr_events::is_app_key_link_request_event_coordinate(event) {
        let outcome = relay_sync::apply_remote_app_key_link_request_event(&mut config, event)?;
        emit_daemon_status_event(
            config_dir,
            json!({
                "event": "app_key_link_request",
                "event_id": event.id.to_hex(),
                "author": pubkey_npub(&event.pubkey.to_hex()),
                "outcome": format!("{outcome:?}"),
            }),
        );
        let changed = matches!(outcome, relay_sync::AppKeyLinkRequestApply::Recorded);
        if changed {
            config.save(config_path_in(config_dir))?;
        }
        return Ok(if changed {
            EventApplyOutcome::Changed
        } else {
            EventApplyOutcome::Unchanged
        });
    } else if iris_drive_core::is_nostr_identity_roster_op_event_coordinate(event) {
        let outcome = relay_sync::apply_remote_nostr_identity_roster_op_event(&mut config, event)?;
        emit_daemon_status_event(
            config_dir,
            json!({
                "event": "nostr_identity_roster_op",
                "event_id": event.id.to_hex(),
                "author": pubkey_npub(&event.pubkey.to_hex()),
                "outcome": format!("{outcome:?}"),
            }),
        );
        if matches!(outcome, relay_sync::NostrIdentityRosterOpApply::Applied) {
            config.save(config_path_in(config_dir))?;
            drop(config_lock);
            if let Some(sync) = fips_blocks.as_deref() {
                sync.refresh_authorized_peers(&config).await;
            }
            return Ok(EventApplyOutcome::Changed);
        }
        return Ok(EventApplyOutcome::Unchanged);
    } else if iris_drive_core::is_share_access_snapshot_event_coordinate(event) {
        let outcome = relay_sync::apply_remote_share_access_snapshot_event(&mut config, event)?;
        emit_daemon_status_event(
            config_dir,
            json!({
                "event": "share_access_snapshot",
                "event_id": event.id.to_hex(),
                "author": pubkey_npub(&event.pubkey.to_hex()),
                "outcome": format!("{outcome:?}"),
            }),
        );
        if matches!(outcome, relay_sync::ShareAccessSnapshotApply::Applied) {
            config.save(config_path_in(config_dir))?;
            drop(config_lock);
            if let Some(sync) = fips_blocks.as_deref() {
                sync.refresh_authorized_peers(&config).await;
            }
            return Ok(EventApplyOutcome::Changed);
        }
        return Ok(EventApplyOutcome::Unchanged);
    } else if iris_drive_core::nostr_events::is_drive_root_event_coordinate(event) {
        let device = iris_drive_core::identity::AppKey::load(key_path_in(config_dir))
            .context("loading app key")?;
        let parsed =
            iris_drive_core::nostr_events::parse_drive_root_event_for_device(event, device.keys())
                .ok();
        let outcome =
            relay_sync::apply_remote_drive_root_event(&mut config, event, Some(device.keys()))?;
        let retryable = drive_root_apply_outcome_is_retryable(&outcome);
        let was_applied = matches!(outcome, relay_sync::DriveRootApply::Applied);
        let stale_current_root = matches!(outcome, relay_sync::DriveRootApply::StaleTimestamp)
            && parsed
                .as_ref()
                .is_some_and(|(app_key_pubkey, _, drive_id, root_ref)| {
                    config
                        .drive(drive_id)
                        .and_then(|drive| drive.app_key_roots.get(app_key_pubkey))
                        .is_some_and(|stored| stored.root_cid == root_ref.root_cid)
                });
        let parsed_root_cid = parsed
            .as_ref()
            .map(|(_, _, _, root_ref)| root_ref.root_cid.clone());
        let root_blocks_already_synced = parsed_root_cid
            .as_deref()
            .is_some_and(|root_cid| root_has_successful_block_sync(config_dir, root_cid));
        let followup =
            drive_root_followup_plan(was_applied, stale_current_root, root_blocks_already_synced);
        let root_cid_to_pull = parsed_root_cid
            .as_ref()
            .filter(|_| followup.pull_blocks)
            .cloned();
        emit_daemon_status_event(
            config_dir,
            json!({
                "event": "drive_root",
                "event_id": event.id.to_hex(),
                "author": pubkey_npub(&event.pubkey.to_hex()),
                "outcome": format!("{outcome:?}"),
                "root_cid": root_cid_to_pull.clone(),
            }),
        );
        if was_applied {
            config.save(config_path_in(config_dir))?;
        }
        drop(config_lock);
        if let Some(sync) = fips_blocks.as_deref() {
            sync.refresh_authorized_peers(&config).await;
        }

        enqueue_root_apply_followup(
            config_dir.to_path_buf(),
            config.clone(),
            root_cid_to_pull,
            fips_blocks,
            followup.refresh_projection,
            "projected_drive_root",
            mount_refresh,
            daemon_tasks,
        );
        return Ok(if retryable {
            EventApplyOutcome::RetryablePrerequisiteMissing
        } else if was_applied {
            EventApplyOutcome::Changed
        } else {
            EventApplyOutcome::Unchanged
        });
    } else if kind == iris_drive_core::nostr_events::KIND_HASHTREE_ROOT {
        let Some(account_state) = config.profile.clone() else {
            return Ok(EventApplyOutcome::Unchanged);
        };
        return apply_files_root_event(
            config_dir,
            event,
            fips_blocks,
            mount_refresh,
            daemon_tasks,
            &mut config,
            account_state,
        );
    } else {
        // Unknown kind; ignore.
        return Ok(EventApplyOutcome::Unchanged);
    }
}

pub(crate) fn apply_files_root_event(
    config_dir: &std::path::Path,
    event: &nostr_sdk::Event,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
    daemon_tasks: &DaemonTaskSet,
    config: &mut AppConfig,
    account_state: ProfileState,
) -> Result<EventApplyOutcome> {
    use iris_drive_core::relay_sync;
    if !account_state.can_write_roots() {
        println!(
            "{}",
            json!({
                "event": "files_root",
                "event_id": event.id.to_hex(),
                "author": pubkey_npub(&event.pubkey.to_hex()),
                "outcome": "app_key_cannot_write_roots",
            })
        );
        return Ok(EventApplyOutcome::Unchanged);
    }
    let account = Profile::load(account_state, config_dir).context("loading profile")?;
    let outcome =
        relay_sync::apply_remote_files_root_event(config, event, Some(account.app_key.keys()))?;
    let was_applied = matches!(outcome, relay_sync::FilesRootApply::Applied);
    let tree_name = event
        .tags
        .identifier()
        .map_or_else(|| iris_drive_core::PRIMARY_DRIVE_ID.to_string(), str::to_owned);
    let root_cid_to_pull = if was_applied {
        config
            .drive(&tree_name)
            .and_then(|drive| drive.app_key_roots.get(&account.state.app_key_pubkey))
            .map(|root| root.root_cid.clone())
    } else {
        None
    };
    emit_daemon_status_event(
        config_dir,
        json!({
            "event": "files_root",
            "event_id": event.id.to_hex(),
            "author": pubkey_npub(&event.pubkey.to_hex()),
            "outcome": files_root_apply_label(&outcome),
            "tree_name": tree_name.clone(),
            "root_cid": root_cid_to_pull.clone(),
        }),
    );
    if was_applied {
        config.save(config_path_in(config_dir))?;
    }
    enqueue_root_apply_followup(
        config_dir.to_path_buf(),
        config.clone(),
        root_cid_to_pull,
        fips_blocks,
        was_applied && tree_name == iris_drive_core::PRIMARY_DRIVE_ID,
        "projected_files_root",
        mount_refresh,
        daemon_tasks,
    );
    Ok(if was_applied {
        EventApplyOutcome::Changed
    } else {
        EventApplyOutcome::Unchanged
    })
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct DriveRootFollowupPlan {
    pull_blocks: bool,
    refresh_projection: bool,
}

fn drive_root_followup_plan(
    was_applied: bool,
    stale_current_root: bool,
    _root_blocks_already_synced: bool,
) -> DriveRootFollowupPlan {
    DriveRootFollowupPlan {
        pull_blocks: was_applied || stale_current_root,
        refresh_projection: was_applied || stale_current_root,
    }
}

pub(crate) fn root_has_successful_block_sync(config_dir: &Path, root_cid: &str) -> bool {
    load_daemon_status(config_dir)
        .and_then(|status| {
            status
                .get("block_sync_by_root")
                .and_then(|roots| roots.get(root_cid))
                .cloned()
        })
        .is_some()
}

fn startup_root_cids_needing_sync(config_dir: &Path, config: &AppConfig) -> Vec<String> {
    config
        .drives
        .iter()
        .flat_map(|drive| drive.app_key_roots.values())
        .filter(|root| !root.local_only)
        .filter(|root| !root_has_successful_block_sync(config_dir, &root.root_cid))
        .map(|root| root.root_cid.clone())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

pub(crate) fn enqueue_pending_root_sync_followups(
    config_dir: &Path,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
    daemon_tasks: &DaemonTaskSet,
    projection_event: &'static str,
) -> usize {
    let Ok(config) = AppConfig::load_or_default_cached_profile(config_path_in(config_dir)) else {
        return 0;
    };
    let roots = startup_root_cids_needing_sync(config_dir, &config);
    let mut enqueued = 0;
    for root_cid in roots {
        if enqueue_root_apply_followup(
            config_dir.to_path_buf(),
            config.clone(),
            Some(root_cid),
            fips_blocks.clone(),
            true,
            projection_event,
            mount_refresh.clone(),
            daemon_tasks,
        ) {
            enqueued += 1;
        }
    }
    if enqueued > 0 {
        println!(
            "{}",
            json!({
                "event": "pending_root_sync_retry_enqueued",
                "count": enqueued,
                "trigger": projection_event,
            })
        );
    }
    enqueued
}

#[allow(clippy::too_many_lines)]
pub(crate) fn spawn_root_apply_followup(
    config_dir: PathBuf,
    config: AppConfig,
    root_cid_to_pull: Option<String>,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    should_refresh_projection: bool,
    projection_event: &'static str,
    mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
) -> Option<tokio::task::JoinHandle<()>> {
    if root_cid_to_pull.is_none() && !should_refresh_projection {
        return None;
    }
    let expected_projection_root_key =
        root_apply_followup_key(&config, root_cid_to_pull.as_deref(), should_refresh_projection);
    let root_cid_for_materialize = root_cid_to_pull
        .as_ref()
        .filter(|root_cid| root_cid_belongs_to_peer(&config, root_cid))
        .cloned();
    let should_coalesce_followup =
        root_cid_to_pull.is_some() && expected_projection_root_key.is_some();

    Some(tokio::spawn(async move {
        if should_coalesce_followup {
            tokio::time::sleep(std::time::Duration::from_millis(
                ROOT_APPLY_FOLLOWUP_COALESCE_MS,
            ))
            .await;
        }
        if let Some(root_cid) = root_cid_to_pull {
            if root_apply_followup_is_stale(&config_dir, expected_projection_root_key.as_ref()) {
                println!(
                    "{}",
                    json!({
                        "event": "root_apply_followup_skipped_stale",
                        "root_cid": root_cid,
                    })
                );
                return;
            }
            request_latest_direct_root_state(
                &config,
                fips_blocks.as_deref(),
                projection_event,
                false,
            )
            .await;
            if settle_after_direct_root_state_request(
                &config_dir,
                expected_projection_root_key.as_ref(),
                &root_cid,
            )
            .await
            {
                return;
            }
            let mut last_error = None;
            for delay_secs in event_block_pull_retry_delays(&config) {
                if root_apply_followup_is_stale(&config_dir, expected_projection_root_key.as_ref())
                {
                    println!(
                        "{}",
                        json!({
                            "event": "root_apply_followup_skipped_stale",
                            "root_cid": root_cid,
                        })
                    );
                    return;
                }
                if *delay_secs > 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(*delay_secs)).await;
                    if root_apply_followup_is_stale(
                        &config_dir,
                        expected_projection_root_key.as_ref(),
                    ) {
                        println!(
                            "{}",
                            json!({
                                "event": "root_apply_followup_skipped_stale",
                                "root_cid": root_cid,
                            })
                        );
                        return;
                    }
                }
                match pull_blocks_for_root_bounded(
                    &config_dir,
                    &config,
                    &root_cid,
                    fips_blocks.as_deref(),
                )
                .await
                {
                    Ok(()) => {
                        last_error = None;
                        break;
                    }
                    Err(error) => {
                        println!(
                            "{}",
                            json!({
                                "event": "block_download_retry",
                                "root_cid": root_cid,
                                "delay_secs": delay_secs,
                                "error": error,
                            })
                        );
                        last_error = Some(error);
                        request_latest_direct_root_state(
                            &config,
                            fips_blocks.as_deref(),
                            projection_event,
                            true,
                        )
                        .await;
                        if settle_after_direct_root_state_request(
                            &config_dir,
                            expected_projection_root_key.as_ref(),
                            &root_cid,
                        )
                        .await
                        {
                            return;
                        }
                    }
                }
            }
            if let Some(error) = last_error {
                request_latest_direct_root_state(
                    &config,
                    fips_blocks.as_deref(),
                    projection_event,
                    true,
                )
                .await;
                record_block_sync_error(&config_dir, &root_cid, &error);
                println!(
                    "{}",
                    json!({
                        "event": "block_download_error",
                        "root_cid": root_cid,
                        "error": error,
                        "projection_refresh_skipped": should_refresh_projection,
                    })
                );
                return;
            }
        }

        if root_cid_for_materialize.is_some() {
            match materialize_primary_merged_root_for_followup(&config_dir).await
            {
                Ok(Some(report)) => emit_daemon_status_event(
                    &config_dir,
                    json!({
                        "event": "merged_root_materialized",
                        "root_cid": report.root_cid.clone(),
                        "file_count": report.file_count,
                        "top_level_entries": report.top_level_entries,
                        "hashtree": {
                            "current_root_cid": report.root_cid,
                            "file_count": report.file_count,
                            "top_level_entries": report.top_level_entries,
                        },
                    }),
                ),
                Ok(None) => {}
                Err(error) => {
                    println!(
                        "{}",
                        json!({
                            "event": "merged_root_materialize_error",
                            "error": format!("{error:#}"),
                        })
                    );
                    return;
                }
            }
        }

        if should_refresh_projection {
            let mut refreshed_windows_cloud = false;
            if let Some(sync_root) = windows_cloud_projection_root() {
                match refresh_windows_cloud_local_projection(&config_dir, &sync_root).await {
                    Ok(report) => {
                        refreshed_windows_cloud = true;
                        println!(
                            "{}",
                            json!({
                                "event": "windows_cloud_projection_refreshed",
                                "trigger": projection_event,
                                "root": sync_root.display().to_string(),
                                "entry_count": report.entry_count,
                                "removed_paths": report.removed_paths,
                                "changed_paths": report.changed_paths,
                            })
                        );
                    }
                    Err(error) => println!(
                        "{}",
                        json!({
                            "event": "windows_cloud_projection_refresh_error",
                            "trigger": projection_event,
                            "root": sync_root.display().to_string(),
                            "error": format!("{error:#}"),
                        })
                    ),
                }
            }
            if let Some(tx) = mount_refresh {
                if tx.send(projection_event).await.is_err() {
                    println!(
                        "{}",
                        json!({"event": "mount_refresh_error", "error": "mount refresh worker stopped"})
                    );
                }
                return;
            }
            if refreshed_windows_cloud {
                return;
            }
            println!(
                "{}",
                json!({"event": "mount_refresh_skipped", "reason": "no_virtual_mount"})
            );
        }
    }))
}

async fn settle_after_direct_root_state_request(
    config_dir: &Path,
    expected_projection_root_key: Option<&RootApplyFollowupKey>,
    root_cid: &str,
) -> bool {
    tokio::time::sleep(std::time::Duration::from_millis(
        ROOT_APPLY_STATE_REPLY_SETTLE_MS,
    ))
    .await;
    if root_apply_followup_is_stale(config_dir, expected_projection_root_key) {
        println!(
            "{}",
            json!({
                "event": "root_apply_followup_skipped_stale",
                "root_cid": root_cid,
            })
        );
        true
    } else {
        false
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn enqueue_root_apply_followup(
    config_dir: PathBuf,
    config: AppConfig,
    root_cid_to_pull: Option<String>,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    should_refresh_projection: bool,
    projection_event: &'static str,
    mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
    daemon_tasks: &DaemonTaskSet,
) -> bool {
    let Some(task) = spawn_root_apply_followup(
        config_dir,
        config.clone(),
        root_cid_to_pull.clone(),
        fips_blocks,
        should_refresh_projection,
        projection_event,
        mount_refresh,
    ) else {
        return false;
    };
    let exact_key = root_apply_followup_key(
        &config,
        root_cid_to_pull.as_deref(),
        should_refresh_projection,
    )
    .map(|key| format!("root_apply_followup:{key:?}"));
    let group_key = root_apply_followup_queue_key(
        &config,
        root_cid_to_pull.as_deref(),
        should_refresh_projection,
    )
    .map(|key| format!("root_apply_followup_group:{key:?}"));

    let enqueued = match (exact_key.clone(), group_key.clone()) {
        (Some(key), Some(group)) => daemon_tasks.push_keyed_replacing_group(key, group, task),
        (Some(key), None) | (None, Some(key)) => daemon_tasks.push_keyed(key, task),
        (None, None) => {
            daemon_tasks.push(task);
            return true;
        }
    };

    if enqueued {
        true
    } else {
        println!(
            "{}",
            json!({
                "event": "root_apply_followup_coalesced",
                "root_cid": root_cid_to_pull,
                "key": exact_key,
                "group": group_key,
            })
        );
        false
    }
}

async fn materialize_primary_merged_root_for_followup(
    config_dir: &Path,
) -> Result<Option<iris_drive_core::ImportReport>> {
    let Some(_config_lock) =
        ConfigMutationLock::acquire_for_background(config_dir, || false).await?
    else {
        return Ok(None);
    };
    let mut daemon =
        iris_drive_core::Daemon::open(config_dir).context("opening daemon to materialize merge")?;
    daemon
        .materialize_primary_merged_root()
        .await
        .context("materializing merged root")
}

fn root_cid_belongs_to_peer(config: &AppConfig, root_cid: &str) -> bool {
    let Some(account) = config.profile.as_ref() else {
        return false;
    };
    config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .is_some_and(|drive| {
            drive
                .app_key_roots
                .iter()
                .any(|(device, root)| device != &account.app_key_pubkey && root.root_cid == root_cid)
        })
}

#[allow(clippy::too_many_lines)]
pub(crate) async fn pull_blocks_for_root(
    config_dir: &std::path::Path,
    config: &AppConfig,
    root_cid_str: &str,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<()> {
    let cid =
        Cid::parse(root_cid_str).with_context(|| format!("parsing root cid {root_cid_str}"))?;
    let mut attempted = false;
    let mut fips_had_peers = false;
    let mut errors = Vec::new();
    if let Some(sync) = fips_blocks {
        let connected_peers = sync.connected_peer_ids().await;
        let mesh_peers = sync.mesh_peer_ids().await;
        fips_had_peers = !connected_peers.is_empty() || !mesh_peers.is_empty();
        if connected_peers.is_empty() && mesh_peers.is_empty() {
            println!(
                "{}",
                json!({
                    "event": "fips_download_skipped",
                    "root_cid": root_cid_str,
                    "reason": "no_connected_peers",
                })
            );
        } else {
            attempted = true;
            match download_tree_over_fips_with_retry(sync, &cid, fips_download_policy(config)).await
            {
                Ok(report) => {
                    record_block_sync(config_dir, root_cid_str, "fips", &report);
                    println!(
                        "{}",
                        json!({
                            "event": "fips_downloaded",
                            "root_cid": root_cid_str,
                            "report": download_report_json(&report),
                        })
                    );
                    return Ok(());
                }
                Err(error) => {
                    let error = format!("{error:#}");
                    errors.push(format!("fips: {error}"));
                    println!(
                        "{}",
                        json!({
                            "event": "fips_download_error",
                            "root_cid": root_cid_str,
                            "error": error,
                            "connected_peers": connected_peers,
                            "mesh_peers": mesh_peers,
                        })
                    );
                }
            }
        }
    }

    if should_try_blossom_download(config, attempted, fips_had_peers) {
        attempted = true;
        match download_roots_over_blossom(config_dir, config, &[root_cid_str.to_string()]).await {
            Ok(report) => {
                record_block_sync(config_dir, root_cid_str, "blossom", &report);
                println!(
                    "{}",
                    json!({
                        "event": "blossom_downloaded",
                        "root_cid": root_cid_str,
                        "report": download_report_json(&report),
                    })
                );
                return Ok(());
            }
            Err(error) => {
                let error = error.to_string();
                errors.push(format!("blossom: {error}"));
                println!(
                    "{}",
                    json!({
                        "event": "blossom_download_error",
                        "root_cid": root_cid_str,
                        "error": error,
                    })
                );
            }
        }
    } else if !config.blossom_servers.is_empty() && attempted && fips_had_peers {
        println!(
            "{}",
            json!({
                "event": "blossom_download_skipped",
                "root_cid": root_cid_str,
                "reason": "fips_peers_available",
            })
        );
    }

    if attempted {
        Err(anyhow::anyhow!(
            "all block download transports failed for {root_cid_str}: {}",
            errors.join("; ")
        ))
    } else {
        Err(anyhow::anyhow!(
            "no block download transport available for {root_cid_str}"
        ))
    }
}

async fn request_latest_direct_root_state(
    config: &AppConfig,
    fips_blocks: Option<&FsFipsBlockSync>,
    projection_event: &'static str,
    bypass_throttle: bool,
) {
    let Some(sync) = fips_blocks else {
        return;
    };
    let Some(root_scope_id) = config.profile.as_ref().map(ProfileState::root_scope_id) else {
        return;
    };
    let mut visible_peers = sync
        .connected_peer_ids()
        .await
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>();
    visible_peers.extend(sync.mesh_peer_ids().await);
    if !should_publish_direct_root_state_request(
        &root_scope_id,
        visible_peers.iter().map(String::as_str),
        bypass_throttle,
    ) {
        println!(
            "{}",
            json!({
                "event": "direct_root_state_request_throttled",
                "trigger": projection_event,
                "root_scope_id": root_scope_id,
                "visible_peers": visible_peers.len(),
            })
        );
        return;
    }
    let bytes = match iris_drive_core::encode_direct_root_state_request_frame(&root_scope_id) {
        Ok(bytes) => bytes,
        Err(error) => {
            println!(
                "{}",
                json!({
                    "event": "direct_root_state_request_error",
                    "trigger": projection_event,
                    "error": format!("{error:#}"),
                })
            );
            return;
        }
    };
    let selected_peers = sync.authorized_peer_ids().await.len();
    match sync
        .broadcast_app_message(iris_drive_core::DIRECT_ROOT_APP_TOPIC, bytes.clone())
        .await
    {
        Ok(sent_peers) => println!(
            "{}",
            json!({
                "event": "direct_root_state_request_publish",
                "trigger": projection_event,
                "root_scope_id": root_scope_id.clone(),
                "selected_peers": selected_peers,
                "visible_peers": visible_peers.len(),
                "sent_peers": sent_peers,
            })
        ),
        Err(error) => println!(
            "{}",
            json!({
                "event": "direct_root_state_request_error",
                "trigger": projection_event,
                "root_scope_id": root_scope_id.clone(),
                "selected_peers": selected_peers,
                "visible_peers": visible_peers.len(),
                "error": format!("{error:#}"),
            })
        ),
    }
    let stream = iris_drive_core::direct_root_mesh_stream(&root_scope_id);
    let seq = direct_root_followup_mesh_seq();
    let publish_stats = sync.publish_mesh_pubsub(stream.clone(), seq, bytes).await;
    println!(
        "{}",
        json!({
            "event": "direct_root_state_request_mesh_publish",
            "trigger": projection_event,
            "stream": stream,
            "seq": seq,
            "root_scope_id": root_scope_id,
            "selected_peers": publish_stats.selected_peers,
            "sent_peers": publish_stats.sent_peers,
            "sent_bytes": publish_stats.sent_bytes,
        })
    );
}

fn direct_root_followup_mesh_seq() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_millis().try_into().unwrap_or(u64::MAX)
        })
}

fn should_publish_direct_root_state_request<'a>(
    root_scope_id: &str,
    visible_peers: impl IntoIterator<Item = &'a str>,
    bypass_throttle: bool,
) -> bool {
    if bypass_throttle {
        return true;
    }
    let throttle = DIRECT_ROOT_STATE_REQUEST_THROTTLE
        .get_or_init(|| std::sync::Mutex::new(std::collections::BTreeMap::new()));
    let Ok(mut throttle) = throttle.lock() else {
        return true;
    };
    let now = std::time::Instant::now();
    let mut throttle_keys = visible_peers
        .into_iter()
        .filter(|peer| !peer.is_empty())
        .map(|peer| format!("request:{peer}:{root_scope_id}"))
        .collect::<Vec<_>>();
    if throttle_keys.is_empty() {
        throttle_keys.push(format!("request:*:{root_scope_id}"));
    }
    let interval = std::time::Duration::from_secs(DIRECT_ROOT_STATE_REQUEST_MIN_INTERVAL_SECS);
    if throttle_keys.iter().all(|key| {
        throttle
            .get(key)
            .is_some_and(|last| now.duration_since(*last) < interval)
    }) {
        return false;
    }
    for key in throttle_keys {
        throttle.insert(key, now);
    }
    true
}

fn should_try_blossom_download(
    config: &AppConfig,
    _fips_attempted: bool,
    _fips_had_peers: bool,
) -> bool {
    !config.blossom_servers.is_empty()
}

pub(crate) async fn pull_blocks_for_root_bounded(
    config_dir: &std::path::Path,
    config: &AppConfig,
    root_cid_str: &str,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> std::result::Result<(), String> {
    let timeout_secs = event_block_pull_timeout_secs(config);
    match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        pull_blocks_for_root(config_dir, config, root_cid_str, fips_blocks),
    )
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err(format!("timed out after {timeout_secs}s")),
    }
}

fn event_block_pull_retry_delays(config: &AppConfig) -> &'static [u64] {
    if config.blossom_servers.is_empty() {
        EVENT_BLOCK_PULL_RETRY_DELAYS
    } else {
        EVENT_BLOCK_PULL_WITH_BLOSSOM_RETRY_DELAYS
    }
}

fn event_block_pull_timeout_secs(config: &AppConfig) -> u64 {
    if config.blossom_servers.is_empty() {
        EVENT_BLOCK_PULL_TIMEOUT_SECS
    } else {
        FIPS_DOWNLOAD_BEFORE_BLOSSOM_ATTEMPT_TIMEOUT_SECS
            + BLOSSOM_DOWNLOAD_RETRY_DELAYS.iter().sum::<u64>()
            + EVENT_BLOCK_PULL_WITH_BLOSSOM_HEADROOM_SECS
    }
}
