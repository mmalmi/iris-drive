/// The short event-driven attempts have already failed. Give this exact current
/// head a bounded chance to catch a late source without waiting for maintenance.
/// The daemon owns one task per source head and cancels it on supersession.
async fn recover_pending_peer_blocks(
    config_dir: &Path,
    initial: &AppConfig,
    root_cid: &str,
    expected: Option<&RootApplyFollowupKey>,
    fips: Option<&FsFipsBlockSync>,
) -> bool {
    let (Some(initial_profile), Some(RootApplyFollowupKey::AppKeyRoots(expected_roots))) =
        (initial.profile.as_ref(), expected)
    else {
        return false;
    };
    if !is_current_peer_block_repair(initial, initial_profile, root_cid, expected_roots) {
        return false;
    }
    for delay in [5, 15, 45] {
        tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        let Ok(config) = AppConfig::load_or_default(config_path_in(config_dir)) else {
            return false;
        };
        if !is_current_peer_block_repair(&config, initial_profile, root_cid, expected_roots) {
            return false;
        }
        if root_has_successful_block_sync(config_dir, root_cid) {
            return true;
        }
        match pull_blocks_for_root_bounded(config_dir, &config, root_cid, fips).await {
            Ok(()) => return true,
            Err(error) => record_block_sync_error(config_dir, root_cid, &error),
        }
    }
    false
}

fn is_current_peer_block_repair(
    config: &AppConfig,
    initial: &ProfileState,
    cid: &str,
    expected: &[(String, String, u64)],
) -> bool {
    let Some(profile) = config.profile.as_ref() else {
        return false;
    };
    if profile.profile_id != initial.profile_id
        || profile.app_key_pubkey != initial.app_key_pubkey
        || profile.authorization_state != iris_drive_core::AppKeyAuthorizationState::Authorized
    {
        return false;
    }
    let Some(drive) = config.drive(PRIMARY_DRIVE_ID) else {
        return false;
    };
    drive
        .active_app_key_roots(Some(profile))
        .into_iter()
        .any(|(author, head)| {
            author != &profile.app_key_pubkey
                && !head.local_only
                && head.root_cid == cid
                && expected.iter().any(|(key, root, seq)| {
                    key == author && root == cid && *seq == head.app_key_seq
                })
        })
}
