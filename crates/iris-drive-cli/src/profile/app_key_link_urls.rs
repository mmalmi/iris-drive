#[allow(clippy::wildcard_imports)]
use super::*;
pub(crate) use iris_drive_core::app_key_link_transport::create_app_key_approval_bootstrap;
use nostr_sdk::Keys;

pub(crate) fn normalize_pubkey(input: &str) -> Result<String> {
    iris_drive_core::normalize_app_key_pubkey(input)
}

pub(crate) fn resolve_app_key_approval_input(
    config: &AppConfig,
    input: &str,
    expected_profile_id: iris_drive_core::NostrIdentityId,
    expected_admin_app_key_pubkey: &str,
    explicit_label: Option<String>,
) -> Result<(String, Option<String>)> {
    let _ = (expected_profile_id, expected_admin_app_key_pubkey);
    let bootstrap = decode_app_key_approval_bootstrap(config, input)?;
    let app_key_pubkey = nostr_sdk::PublicKey::parse(&bootstrap.device_app_key_npub)
        .context("parsing approval bootstrap device AppKey")?
        .to_hex();
    let label = explicit_label.or(bootstrap.label);
    Ok((app_key_pubkey, label))
}

pub(crate) fn decode_app_key_approval_bootstrap(
    config: &AppConfig,
    input: &str,
) -> Result<iris_drive_core::app_key_link_transport::AppKeyApprovalBootstrap> {
    config
        .profile
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("profile admin is required to approve devices"))?;
    iris_drive_core::app_key_link_transport::parse_app_key_approval_bootstrap(input)?
        .context("app-key approval bootstrap is missing or invalid")
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

pub(crate) fn app_key_link_request_json_with_keys(state: &ProfileState, _keys: &Keys) -> Value {
    app_key_link_request_json_for_admin_state(state, state.can_admin_profile())
}

pub(crate) fn queue_cached_app_key_link_request(
    state: &mut ProfileState,
    keys: &Keys,
    admin_app_key_pubkey: String,
    invite_pubkey: &str,
    requested_at: u64,
) -> Result<()> {
    let approval_request = create_app_key_approval_bootstrap(keys, state.app_key_label.as_deref())?;
    state
        .queue_outbound_app_key_link_request(
            admin_app_key_pubkey,
            invite_pubkey,
            requested_at,
            approval_request.url,
            approval_request.request_keys.secret_key().to_secret_hex(),
        )
        .context("queueing app-key link request")?;
    Ok(())
}

pub(crate) fn cached_app_key_link_request_json(state: &ProfileState) -> Value {
    app_key_link_request_json_for_admin_state(state, cached_can_admin_profile(state))
}

fn app_key_link_request_json_for_admin_state(
    state: &ProfileState,
    can_admin_profile: bool,
) -> Value {
    if can_admin_profile
        || state.authorization_state != iris_drive_core::AppKeyAuthorizationState::AwaitingApproval
    {
        return Value::Null;
    }

    let Some(pending) = state.outbound_app_key_link_request.as_ref() else {
        return Value::Null;
    };
    if iris_drive_core::app_key_link_transport::parse_pending_app_key_approval_bootstrap(pending)
        .is_err()
    {
        return Value::Null;
    }
    let url = pending.request_url.clone();

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
    if let Some(pending) = pending
        && iris_drive_core::app_key_link_transport::parse_pending_app_key_approval_bootstrap(
            pending,
        )
        .is_ok()
    {
        return Ok(false);
    }
    let requested_at = pending.map_or_else(unix_now_seconds, |pending| pending.requested_at);
    let app_key_label = state.app_key_label.clone();
    let app_key =
        iris_drive_core::AppKey::load(key_path_in(config_dir)).context("loading app key")?;
    let approval_request =
        create_app_key_approval_bootstrap(app_key.keys(), app_key_label.as_deref())?;

    let Some(state) = config.profile.as_mut() else {
        return Ok(false);
    };
    let changed = state.queue_unbound_app_key_join_request(
        requested_at,
        approval_request.url,
        approval_request.request_keys.secret_key().to_secret_hex(),
    );
    if changed {
        config.save(config_path_in(config_dir))?;
    }
    Ok(changed)
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
            json!({
                "url": request.request_url,
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
