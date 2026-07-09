use super::*;

pub(super) fn app_key_link_request_url(
    state: &iris_drive_core::ProfileState,
    config_dir: &Path,
) -> String {
    if state.can_admin_profile()
        || state.authorization_state != AppKeyAuthorizationState::AwaitingApproval
    {
        return String::new();
    }
    let Some(pending) = state.outbound_app_key_link_request.as_ref() else {
        return String::new();
    };
    if iris_drive_core::app_key_link_transport::app_key_approval_request_url_is_full(
        &pending.request_url,
    ) {
        return pending.request_url.clone();
    }
    let Ok(app_key) = iris_drive_core::AppKey::load(key_path_in(config_dir)) else {
        return String::new();
    };
    encode_app_key_approval_request(
        app_key.keys(),
        pending_request_profile_id(state, pending),
        pending_admin_app_key_pubkey(pending),
        state.app_key_label.as_deref(),
        pending.requested_at,
    )
    .unwrap_or_default()
}

fn pending_request_profile_id(
    state: &iris_drive_core::ProfileState,
    pending: &iris_drive_core::profile::PendingAppKeyLinkRequest,
) -> Option<iris_drive_core::NostrIdentityId> {
    (!pending.admin_app_key_pubkey.trim().is_empty()).then_some(state.profile_id)
}

fn pending_admin_app_key_pubkey(
    pending: &iris_drive_core::profile::PendingAppKeyLinkRequest,
) -> Option<&str> {
    let admin = pending.admin_app_key_pubkey.trim();
    (!admin.is_empty()).then_some(admin)
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
    if pending.is_some_and(|pending| {
        iris_drive_core::app_key_link_transport::app_key_approval_request_url_is_full(
            &pending.request_url,
        )
    }) {
        return Ok(());
    }
    let profile_id = state.profile_id;
    let admin_app_key_pubkey = pending
        .map(|pending| pending.admin_app_key_pubkey.clone())
        .unwrap_or_default();
    let app_key_label = state.app_key_label.clone();
    let requested_at = pending.map_or_else(unix_now_seconds, |pending| pending.requested_at);
    let app_key = iris_drive_core::AppKey::load(key_path_in(config_dir))
        .map_err(|error| error.to_string())?;
    let request_url = encode_app_key_approval_request(
        app_key.keys(),
        request_profile_id(profile_id, &admin_app_key_pubkey),
        request_admin_app_key_pubkey(&admin_app_key_pubkey),
        app_key_label.as_deref(),
        requested_at,
    )
    .map_err(|error| error.to_string())?;
    let changed = if let Some(pending) = config
        .profile
        .as_mut()
        .and_then(|state| state.outbound_app_key_link_request.as_mut())
    {
        if pending.request_url == request_url {
            false
        } else {
            pending.request_url = request_url;
            true
        }
    } else if let Some(state) = config.profile.as_mut() {
        state.queue_unbound_app_key_join_request(requested_at, request_url)
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
