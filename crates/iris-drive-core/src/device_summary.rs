use crate::account::DeviceAuthorizationState;
use crate::app_keys::DeviceRole;

pub fn authorization_state_key(state: DeviceAuthorizationState) -> &'static str {
    match state {
        DeviceAuthorizationState::Authorized => "authorized",
        DeviceAuthorizationState::AwaitingApproval => "awaiting_approval",
        DeviceAuthorizationState::Revoked => "revoked",
    }
}

pub fn primary_status_for_setup_state(setup_state: &str) -> &'static str {
    match setup_state {
        "authorized" => "ready",
        "awaiting_approval" => "awaiting_approval",
        "revoked" => "revoked",
        _ => "not_setup",
    }
}

pub fn setup_label_for_setup_state(setup_state: &str) -> &'static str {
    match setup_state {
        "authorized" => "Linked",
        "awaiting_approval" => "Awaiting approval",
        "revoked" => "Revoked",
        _ => "Not linked",
    }
}

pub fn primary_status_label(primary_status: &str) -> &'static str {
    match primary_status {
        "revoked" => "Device removed",
        "awaiting_approval" => "Waiting for approval",
        _ => "Ready",
    }
}

pub fn device_role_key(role: DeviceRole) -> &'static str {
    match role {
        DeviceRole::Admin => "admin",
        DeviceRole::Member => "member",
    }
}

pub fn device_role_label(role: DeviceRole) -> &'static str {
    match role {
        DeviceRole::Admin => "Admin",
        DeviceRole::Member => "Member",
    }
}

pub fn device_display_label(
    is_current_device: bool,
    label: Option<&str>,
    fallback: &str,
) -> String {
    if is_current_device {
        return "This device".to_owned();
    }
    label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .unwrap_or(fallback)
        .to_owned()
}

pub fn device_connection_state(
    is_current_device: bool,
    is_online: bool,
    is_direct: bool,
    is_mesh: bool,
) -> &'static str {
    if is_current_device {
        "local"
    } else if is_direct {
        "direct"
    } else if is_mesh {
        "mesh"
    } else if is_online {
        "online"
    } else {
        "offline"
    }
}

pub fn device_connection_label(
    connection_state: &str,
    transport_type: Option<&str>,
    srtt_ms: Option<u64>,
) -> String {
    if connection_state == "local" {
        return "This device".to_owned();
    }
    if connection_state == "offline" {
        return "Offline".to_owned();
    }
    let transport = transport_type.map(str::to_uppercase);
    match (transport, srtt_ms, connection_state) {
        (Some(transport), Some(srtt_ms), _) => format!("Online ({transport}, {srtt_ms} ms)"),
        (Some(transport), None, _) => format!("Online ({transport})"),
        (None, Some(srtt_ms), _) => format!("Online ({srtt_ms} ms)"),
        (None, None, "mesh") => "Online (Mesh)".to_owned(),
        _ => "Online".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceAuthorizationState, DeviceRole};

    #[test]
    fn shared_device_summary_labels_match_native_clients() {
        assert_eq!(
            authorization_state_key(DeviceAuthorizationState::Authorized),
            "authorized"
        );
        assert_eq!(
            authorization_state_key(DeviceAuthorizationState::AwaitingApproval),
            "awaiting_approval"
        );
        assert_eq!(
            authorization_state_key(DeviceAuthorizationState::Revoked),
            "revoked"
        );

        assert_eq!(primary_status_for_setup_state("authorized"), "ready");
        assert_eq!(setup_label_for_setup_state("authorized"), "Linked");
        assert_eq!(
            primary_status_label("awaiting_approval"),
            "Waiting for approval"
        );

        assert_eq!(device_role_key(DeviceRole::Admin), "admin");
        assert_eq!(device_role_label(DeviceRole::Member), "Member");
        assert_eq!(
            device_display_label(true, Some("Mac"), "npub1x"),
            "This device"
        );
        assert_eq!(
            device_display_label(false, Some("  Phone  "), "npub1x"),
            "Phone"
        );
        assert_eq!(device_display_label(false, Some("  "), "npub1x"), "npub1x");

        let direct = device_connection_state(false, true, true, false);
        assert_eq!(direct, "direct");
        assert_eq!(
            device_connection_label(direct, Some("tcp"), Some(17)),
            "Online (TCP, 17 ms)"
        );
        assert_eq!(device_connection_label("mesh", None, None), "Online (Mesh)");
        assert_eq!(device_connection_label("offline", None, None), "Offline");
    }
}
