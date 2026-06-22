#[allow(clippy::too_many_lines)]
pub(crate) async fn apply_one_event(
    _client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    event: &nostr_sdk::Event,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
    daemon_tasks: &DaemonTaskSet,
) -> Result<()> {
    use iris_drive_core::relay_sync;
    let _config_lock = ConfigMutationLock::acquire(config_dir).await?;
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
        if matches!(outcome, relay_sync::AppKeyLinkRequestApply::Recorded) {
            config.save(config_path_in(config_dir))?;
        }
        return Ok(());
    } else if iris_drive_core::is_iris_profile_roster_op_event_coordinate(event) {
        let outcome = relay_sync::apply_remote_iris_profile_roster_op_event(&mut config, event)?;
        emit_daemon_status_event(
            config_dir,
            json!({
                "event": "iris_profile_roster_op",
                "event_id": event.id.to_hex(),
                "author": pubkey_npub(&event.pubkey.to_hex()),
                "outcome": format!("{outcome:?}"),
            }),
        );
        if let Some(sync) = fips_blocks.as_deref() {
            sync.refresh_authorized_peers(&config).await;
        }
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
        }
        if let Some(sync) = fips_blocks.as_deref() {
            sync.refresh_authorized_peers(&config).await;
        }
    } else if iris_drive_core::nostr_events::is_drive_root_event_coordinate(event) {
        let device = iris_drive_core::identity::AppKey::load(key_path_in(config_dir))
            .context("loading app key")?;
        let parsed =
            iris_drive_core::nostr_events::parse_drive_root_event_for_device(event, device.keys())
                .ok();
        let outcome =
            relay_sync::apply_remote_drive_root_event(&mut config, event, Some(device.keys()))?;
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
        config.save(config_path_in(config_dir))?;
        if let Some(sync) = fips_blocks.as_deref() {
            sync.refresh_authorized_peers(&config).await;
        }

        if let Some(task) = spawn_root_apply_followup(
            config_dir.to_path_buf(),
            config.clone(),
            root_cid_to_pull,
            fips_blocks,
            followup.refresh_projection,
            "projected_drive_root",
            mount_refresh,
        ) {
            daemon_tasks.push(task);
        }
        return Ok(());
    } else if kind == iris_drive_core::nostr_events::KIND_HASHTREE_ROOT {
        let Some(account_state) = config.profile.clone() else {
            return Ok(());
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
        return Ok(());
    }
    config.save(config_path_in(config_dir))?;
    Ok(())
}

pub(crate) fn apply_files_root_event(
    config_dir: &std::path::Path,
    event: &nostr_sdk::Event,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
    daemon_tasks: &DaemonTaskSet,
    config: &mut AppConfig,
    account_state: ProfileState,
) -> Result<()> {
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
        return Ok(());
    }
    let account = Profile::load(account_state, config_dir).context("loading profile")?;
    let outcome =
        relay_sync::apply_remote_files_root_event(config, event, Some(account.app_key.keys()))?;
    let was_applied = matches!(outcome, relay_sync::FilesRootApply::Applied);
    let tree_name = event
        .tags
        .identifier()
        .map(str::to_owned)
        .unwrap_or_else(|| iris_drive_core::PRIMARY_DRIVE_ID.to_string());
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
    config.save(config_path_in(config_dir))?;
    if let Some(task) = spawn_root_apply_followup(
        config_dir.to_path_buf(),
        config.clone(),
        root_cid_to_pull,
        fips_blocks,
        was_applied && tree_name == iris_drive_core::PRIMARY_DRIVE_ID,
        "projected_files_root",
        mount_refresh,
    ) {
        daemon_tasks.push(task);
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
struct DriveRootFollowupPlan {
    pull_blocks: bool,
    refresh_projection: bool,
}

fn drive_root_followup_plan(
    was_applied: bool,
    stale_current_root: bool,
    root_blocks_already_synced: bool,
) -> DriveRootFollowupPlan {
    DriveRootFollowupPlan {
        pull_blocks: was_applied || stale_current_root,
        refresh_projection: was_applied || (stale_current_root && !root_blocks_already_synced),
    }
}

fn root_has_successful_block_sync(config_dir: &Path, root_cid: &str) -> bool {
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

    Some(tokio::spawn(async move {
        if let Some(root_cid) = root_cid_to_pull {
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
                    }
                }
            }
            if let Some(error) = last_error {
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
            match materialize_primary_merged_root_for_followup(&config_dir).await {
                Ok(Some(report)) => println!(
                    "{}",
                    json!({
                        "event": "merged_root_materialized",
                        "root_cid": report.root_cid,
                        "file_count": report.file_count,
                        "top_level_entries": report.top_level_entries,
                    })
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
            if root_apply_followup_is_stale(&config_dir, expected_projection_root_key.as_ref()) {
                println!(
                    "{}",
                    json!({
                        "event": "root_apply_projection_refresh_skipped_stale",
                        "root_key": root_apply_followup_key_label(expected_projection_root_key.as_ref()),
                    })
                );
                return;
            }
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

async fn materialize_primary_merged_root_for_followup(
    config_dir: &Path,
) -> Result<Option<iris_drive_core::ImportReport>> {
    let _config_lock = ConfigMutationLock::acquire(config_dir).await?;
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
