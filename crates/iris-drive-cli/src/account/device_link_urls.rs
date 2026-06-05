#[allow(clippy::wildcard_imports)]
use super::*;
pub(crate) use iris_drive_core::device_link_transport::{
    encode_device_approval_request, parse_device_approval_request as decode_device_approval_request,
};

pub(crate) fn normalize_pubkey(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.starts_with("npub1") {
        let pk = PublicKey::from_bech32(trimmed).context("parsing npub")?;
        Ok(pk.to_hex())
    } else if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(trimmed.to_string())
    } else {
        Err(anyhow::anyhow!(
            "expected npub1... or 64-char hex pubkey, got {trimmed}"
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceLinkTarget {
    pub(crate) profile_id: iris_drive_core::IrisProfileId,
    pub(crate) owner_hex: String,
    pub(crate) admin_device_hex: String,
    pub(crate) link_secret: String,
}

pub(crate) fn resolve_device_approval_input(
    input: &str,
    expected_profile_id: iris_drive_core::IrisProfileId,
    explicit_label: Option<String>,
) -> Result<(String, Option<String>)> {
    if let Some(request) = decode_device_approval_request(input)? {
        if request
            .profile_id
            .is_some_and(|profile_id| profile_id != expected_profile_id)
        {
            return Err(anyhow::anyhow!(
                "AppKey-link request belongs to a different profile"
            ));
        }
        let label = explicit_label.or(request.label);
        return Ok((request.device_hex, label));
    }

    Ok((
        normalize_pubkey(input).context("parsing AppKey pubkey")?,
        explicit_label,
    ))
}

pub(crate) fn resolve_device_link_target_with_admin(
    input: &str,
    admin_device: Option<&str>,
) -> Result<DeviceLinkTarget> {
    if let Some(invite) = decode_device_link_invite(input)? {
        if admin_device.is_some() {
            return Err(anyhow::anyhow!(
                "--admin-app-key is only valid with a manual IrisProfile UUID, not an invite URL"
            ));
        }
        let admin_device_hex = invite.admin_device_hex;
        let profile_id = invite
            .profile_id
            .ok_or_else(|| anyhow::anyhow!("AppKey invite is missing IrisProfile id"))?;
        return Ok(DeviceLinkTarget {
            profile_id,
            owner_hex: admin_device_hex.clone(),
            admin_device_hex,
            link_secret: invite.link_secret,
        });
    }

    let Some(admin_device) = admin_device else {
        return Err(anyhow::anyhow!(
            "manual AppKey linking requires an IrisProfile UUID and --admin-app-key; otherwise paste an admin invite URL"
        ));
    };
    let trimmed = input.trim();
    if normalize_pubkey(trimmed).is_ok() {
        return Err(anyhow::anyhow!(
            "manual AppKey linking requires an IrisProfile UUID and --admin-app-key; otherwise paste an admin invite URL"
        ));
    }
    let profile_id = input
        .trim()
        .parse::<iris_drive_core::IrisProfileId>()
        .context("parsing IrisProfile UUID")?;
    let admin_device_hex = normalize_pubkey(admin_device).context("parsing admin AppKey pubkey")?;
    Ok(DeviceLinkTarget {
        profile_id,
        owner_hex: admin_device_hex.clone(),
        admin_device_hex,
        link_secret: String::new(),
    })
}

pub(crate) fn device_link_request_json(state: &AccountState) -> Value {
    if state.can_manage_devices()
        || state.authorization_state != iris_drive_core::DeviceAuthorizationState::AwaitingApproval
    {
        return Value::Null;
    }

    let url = encode_device_approval_request(
        state.profile_id,
        &state.device_pubkey,
        state
            .outbound_device_link_request
            .as_ref()
            .and_then(|request| {
                (!request.link_secret.trim().is_empty()).then_some(request.link_secret.as_str())
            })
            .unwrap_or(state.device_link_secret.as_str()),
        state.device_label.as_deref(),
    );

    json!({
        "url": url,
        "profile_id": state.profile_id.to_string(),
        "app_key_npub": account_npub(&state.device_pubkey),
        "label": state.device_label.as_deref(),
        "admin_app_key_npub": state
            .outbound_device_link_request
            .as_ref()
            .map(|request| account_npub(&request.admin_device_pubkey)),
        "requested_at": state
            .outbound_device_link_request
            .as_ref()
            .map(|request| request.requested_at),
        "sent_over_relay": state.outbound_device_link_request.is_some(),
        "sent_over_fips": state.outbound_device_link_request.is_some(),
    })
}

pub(crate) fn device_link_invite_json(state: &AccountState) -> Value {
    if !state.can_manage_devices() {
        return Value::Null;
    }
    let Ok(url) = iris_drive_core::device_link_invite::encode_device_link_invite(
        state.profile_id,
        &state.device_pubkey,
        &state.device_link_secret,
    ) else {
        return Value::Null;
    };
    json!({
        "url": url,
        "web_url": device_link_web_url(&url),
        "profile_id": state.profile_id.to_string(),
        "admin_app_key_npub": account_npub(&state.device_pubkey),
    })
}

pub(crate) fn inbound_device_link_requests_json(state: &AccountState) -> Vec<Value> {
    state
        .inbound_device_link_requests
        .iter()
        .map(|request| {
            json!({
                "url": encode_device_approval_request(
                    state.profile_id,
                    &request.device_pubkey,
                    &request.link_secret,
                    request.label.as_deref(),
                ),
                "profile_id": state.profile_id.to_string(),
                "app_key_npub": account_npub(&request.device_pubkey),
                "label": request.label.as_deref(),
                "requested_at": request.requested_at,
            })
        })
        .collect()
}

pub(crate) fn device_link_web_url(invite_url: &str) -> String {
    iris_drive_core::device_link_invite::device_link_invite_web_url(invite_url)
}

pub(crate) fn decode_device_link_invite(
    input: &str,
) -> Result<Option<iris_drive_core::device_link_invite::ParsedDeviceLinkInvite>> {
    iris_drive_core::device_link_invite::parse_device_link_invite(input)
}
