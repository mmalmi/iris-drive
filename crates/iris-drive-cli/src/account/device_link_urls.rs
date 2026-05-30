#[allow(clippy::wildcard_imports)]
use super::*;

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
pub(crate) struct DeviceApprovalRequest {
    pub(crate) owner_hex: String,
    pub(crate) device_hex: String,
    pub(crate) link_secret: String,
    pub(crate) label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DeviceLinkTarget {
    pub(crate) owner_hex: String,
    pub(crate) admin_device_hex: Option<String>,
    pub(crate) link_secret: String,
}

pub(crate) fn resolve_device_approval_input(
    input: &str,
    expected_owner_hex: &str,
    explicit_label: Option<String>,
) -> Result<(String, Option<String>)> {
    if let Some(request) = decode_device_approval_request(input)? {
        if request.owner_hex != expected_owner_hex {
            return Err(anyhow::anyhow!(
                "device request belongs to a different owner"
            ));
        }
        let label = explicit_label.or(request.label);
        return Ok((request.device_hex, label));
    }

    Ok((
        normalize_pubkey(input).context("parsing device pubkey")?,
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
                "--admin-device is only valid with a manual owner pubkey, not an invite URL"
            ));
        }
        return Ok(DeviceLinkTarget {
            owner_hex: invite.owner_hex,
            admin_device_hex: Some(invite.admin_device_hex),
            link_secret: invite.link_secret,
        });
    }

    let admin_device_hex = admin_device
        .map(|admin| normalize_pubkey(admin).context("parsing admin device pubkey"))
        .transpose()?;
    Ok(DeviceLinkTarget {
        owner_hex: normalize_pubkey(input).context("parsing owner pubkey")?,
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
        &state.owner_pubkey,
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
        "owner_npub": account_npub(&state.owner_pubkey),
        "device_npub": account_npub(&state.device_pubkey),
        "label": state.device_label.as_deref(),
        "admin_device_npub": state
            .outbound_device_link_request
            .as_ref()
            .map(|request| account_npub(&request.admin_device_pubkey)),
        "requested_at": state
            .outbound_device_link_request
            .as_ref()
            .map(|request| request.requested_at),
        "sent_over_fips": state.outbound_device_link_request.is_some(),
    })
}

pub(crate) fn device_link_invite_json(state: &AccountState) -> Value {
    if !state.can_manage_devices() {
        return Value::Null;
    }
    let Ok(url) = iris_drive_core::device_link_invite::encode_device_link_invite(
        &state.owner_pubkey,
        &state.device_pubkey,
        &state.device_link_secret,
    ) else {
        return Value::Null;
    };
    json!({
        "url": url,
        "web_url": device_link_web_url(&url),
        "owner_npub": account_npub(&state.owner_pubkey),
        "admin_device_npub": account_npub(&state.device_pubkey),
    })
}

pub(crate) fn inbound_device_link_requests_json(state: &AccountState) -> Vec<Value> {
    state
        .inbound_device_link_requests
        .iter()
        .map(|request| {
            json!({
                "url": encode_device_approval_request(
                    &state.owner_pubkey,
                    &request.device_pubkey,
                    &request.link_secret,
                    request.label.as_deref(),
                ),
                "owner_npub": account_npub(&state.owner_pubkey),
                "device_npub": account_npub(&request.device_pubkey),
                "label": request.label.as_deref(),
                "requested_at": request.requested_at,
            })
        })
        .collect()
}

pub(crate) fn encode_device_approval_request(
    owner_hex: &str,
    device_hex: &str,
    link_secret: &str,
    label: Option<&str>,
) -> String {
    let mut url = format!(
        "iris-drive://device-link?owner={}&device={}",
        account_npub(owner_hex),
        account_npub(device_hex)
    );
    if !link_secret.trim().is_empty() {
        url.push_str("&secret=");
        url.push_str(&percent_encode_component(link_secret.trim()));
    }
    if let Some(label) = label.map(str::trim).filter(|label| !label.is_empty()) {
        url.push_str("&label=");
        url.push_str(&percent_encode_component(label));
    }
    url
}

pub(crate) fn device_link_web_url(invite_url: &str) -> String {
    iris_drive_core::device_link_invite::device_link_invite_web_url(invite_url)
}

pub(crate) fn decode_device_link_invite(
    input: &str,
) -> Result<Option<iris_drive_core::device_link_invite::ParsedDeviceLinkInvite>> {
    iris_drive_core::device_link_invite::parse_device_link_invite(input)
}

pub(crate) fn decode_device_approval_request(input: &str) -> Result<Option<DeviceApprovalRequest>> {
    let trimmed = input.trim();
    let Some(query) = device_approval_query(trimmed) else {
        return Ok(None);
    };

    let mut owner = None;
    let mut device = None;
    let mut link_secret = None;
    let mut label = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode_component(raw_key)?;
        let value = percent_decode_component(raw_value)?;
        match key.as_str() {
            "owner" if !value.trim().is_empty() => owner = Some(value),
            "device" if !value.trim().is_empty() => device = Some(value),
            "secret" | "link_secret" if !value.trim().is_empty() => link_secret = Some(value),
            "label" if !value.trim().is_empty() => label = Some(value),
            _ => {}
        }
    }

    let owner = owner.ok_or_else(|| anyhow::anyhow!("device request is missing owner"))?;
    let device = device.ok_or_else(|| anyhow::anyhow!("device request is missing device"))?;

    Ok(Some(DeviceApprovalRequest {
        owner_hex: normalize_pubkey(&owner).context("parsing request owner")?,
        device_hex: normalize_pubkey(&device).context("parsing request device")?,
        link_secret: link_secret.unwrap_or_default().trim().to_string(),
        label,
    }))
}

pub(crate) fn device_approval_query(input: &str) -> Option<&str> {
    if let Some(rest) = input.strip_prefix("iris-drive://device-link") {
        return rest.strip_prefix('?');
    }
    if let Some(rest) = input.strip_prefix("iris-drive:/device-link") {
        return rest.strip_prefix('?');
    }
    if let Some(rest) = input.strip_prefix("https://drive.iris.to/device-link") {
        return rest.strip_prefix('?');
    }
    None
}
