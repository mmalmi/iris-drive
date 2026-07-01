#[allow(clippy::wildcard_imports)]
use super::*;
pub(crate) use iris_drive_core::app_key_link_transport::{
    encode_app_key_approval_request,
    parse_app_key_approval_request as decode_app_key_approval_request,
};
use nostr_sdk::Keys;

pub(crate) fn normalize_pubkey(input: &str) -> Result<String> {
    iris_drive_core::normalize_app_key_pubkey(input)
}

pub(crate) fn resolve_app_key_approval_input(
    input: &str,
    expected_profile_id: iris_drive_core::NostrIdentityId,
    explicit_label: Option<String>,
) -> Result<(String, Option<String>)> {
    if let Some(request) = decode_app_key_approval_request(input)? {
        if request
            .profile_id
            .is_some_and(|profile_id| profile_id != expected_profile_id)
        {
            return Err(anyhow::anyhow!(
                "AppKey-link request belongs to a different profile"
            ));
        }
        let label = explicit_label.or(request.label);
        return Ok((request.app_key_hex, label));
    }

    Ok((
        normalize_pubkey(input).context("parsing AppKey pubkey")?,
        explicit_label,
    ))
}

pub(crate) fn resolve_app_key_link_target_with_admin(
    input: &str,
    admin_app_key: Option<&str>,
) -> Result<iris_drive_core::AppKeyLinkTarget> {
    iris_drive_core::resolve_app_key_link_target(input, admin_app_key)
}

fn cached_can_admin_profile(state: &ProfileState) -> bool {
    state
        .app_keys
        .as_ref()
        .is_some_and(|snapshot| snapshot.is_admin(&state.app_key_pubkey))
}

pub(crate) fn app_key_link_request_json_with_keys(state: &ProfileState, keys: &Keys) -> Value {
    app_key_link_request_json_for_admin_state(state, state.can_admin_profile(), Some(keys))
}

pub(crate) fn cached_app_key_link_request_json(state: &ProfileState) -> Value {
    app_key_link_request_json_for_admin_state(state, cached_can_admin_profile(state), None)
}

fn app_key_link_request_json_for_admin_state(
    state: &ProfileState,
    can_admin_profile: bool,
    keys: Option<&Keys>,
) -> Value {
    if can_admin_profile
        || state.authorization_state != iris_drive_core::AppKeyAuthorizationState::AwaitingApproval
    {
        return Value::Null;
    }

    let Some(pending) = state.outbound_app_key_link_request.as_ref() else {
        return Value::Null;
    };
    let url = if let Some(keys) = keys {
        let Ok(url) = encode_app_key_approval_request(
            keys,
            request_profile_id(state, pending),
            request_admin_app_key_pubkey(pending),
            state.app_key_label.as_deref(),
            pending.requested_at,
        ) else {
            return Value::Null;
        };
        url
    } else if pending.request_url.trim().is_empty() {
        return Value::Null;
    } else {
        pending.request_url.clone()
    };

    let has_network_target = !pending.admin_app_key_pubkey.trim().is_empty();
    json!({
        "url": url,
        "profile_id": state.profile_id.to_string(),
        "app_key_npub": pubkey_npub(&state.app_key_pubkey),
        "label": state.app_key_label.as_deref(),
        "admin_app_key_npub": pubkey_npub(&pending.admin_app_key_pubkey),
        "requested_at": pending.requested_at,
        "sent_over_relay": has_network_target,
        "sent_over_fips": has_network_target,
    })
}

fn request_profile_id(
    state: &ProfileState,
    pending: &iris_drive_core::profile::PendingAppKeyLinkRequest,
) -> Option<iris_drive_core::NostrIdentityId> {
    (!pending.admin_app_key_pubkey.trim().is_empty()).then_some(state.profile_id)
}

fn request_admin_app_key_pubkey(
    pending: &iris_drive_core::profile::PendingAppKeyLinkRequest,
) -> Option<&str> {
    let admin = pending.admin_app_key_pubkey.trim();
    (!admin.is_empty()).then_some(admin)
}

pub(crate) fn ensure_cached_app_key_link_request(
    config: &mut AppConfig,
    config_dir: &Path,
) -> Result<bool> {
    let Some(state) = config.profile.as_ref() else {
        return Ok(false);
    };
    if cached_can_admin_profile(state)
        || state.authorization_state != iris_drive_core::AppKeyAuthorizationState::AwaitingApproval
    {
        return Ok(false);
    }

    let pending = state.outbound_app_key_link_request.as_ref();
    let profile_id = state.profile_id;
    let admin_app_key_pubkey = pending
        .map(|pending| pending.admin_app_key_pubkey.clone())
        .unwrap_or_default();
    let requested_at = pending.map_or_else(unix_now_seconds, |pending| pending.requested_at);
    let app_key_label = state.app_key_label.clone();
    let app_key =
        iris_drive_core::AppKey::load(key_path_in(config_dir)).context("loading app key")?;
    let request_url = encode_app_key_approval_request(
        app_key.keys(),
        request_profile_id_for_admin(profile_id, &admin_app_key_pubkey),
        request_admin_app_key_pubkey_for_admin(&admin_app_key_pubkey),
        app_key_label.as_deref(),
        requested_at,
    )?;

    let Some(state) = config.profile.as_mut() else {
        return Ok(false);
    };
    let changed = if let Some(pending) = state.outbound_app_key_link_request.as_mut() {
        if pending.request_url == request_url {
            false
        } else {
            pending.request_url = request_url;
            true
        }
    } else {
        state.queue_unbound_app_key_join_request(requested_at, request_url)
    };
    if changed {
        config.save(config_path_in(config_dir))?;
    }
    Ok(changed)
}

fn request_profile_id_for_admin(
    profile_id: iris_drive_core::NostrIdentityId,
    admin_app_key_pubkey: &str,
) -> Option<iris_drive_core::NostrIdentityId> {
    (!admin_app_key_pubkey.trim().is_empty()).then_some(profile_id)
}

fn request_admin_app_key_pubkey_for_admin(admin_app_key_pubkey: &str) -> Option<&str> {
    let admin = admin_app_key_pubkey.trim();
    (!admin.is_empty()).then_some(admin)
}

pub(crate) fn app_key_link_invite_json(state: &ProfileState) -> Value {
    app_key_link_invite_json_for_admin_state(state, state.can_admin_profile())
}

pub(crate) fn cached_app_key_link_invite_json(state: &ProfileState) -> Value {
    app_key_link_invite_json_for_admin_state(state, cached_can_admin_profile(state))
}

fn app_key_link_invite_json_for_admin_state(
    state: &ProfileState,
    can_admin_profile: bool,
) -> Value {
    if !can_admin_profile {
        return Value::Null;
    }
    let Ok(invite_pubkey) = iris_drive_core::app_key_link_invite_pubkey(&state.app_key_link_secret)
    else {
        return Value::Null;
    };
    let Ok(url) = iris_drive_core::app_key_link_invite::encode_app_key_link_invite(
        state.profile_id,
        &state.app_key_pubkey,
        &invite_pubkey,
    ) else {
        return Value::Null;
    };
    json!({
        "url": url,
        "web_url": app_key_link_web_url(&url),
        "profile_id": state.profile_id.to_string(),
        "admin_app_key_npub": pubkey_npub(&state.app_key_pubkey),
    })
}

pub(crate) fn inbound_app_key_link_requests_json(state: &ProfileState) -> Vec<Value> {
    state
        .inbound_app_key_link_requests
        .iter()
        .map(|request| {
            let request_url = request.request_url.trim();
            let url = if request_url.is_empty() {
                pubkey_npub(&request.app_key_pubkey)
            } else {
                request.request_url.clone()
            };
            json!({
                "url": url,
                "profile_id": state.profile_id.to_string(),
                "app_key_npub": pubkey_npub(&request.app_key_pubkey),
                "label": request.label.as_deref(),
                "requested_at": request.requested_at,
            })
        })
        .collect()
}

pub(crate) fn app_key_link_web_url(invite_url: &str) -> String {
    iris_drive_core::app_key_link_invite::app_key_link_invite_web_url(invite_url)
}
