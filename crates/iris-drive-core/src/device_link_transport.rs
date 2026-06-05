use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::ToBech32;
use serde::{Deserialize, Serialize};

use crate::{AccountState, DeviceAuthorizationState, IrisProfileId, SignedIrisProfileRosterOp};

pub const DEVICE_LINK_REQUEST_APP_TOPIC: &str = "iris-drive/device-link/v1/request";
pub const DEVICE_LINK_ROSTER_APP_TOPIC: &str = "iris-drive/device-link/v1/roster";
pub const DEVICE_LINK_ROSTER_ACK_APP_TOPIC: &str = "iris-drive/device-link/v1/roster-ack";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceLinkRequestFrame {
    pub schema: u32,
    pub owner_pubkey: String,
    pub device_pubkey: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub link_secret: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub requested_at: u64,
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceLinkRosterFrame {
    pub schema: u32,
    pub profile_id: IrisProfileId,
    pub owner_pubkey: String,
    pub admin_device_pubkey: String,
    pub profile_roster_ops: Vec<SignedIrisProfileRosterOp>,
    pub sent_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceLinkRosterAckFrame {
    pub schema: u32,
    pub owner_pubkey: String,
    pub admin_device_pubkey: String,
    pub device_pubkey: String,
    pub app_keys_created_at: i64,
    pub dck_generation: u64,
    pub acknowledged_at: u64,
}

#[must_use]
pub fn pending_device_link_request_frame(state: &AccountState) -> Option<DeviceLinkRequestFrame> {
    if state.can_manage_devices()
        || state.authorization_state != DeviceAuthorizationState::AwaitingApproval
    {
        return None;
    }
    let pending = state.outbound_device_link_request.as_ref()?;
    let link_secret = if pending.link_secret.trim().is_empty() {
        state.device_link_secret.clone()
    } else {
        pending.link_secret.clone()
    };
    Some(DeviceLinkRequestFrame {
        schema: 1,
        owner_pubkey: state.owner_pubkey.clone(),
        device_pubkey: state.device_pubkey.clone(),
        link_secret: link_secret.clone(),
        label: state.device_label.clone(),
        requested_at: pending.requested_at,
        url: encode_device_approval_request(
            &state.owner_pubkey,
            &state.device_pubkey,
            &link_secret,
            state.device_label.as_deref(),
        ),
    })
}

#[must_use]
pub fn encode_device_approval_request(
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

fn account_npub(hex: &str) -> String {
    PublicKey::from_hex(hex)
        .ok()
        .and_then(|pk| pk.to_bech32().ok())
        .unwrap_or_else(|| hex.to_string())
}

fn percent_encode_component(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0f));
        }
    }
    encoded
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        _ => (b'A' + (value - 10)) as char,
    }
}
