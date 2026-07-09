use super::*;

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) fn apply_native_app_key_link_roster_candidate_to_config(
    config: &mut AppConfig,
    candidate: &iris_drive_core::relay_sync::NostrIdentityAppKeyApprovalCandidate,
) -> Result<NativeAppKeyLinkRelayEventApply, String> {
    let Some(state) = config.profile.as_ref() else {
        return Ok(NativeAppKeyLinkRelayEventApply::Ignored);
    };
    if state.app_key_pubkey != candidate.app_key_pubkey {
        return Ok(NativeAppKeyLinkRelayEventApply::Ignored);
    }
    if state
        .outbound_app_key_link_request
        .as_ref()
        .is_some_and(|pending| {
            let expected_admin = pending.admin_app_key_pubkey.trim();
            !expected_admin.is_empty() && expected_admin != candidate.admin_app_key_pubkey
        })
    {
        return Ok(NativeAppKeyLinkRelayEventApply::Ignored);
    }
    if state.outbound_app_key_link_request.is_none() {
        let requested_at = unix_now_seconds();
        if let Some(state) = config.profile.as_mut() {
            state.queue_unbound_app_key_join_request(requested_at, String::new());
        }
    }
    let frame = iris_drive_core::app_key_link_transport::AppKeyLinkRosterFrame {
        schema: 1,
        profile_id: candidate.profile_id,
        admin_app_key_pubkey: candidate.admin_app_key_pubkey.clone(),
        profile_roster_ops: candidate.profile_roster_ops.clone(),
        sent_at: unix_now_seconds(),
    };
    let outcome = iris_drive_core::relay_sync::apply_app_key_link_roster_frame(
        config,
        &frame,
        &candidate.admin_app_key_pubkey,
    )
    .map_err(|error| format!("applying app-key approval roster candidate: {error}"))?;
    Ok(match outcome {
        iris_drive_core::relay_sync::AppKeyLinkRosterApply::Current => {
            NativeAppKeyLinkRelayEventApply::Current
        }
        iris_drive_core::relay_sync::AppKeyLinkRosterApply::Applied(decision)
            if decision != iris_drive_core::ApplyDecision::Rejected =>
        {
            NativeAppKeyLinkRelayEventApply::AppliedRoster
        }
        iris_drive_core::relay_sync::AppKeyLinkRosterApply::Applied(_)
        | iris_drive_core::relay_sync::AppKeyLinkRosterApply::Ignored => {
            NativeAppKeyLinkRelayEventApply::Ignored
        }
    })
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
pub(super) async fn backfill_native_unbound_app_key_approval_candidates(
    config_dir: &Path,
    relay_client: &nostr_sdk::Client,
    sync: &iris_drive_core::FsFipsBlockSync,
    state: &iris_drive_core::ProfileState,
) -> Result<bool, String> {
    if state.authorization_state != AppKeyAuthorizationState::AwaitingApproval {
        return Ok(false);
    }
    if state
        .outbound_app_key_link_request
        .as_ref()
        .is_some_and(|pending| !pending.admin_app_key_pubkey.trim().is_empty())
    {
        return Ok(false);
    }

    let candidates = iris_drive_core::relay_sync::fetch_nostr_identity_app_key_approval_candidates(
        relay_client,
        &state.app_key_pubkey,
        std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
    )
    .await
    .map_err(|error| format!("fetching unbound app-key approval candidates: {error}"))?;
    for candidate in candidates {
        let mut config = AppConfig::load_or_default(config_path_in(config_dir))
            .map_err(|error| format!("loading config: {error}"))?;
        let outcome =
            apply_native_app_key_link_roster_candidate_to_config(&mut config, &candidate)?;
        if outcome != NativeAppKeyLinkRelayEventApply::AppliedRoster {
            continue;
        }
        config
            .save(config_path_in(config_dir))
            .map_err(|error| format!("saving config: {error}"))?;
        sync.refresh_authorized_peers(&config).await;
        match iris_drive_core::sync_once_with_fips(
            config_dir,
            &[],
            std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
            Some(sync),
        )
        .await
        {
            Ok(report) => tracing::debug!(
                drive_root_events_applied = report.drive_root_events_applied,
                fips_download = report.fips_download.is_some(),
                blossom_download = report.blossom_download.is_some(),
                admin_app_key_npub = pubkey_npub(&candidate.admin_app_key_pubkey),
                "synced drive roots after native unbound app-key approval"
            ),
            Err(error) => tracing::warn!(
                error = %error,
                admin_app_key_npub = pubkey_npub(&candidate.admin_app_key_pubkey),
                "syncing drive roots after native unbound app-key approval failed"
            ),
        }
        return Ok(true);
    }
    Ok(false)
}
