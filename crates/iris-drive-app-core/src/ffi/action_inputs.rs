use super::{
    AppConfig, AppKeyApprovalBootstrap, UiAppKeyLinkRequest, pubkey_npub, unix_now_seconds,
};

pub(super) fn inbound_app_key_link_requests(
    state: &iris_drive_core::ProfileState,
) -> Vec<UiAppKeyLinkRequest> {
    if !state.can_admin_profile() {
        return Vec::new();
    }
    state
        .inbound_app_key_link_requests
        .iter()
        .map(|request| UiAppKeyLinkRequest {
            app_key_pubkey: pubkey_npub(&request.app_key_pubkey),
            label: request.label.clone().unwrap_or_default(),
            requested_at: request.requested_at,
            request_link: request.request_url.clone(),
        })
        .collect()
}

pub(super) fn resolve_app_key_link_target(
    input: &str,
) -> Result<iris_drive_core::AppKeyLinkTarget, String> {
    iris_drive_core::resolve_app_key_link_target(input, None).map_err(|error| {
        if error.to_string().contains("NostrIdentity UUID") {
            "paste an NostrIdentity invite URL to link this device".to_owned()
        } else {
            error.to_string()
        }
    })
}

pub(super) fn decode_app_key_approval_bootstrap(
    config: &AppConfig,
    request: &str,
) -> Result<AppKeyApprovalBootstrap, String> {
    config
        .profile
        .as_ref()
        .ok_or_else(|| "profile admin is required to approve devices".to_string())?;
    iris_drive_core::app_key_link_transport::parse_app_key_approval_bootstrap(request)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "app-key approval bootstrap is missing or invalid".to_string())
}

pub(super) fn optional_trimmed(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

pub(super) fn parse_share_role(value: &str) -> anyhow::Result<iris_drive_core::ShareRole> {
    iris_drive_core::ShareRole::parse_user_input(value).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid share role {}; expected reader, editor, or admin",
            value.trim()
        )
    })
}

pub(super) fn share_now_seconds() -> i64 {
    i64::try_from(unix_now_seconds()).unwrap_or(i64::MAX)
}
