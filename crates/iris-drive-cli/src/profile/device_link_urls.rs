#[allow(clippy::wildcard_imports)]
use super::*;
pub(crate) use iris_drive_core::device_link_transport::{
    encode_app_key_approval_request,
    parse_app_key_approval_request as decode_app_key_approval_request,
};

pub(crate) fn normalize_pubkey(input: &str) -> Result<String> {
    iris_drive_core::normalize_app_key_pubkey(input)
}

pub(crate) fn resolve_app_key_approval_input(
    input: &str,
    expected_profile_id: iris_drive_core::IrisProfileId,
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

pub(crate) fn app_key_link_request_json(state: &ProfileState) -> Value {
    if state.can_admin_profile()
        || state.authorization_state != iris_drive_core::AppKeyAuthorizationState::AwaitingApproval
    {
        return Value::Null;
    }

    let url = encode_app_key_approval_request(
        state.profile_id,
        &state.app_key_pubkey,
        state
            .outbound_app_key_link_request
            .as_ref()
            .and_then(|request| {
                (!request.link_secret.trim().is_empty()).then_some(request.link_secret.as_str())
            })
            .unwrap_or(state.app_key_link_secret.as_str()),
        state.app_key_label.as_deref(),
    );

    json!({
        "url": url,
        "profile_id": state.profile_id.to_string(),
        "app_key_npub": pubkey_npub(&state.app_key_pubkey),
        "label": state.app_key_label.as_deref(),
        "admin_app_key_npub": state
            .outbound_app_key_link_request
            .as_ref()
            .map(|request| pubkey_npub(&request.admin_app_key_pubkey)),
        "requested_at": state
            .outbound_app_key_link_request
            .as_ref()
            .map(|request| request.requested_at),
        "sent_over_relay": state.outbound_app_key_link_request.is_some(),
        "sent_over_fips": state.outbound_app_key_link_request.is_some(),
    })
}

pub(crate) fn app_key_link_invite_json(state: &ProfileState) -> Value {
    if !state.can_admin_profile() {
        return Value::Null;
    }
    let Ok(url) = iris_drive_core::app_key_link_invite::encode_app_key_link_invite(
        state.profile_id,
        &state.app_key_pubkey,
        &state.app_key_link_secret,
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
                "url": encode_app_key_approval_request(
                    state.profile_id,
                    &request.app_key_pubkey,
                    &request.link_secret,
                    request.label.as_deref(),
                ),
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
