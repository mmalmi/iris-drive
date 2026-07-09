use super::*;

pub(super) fn app_key_link_request_url(
    state: &iris_drive_core::ProfileState,
    _config_dir: &Path,
) -> String {
    if state.can_admin_profile()
        || state.authorization_state != AppKeyAuthorizationState::AwaitingApproval
    {
        return String::new();
    }
    let Some(pending) = state.outbound_app_key_link_request.as_ref() else {
        return String::new();
    };
    if iris_drive_core::app_key_link_transport::parse_pending_app_key_approval_request(pending)
        .is_err()
    {
        return String::new();
    }
    pending.request_url.clone()
}

fn request_profile_id(
    profile_id: iris_drive_core::NostrIdentityId,
    admin_app_key_pubkey: &str,
) -> Option<iris_drive_core::NostrIdentityId> {
    (!admin_app_key_pubkey.trim().is_empty()).then_some(profile_id)
}

fn request_admin_app_key_pubkey(admin_app_key_pubkey: &str) -> Option<&str> {
    let admin = admin_app_key_pubkey.trim();
    (!admin.is_empty()).then_some(admin)
}

pub(super) fn ensure_cached_app_key_link_request_url(
    config: &mut AppConfig,
    config_dir: &Path,
) -> Result<(), String> {
    let Some(state) = config.profile.as_ref() else {
        return Ok(());
    };
    if state.can_admin_profile()
        || state.authorization_state != AppKeyAuthorizationState::AwaitingApproval
    {
        return Ok(());
    }
    let pending = state.outbound_app_key_link_request.as_ref();
    if let Some(pending) = pending
        && iris_drive_core::app_key_link_transport::app_key_approval_request_url_is_full(
            &pending.request_url,
        )
    {
        iris_drive_core::app_key_link_transport::parse_pending_app_key_approval_request(pending)
            .map_err(|error| format!("validating persisted app-key approval request: {error}"))?;
        return Ok(());
    }
    let profile_id = state.profile_id;
    let admin_app_key_pubkey = pending
        .map(|pending| pending.admin_app_key_pubkey.clone())
        .unwrap_or_default();
    let invite_pubkey = pending
        .map(|pending| pending.invite_pubkey.clone())
        .unwrap_or_default();
    let app_key_label = state.app_key_label.clone();
    let requested_at = pending.map_or_else(unix_now_seconds, |pending| pending.requested_at);
    let app_key = iris_drive_core::AppKey::load(key_path_in(config_dir))
        .map_err(|error| error.to_string())?;
    let approval_request = create_app_key_approval_request(
        app_key.keys(),
        request_profile_id(profile_id, &admin_app_key_pubkey),
        request_admin_app_key_pubkey(&admin_app_key_pubkey),
        app_key_label.as_deref(),
        requested_at,
    )
    .map_err(|error| error.to_string())?;
    let request_key_secret = approval_request.request_keys.secret_key().to_secret_hex();
    let changed = if let Some(state) = config.profile.as_mut() {
        if admin_app_key_pubkey.trim().is_empty() {
            state.queue_unbound_app_key_join_request(
                requested_at,
                approval_request.url,
                request_key_secret,
            )
        } else {
            state
                .queue_outbound_app_key_link_request(
                    admin_app_key_pubkey,
                    &invite_pubkey,
                    requested_at,
                    approval_request.url,
                    request_key_secret,
                )
                .map_err(|error| error.to_string())?
        }
    } else {
        false
    };
    if changed {
        config
            .save(config_path_in(config_dir))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(super) fn app_key_link_invite_url(state: &iris_drive_core::ProfileState) -> String {
    if !state.can_admin_profile() {
        return String::new();
    }
    let Ok(invite_pubkey) = iris_drive_core::app_key_link_invite_pubkey(&state.app_key_link_secret)
    else {
        return String::new();
    };
    iris_drive_core::app_key_link_invite::encode_app_key_link_invite(
        state.profile_id,
        &state.app_key_pubkey,
        &invite_pubkey,
    )
    .unwrap_or_default()
}
