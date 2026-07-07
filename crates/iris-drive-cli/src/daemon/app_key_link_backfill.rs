async fn backfill_pending_app_key_link_roster_ops(
    client: &nostr_sdk::Client,
    config_dir: &Path,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
    daemon_tasks: &DaemonTaskSet,
    config_cache: &mut AppConfigLoadCache,
) -> Result<Option<EventApplyOutcome>> {
    let config = load_app_config_cached(&config_path_in(config_dir), config_cache)?;
    let Some(state) = config.profile.as_ref() else {
        return Ok(None);
    };
    if state.authorization_state != iris_drive_core::AppKeyAuthorizationState::AwaitingApproval {
        return Ok(None);
    }

    let pending = state.outbound_app_key_link_request.as_ref();
    if pending.is_none_or(|pending| pending.admin_app_key_pubkey.trim().is_empty()) {
        let candidates =
            iris_drive_core::relay_sync::fetch_nostr_identity_app_key_approval_candidates(
                client,
                &state.app_key_pubkey,
                std::time::Duration::from_secs(2),
            )
            .await
            .context("fetching unbound AppKey-link approval roster candidates")?;
        return apply_pending_app_key_link_roster_candidates(
            config_dir,
            candidates,
            fips_blocks,
            mount_refresh,
            daemon_tasks,
        )
        .await;
    }

    let events = iris_drive_core::relay_sync::fetch_nostr_identity_roster_ops(
        client,
        state.profile_id,
        std::time::Duration::from_secs(2),
    )
    .await
    .context("fetching pending AppKey-link roster ops")?;

    let mut changed = false;
    let mut retryable = false;
    for event in events {
        match apply_one_event(
            client,
            config_dir,
            &event,
            fips_blocks.clone(),
            mount_refresh.clone(),
            daemon_tasks,
        )
        .await?
        {
            EventApplyOutcome::Changed => changed = true,
            EventApplyOutcome::RetryablePrerequisiteMissing => retryable = true,
            EventApplyOutcome::Unchanged => {}
        }
    }

    if changed {
        Ok(Some(EventApplyOutcome::Changed))
    } else if retryable {
        Ok(Some(EventApplyOutcome::RetryablePrerequisiteMissing))
    } else {
        Ok(Some(EventApplyOutcome::Unchanged))
    }
}

async fn apply_pending_app_key_link_roster_candidates(
    config_dir: &Path,
    candidates: Vec<iris_drive_core::relay_sync::NostrIdentityAppKeyApprovalCandidate>,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
    daemon_tasks: &DaemonTaskSet,
) -> Result<Option<EventApplyOutcome>> {
    let mut changed = false;
    for candidate in candidates {
        match apply_pending_app_key_link_roster_candidate(
            config_dir,
            candidate,
            fips_blocks.clone(),
            mount_refresh.clone(),
            daemon_tasks,
        )
        .await?
        {
            EventApplyOutcome::Changed => changed = true,
            EventApplyOutcome::RetryablePrerequisiteMissing => {
                return Ok(Some(EventApplyOutcome::RetryablePrerequisiteMissing));
            }
            EventApplyOutcome::Unchanged => {}
        }
        if changed {
            return Ok(Some(EventApplyOutcome::Changed));
        }
    }
    Ok(Some(EventApplyOutcome::Unchanged))
}

async fn apply_pending_app_key_link_roster_candidate(
    config_dir: &Path,
    candidate: iris_drive_core::relay_sync::NostrIdentityAppKeyApprovalCandidate,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
    daemon_tasks: &DaemonTaskSet,
) -> Result<EventApplyOutcome> {
    let config_lock = ConfigMutationLock::acquire(config_dir).await?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(state) = config.profile.as_ref() else {
        return Ok(EventApplyOutcome::Unchanged);
    };
    if state.authorization_state != iris_drive_core::AppKeyAuthorizationState::AwaitingApproval {
        return Ok(EventApplyOutcome::Unchanged);
    }
    if state.app_key_pubkey != candidate.app_key_pubkey {
        return Ok(EventApplyOutcome::Unchanged);
    }
    if state
        .outbound_app_key_link_request
        .as_ref()
        .is_some_and(|pending| {
            let expected_admin = pending.admin_app_key_pubkey.trim();
            !expected_admin.is_empty() && expected_admin != candidate.admin_app_key_pubkey
        })
    {
        return Ok(EventApplyOutcome::Unchanged);
    }
    if state.outbound_app_key_link_request.is_none() {
        let requested_at = u64::try_from(unix_now()).unwrap_or(0);
        if let Some(state) = config.profile.as_mut() {
            state.queue_unbound_app_key_join_request(
                requested_at,
                format!(
                    "{}?app_key={}",
                    iris_drive_core::app_key_link_transport::APP_KEY_APPROVAL_COMPACT_PREFIX,
                    candidate.app_key_pubkey
                ),
            );
        }
    }

    let frame = iris_drive_core::app_key_link_transport::AppKeyLinkRosterFrame {
        schema: 1,
        profile_id: candidate.profile_id,
        admin_app_key_pubkey: candidate.admin_app_key_pubkey.clone(),
        profile_roster_ops: candidate.profile_roster_ops,
        sent_at: u64::try_from(unix_now()).unwrap_or(0),
    };
    let outcome = iris_drive_core::relay_sync::apply_app_key_link_roster_frame(
        &mut config,
        &frame,
        &candidate.admin_app_key_pubkey,
    )?;
    let changed = matches!(
        outcome,
        iris_drive_core::relay_sync::AppKeyLinkRosterApply::Applied(decision)
            if decision != iris_drive_core::ApplyDecision::Rejected
    );
    if changed {
        config.save(config_path_in(config_dir))?;
    }
    emit_daemon_status_event(
        config_dir,
        json!({
            "event": "app_key_link_roster_backfill",
            "admin_app_key_npub": pubkey_npub(&candidate.admin_app_key_pubkey),
            "profile_id": candidate.profile_id.to_string(),
            "outcome": format!("{outcome:?}"),
        }),
    );
    drop(config_lock);
    if changed {
        if let Some(sync) = fips_blocks.as_deref() {
            sync.refresh_authorized_peers(&config).await;
        }
        enqueue_root_apply_followup(
            config_dir.to_path_buf(),
            config,
            None,
            fips_blocks,
            true,
            "app_key_link_roster_backfill",
            mount_refresh,
            daemon_tasks,
        );
        Ok(EventApplyOutcome::Changed)
    } else {
        Ok(EventApplyOutcome::Unchanged)
    }
}
