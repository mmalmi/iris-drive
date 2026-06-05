use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::ToBech32;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

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
    pub roster_fingerprint: String,
    pub acknowledged_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceLinkRosterRecipient {
    pub device_pubkey: String,
    pub roster_fingerprint: String,
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
pub fn device_link_roster_frame(
    state: &AccountState,
    sent_at: u64,
) -> Option<DeviceLinkRosterFrame> {
    if !state.can_manage_devices() || !current_app_key_is_authorized(state) {
        return None;
    }
    Some(DeviceLinkRosterFrame {
        schema: 1,
        profile_id: state.profile_id,
        owner_pubkey: state.owner_pubkey.clone(),
        admin_device_pubkey: state.device_pubkey.clone(),
        profile_roster_ops: state.profile_roster_ops.clone(),
        sent_at,
    })
}

#[must_use]
pub fn device_link_roster_recipients(state: &AccountState) -> Vec<DeviceLinkRosterRecipient> {
    let Some(app_keys) = state.app_keys.as_ref() else {
        return Vec::new();
    };
    app_keys
        .app_actors
        .iter()
        .filter(|actor| actor.pubkey != state.device_pubkey)
        .map(|actor| DeviceLinkRosterRecipient {
            device_pubkey: actor.pubkey.clone(),
            roster_fingerprint: device_link_roster_fingerprint(
                &actor.pubkey,
                state.profile_id,
                &state.profile_roster_ops,
            ),
        })
        .collect()
}

#[must_use]
pub fn device_link_roster_ack_frame(
    state: &AccountState,
    admin_device_pubkey: &str,
    acknowledged_at: u64,
) -> Option<DeviceLinkRosterAckFrame> {
    if !current_app_key_is_authorized(state) {
        return None;
    }
    Some(DeviceLinkRosterAckFrame {
        schema: 1,
        owner_pubkey: state.owner_pubkey.clone(),
        admin_device_pubkey: admin_device_pubkey.to_string(),
        device_pubkey: state.device_pubkey.clone(),
        roster_fingerprint: device_link_roster_fingerprint(
            &state.device_pubkey,
            state.profile_id,
            &state.profile_roster_ops,
        ),
        acknowledged_at,
    })
}

#[must_use]
pub fn device_link_roster_ack_matches_state(
    state: &AccountState,
    frame: &DeviceLinkRosterAckFrame,
) -> bool {
    state.can_manage_devices()
        && state.owner_pubkey == frame.owner_pubkey
        && state.device_pubkey == frame.admin_device_pubkey
        && state
            .app_keys
            .as_ref()
            .is_some_and(|app_keys| app_keys.contains(&frame.device_pubkey))
        && frame.roster_fingerprint
            == device_link_roster_fingerprint(
                &frame.device_pubkey,
                state.profile_id,
                &state.profile_roster_ops,
            )
}

#[must_use]
pub fn device_link_roster_fingerprint(
    device_pubkey: &str,
    profile_id: IrisProfileId,
    profile_roster_ops: &[SignedIrisProfileRosterOp],
) -> String {
    let mut op_ids = profile_roster_ops
        .iter()
        .map(|op| op.op_id.as_str())
        .collect::<Vec<_>>();
    op_ids.sort_unstable();

    let mut digest = Sha256::new();
    digest.update(b"iris-drive:device-link-roster:v1\n");
    digest.update(profile_id.to_string().as_bytes());
    digest.update(b"\n");
    digest.update(device_pubkey.as_bytes());
    for op_id in op_ids {
        digest.update(b"\n");
        digest.update(op_id.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn current_app_key_is_authorized(state: &AccountState) -> bool {
    state
        .app_keys
        .as_ref()
        .is_some_and(|app_keys| app_keys.contains(&state.device_pubkey))
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Account;
    use tempfile::tempdir;

    #[test]
    fn roster_fingerprint_changes_with_profile_roster_ops() {
        let dir = tempdir().unwrap();
        let mut account = Account::create(dir.path(), Some("Mac".into())).unwrap();
        let app_actor = nostr_sdk::Keys::generate().public_key().to_hex();
        let before = device_link_roster_fingerprint(
            &app_actor,
            account.state.profile_id,
            &account.state.profile_roster_ops,
        );

        account
            .approve_device(&app_actor, Some("Browser".into()))
            .unwrap();
        let after = device_link_roster_fingerprint(
            &app_actor,
            account.state.profile_id,
            &account.state.profile_roster_ops,
        );

        assert_ne!(before, after);
    }

    #[test]
    fn roster_ack_matches_current_profile_roster_fingerprint() {
        let dir = tempdir().unwrap();
        let mut account = Account::create(dir.path(), Some("Mac".into())).unwrap();
        let app_actor = nostr_sdk::Keys::generate().public_key().to_hex();
        account
            .approve_device(&app_actor, Some("Browser".into()))
            .unwrap();

        let recipients = device_link_roster_recipients(&account.state);
        let recipient = recipients
            .iter()
            .find(|recipient| recipient.device_pubkey == app_actor)
            .expect("approved app actor is a roster recipient");
        let frame = DeviceLinkRosterAckFrame {
            schema: 1,
            owner_pubkey: account.state.owner_pubkey.clone(),
            admin_device_pubkey: account.state.device_pubkey.clone(),
            device_pubkey: app_actor,
            roster_fingerprint: recipient.roster_fingerprint.clone(),
            acknowledged_at: 123,
        };

        assert!(device_link_roster_ack_matches_state(&account.state, &frame));
    }
}
