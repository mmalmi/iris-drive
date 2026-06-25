use anyhow::{Context, Result, anyhow};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::{FromBech32, ToBech32};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{AppKeyAuthorizationState, IrisProfileId, ProfileState, SignedIrisProfileRosterOp};

pub const APP_KEY_LINK_REQUEST_APP_TOPIC: &str = "iris-drive/app-key-link/v1/request";
pub const APP_KEY_LINK_ROSTER_APP_TOPIC: &str = "iris-drive/app-key-link/v1/roster";
pub const APP_KEY_LINK_ROSTER_ACK_APP_TOPIC: &str = "iris-drive/app-key-link/v1/roster-ack";
pub const APP_KEY_APPROVAL_REQUEST_PREFIX: &str = "iris-drive://app-key-link";
const APP_KEY_APPROVAL_REQUEST_SINGLE_SLASH_PREFIX: &str = "iris-drive:/app-key-link";
pub const APP_KEY_APPROVAL_REQUEST_WEB_PREFIX: &str = "https://drive.iris.to/app-key-link";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppKeyLinkRequestFrame {
    pub schema: u32,
    pub profile_id: IrisProfileId,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub admin_app_key_pubkey: String,
    pub app_key_pubkey: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub invite_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub requested_at: u64,
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppKeyApprovalRequest {
    pub profile_id: Option<IrisProfileId>,
    pub app_key_hex: String,
    pub invite_pubkey: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppKeyLinkRosterFrame {
    pub schema: u32,
    pub profile_id: IrisProfileId,
    pub admin_app_key_pubkey: String,
    pub profile_roster_ops: Vec<SignedIrisProfileRosterOp>,
    pub sent_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppKeyLinkRosterAckFrame {
    pub schema: u32,
    pub admin_app_key_pubkey: String,
    pub app_key_pubkey: String,
    pub roster_fingerprint: String,
    pub acknowledged_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppKeyLinkRosterRecipient {
    pub app_key_pubkey: String,
    pub roster_fingerprint: String,
}

#[must_use]
pub fn pending_app_key_link_request_frame(state: &ProfileState) -> Option<AppKeyLinkRequestFrame> {
    if state.can_admin_profile()
        || state.authorization_state != AppKeyAuthorizationState::AwaitingApproval
    {
        return None;
    }
    let pending = state.outbound_app_key_link_request.as_ref()?;
    Some(AppKeyLinkRequestFrame {
        schema: 1,
        profile_id: state.profile_id,
        admin_app_key_pubkey: pending.admin_app_key_pubkey.clone(),
        app_key_pubkey: state.app_key_pubkey.clone(),
        invite_pubkey: pending.invite_pubkey.clone(),
        label: state.app_key_label.clone(),
        requested_at: pending.requested_at,
        url: encode_app_key_approval_request(
            state.profile_id,
            &state.app_key_pubkey,
            &pending.invite_pubkey,
            state.app_key_label.as_deref(),
        ),
    })
}

#[must_use]
pub fn app_key_link_roster_frame(
    state: &ProfileState,
    sent_at: u64,
) -> Option<AppKeyLinkRosterFrame> {
    if !state.can_admin_profile() || !current_app_key_is_authorized(state) {
        return None;
    }
    Some(AppKeyLinkRosterFrame {
        schema: 1,
        profile_id: state.profile_id,
        admin_app_key_pubkey: state.app_key_pubkey.clone(),
        profile_roster_ops: state.profile_roster_ops.clone(),
        sent_at,
    })
}

#[must_use]
pub fn app_key_link_roster_recipients(state: &ProfileState) -> Vec<AppKeyLinkRosterRecipient> {
    let Some(app_keys) = state.current_app_keys_projection() else {
        return Vec::new();
    };
    app_keys
        .app_actors
        .iter()
        .filter(|actor| actor.pubkey != state.app_key_pubkey)
        .map(|actor| AppKeyLinkRosterRecipient {
            app_key_pubkey: actor.pubkey.clone(),
            roster_fingerprint: app_key_link_roster_fingerprint(
                &actor.pubkey,
                state.profile_id,
                &state.profile_roster_ops,
            ),
        })
        .collect()
}

#[must_use]
pub fn app_key_link_roster_ack_frame(
    state: &ProfileState,
    admin_app_key_pubkey: &str,
    acknowledged_at: u64,
) -> Option<AppKeyLinkRosterAckFrame> {
    if !current_app_key_is_authorized(state) {
        return None;
    }
    Some(AppKeyLinkRosterAckFrame {
        schema: 1,
        admin_app_key_pubkey: admin_app_key_pubkey.to_string(),
        app_key_pubkey: state.app_key_pubkey.clone(),
        roster_fingerprint: app_key_link_roster_fingerprint(
            &state.app_key_pubkey,
            state.profile_id,
            &state.profile_roster_ops,
        ),
        acknowledged_at,
    })
}

#[must_use]
pub fn app_key_link_roster_ack_matches_state(
    state: &ProfileState,
    frame: &AppKeyLinkRosterAckFrame,
) -> bool {
    state.can_admin_profile()
        && state.app_key_pubkey == frame.admin_app_key_pubkey
        && state
            .app_keys
            .as_ref()
            .is_some_and(|app_keys| app_keys.contains(&frame.app_key_pubkey))
        && frame.roster_fingerprint
            == app_key_link_roster_fingerprint(
                &frame.app_key_pubkey,
                state.profile_id,
                &state.profile_roster_ops,
            )
}

#[must_use]
pub fn app_key_link_roster_fingerprint(
    app_key_pubkey: &str,
    profile_id: IrisProfileId,
    profile_roster_ops: &[SignedIrisProfileRosterOp],
) -> String {
    let mut op_ids = profile_roster_ops
        .iter()
        .map(|op| op.op_id.as_str())
        .collect::<Vec<_>>();
    op_ids.sort_unstable();

    let mut digest = Sha256::new();
    digest.update(b"iris-drive:app-key-link-roster:v1\n");
    digest.update(profile_id.to_string().as_bytes());
    digest.update(b"\n");
    digest.update(app_key_pubkey.as_bytes());
    for op_id in op_ids {
        digest.update(b"\n");
        digest.update(op_id.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn current_app_key_is_authorized(state: &ProfileState) -> bool {
    state
        .app_keys
        .as_ref()
        .is_some_and(|app_keys| app_keys.contains(&state.app_key_pubkey))
}

#[must_use]
pub fn encode_app_key_approval_request(
    profile_id: IrisProfileId,
    app_key_hex: &str,
    invite_pubkey: &str,
    label: Option<&str>,
) -> String {
    let mut url = format!(
        "{APP_KEY_APPROVAL_REQUEST_PREFIX}?profile={}&app_key={}",
        profile_id,
        pubkey_npub(app_key_hex)
    );
    if !invite_pubkey.trim().is_empty() {
        url.push_str("&invite=");
        url.push_str(&percent_encode_component(&pubkey_npub(
            invite_pubkey.trim(),
        )));
    }
    if let Some(label) = label.map(str::trim).filter(|label| !label.is_empty()) {
        url.push_str("&label=");
        url.push_str(&percent_encode_component(label));
    }
    url
}

pub fn parse_app_key_approval_request(input: &str) -> Result<Option<AppKeyApprovalRequest>> {
    let trimmed = input.trim();
    let Some(query) = app_key_approval_query(trimmed) else {
        return Ok(None);
    };

    let mut profile_id = None;
    let mut app_key = None;
    let mut invite_pubkey = None;
    let mut label = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode_component(raw_key)?;
        let value = percent_decode_component(raw_value)?;
        match key.as_str() {
            "profile" | "profile_id" | "profileId" if !value.trim().is_empty() => {
                profile_id = Some(value.trim().parse().context("parsing request profile id")?);
            }
            "app_key" | "appKey" if !value.trim().is_empty() => app_key = Some(value),
            "invite" | "invite_pubkey" | "invitePubkey" if !value.trim().is_empty() => {
                invite_pubkey = Some(value);
            }
            "label" if !value.trim().is_empty() => label = Some(value),
            _ => {}
        }
    }

    let profile_id = profile_id.ok_or_else(|| anyhow!("AppKey-link request is missing profile"))?;
    let app_key = app_key.ok_or_else(|| anyhow!("AppKey-link request is missing AppKey"))?;
    let invite_pubkey =
        invite_pubkey.ok_or_else(|| anyhow!("AppKey-link request is missing invite pubkey"))?;

    Ok(Some(AppKeyApprovalRequest {
        profile_id: Some(profile_id),
        app_key_hex: normalize_pubkey_hex(&app_key).context("parsing request AppKey")?,
        invite_pubkey: normalize_pubkey_hex(&invite_pubkey).context("parsing request invite")?,
        label,
    }))
}

#[must_use]
pub fn app_key_approval_query(input: &str) -> Option<&str> {
    if let Some(rest) = input.strip_prefix(APP_KEY_APPROVAL_REQUEST_PREFIX) {
        return rest.strip_prefix('?');
    }
    if let Some(rest) = input.strip_prefix(APP_KEY_APPROVAL_REQUEST_SINGLE_SLASH_PREFIX) {
        return rest.strip_prefix('?');
    }
    if let Some(rest) = input.strip_prefix(APP_KEY_APPROVAL_REQUEST_WEB_PREFIX) {
        return rest.strip_prefix('?');
    }
    None
}

fn pubkey_npub(hex: &str) -> String {
    PublicKey::from_hex(hex)
        .ok()
        .and_then(|pk| pk.to_bech32().ok())
        .unwrap_or_else(|| hex.to_string())
}

fn normalize_pubkey_hex(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.starts_with("npub1") {
        let pubkey = PublicKey::from_bech32(trimmed).context("parsing npub")?;
        return Ok(pubkey.to_hex());
    }
    if trimmed.len() == 64 && trimmed.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return Ok(trimmed.to_ascii_lowercase());
    }
    Err(anyhow!(
        "expected npub1... or 64-char hex pubkey, got {trimmed}"
    ))
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

fn percent_decode_component(value: &str) -> Result<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[index + 1..index + 3])
                    .context("invalid percent escape")?;
                let byte = u8::from_str_radix(hex, 16).context("invalid percent escape")?;
                out.push(byte);
                index += 3;
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).context("invalid utf-8 in percent escape")
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
    use crate::Profile;
    use tempfile::tempdir;

    #[test]
    fn roster_fingerprint_changes_with_profile_roster_ops() {
        let dir = tempdir().unwrap();
        let mut account = Profile::create(dir.path(), Some("Mac".into())).unwrap();
        let app_actor = nostr_sdk::Keys::generate().public_key().to_hex();
        let before = app_key_link_roster_fingerprint(
            &app_actor,
            account.state.profile_id,
            &account.state.profile_roster_ops,
        );

        account
            .approve_app_key(&app_actor, Some("Browser".into()))
            .unwrap();
        let after = app_key_link_roster_fingerprint(
            &app_actor,
            account.state.profile_id,
            &account.state.profile_roster_ops,
        );

        assert_ne!(before, after);
    }

    #[test]
    fn roster_ack_matches_current_profile_roster_fingerprint() {
        let dir = tempdir().unwrap();
        let mut account = Profile::create(dir.path(), Some("Mac".into())).unwrap();
        let app_actor = nostr_sdk::Keys::generate().public_key().to_hex();
        account
            .approve_app_key(&app_actor, Some("Browser".into()))
            .unwrap();

        let recipients = app_key_link_roster_recipients(&account.state);
        let recipient = recipients
            .iter()
            .find(|recipient| recipient.app_key_pubkey == app_actor)
            .expect("approved app actor is a roster recipient");
        let frame = AppKeyLinkRosterAckFrame {
            schema: 1,
            admin_app_key_pubkey: account.state.app_key_pubkey.clone(),
            app_key_pubkey: app_actor,
            roster_fingerprint: recipient.roster_fingerprint.clone(),
            acknowledged_at: 123,
        };

        assert!(app_key_link_roster_ack_matches_state(
            &account.state,
            &frame
        ));
    }

    #[test]
    fn pending_request_frame_carries_profile_id_in_frame_and_url() {
        let owner_dir = tempdir().unwrap();
        let owner = Profile::create(owner_dir.path(), Some("Mac".into())).unwrap();
        let linked_dir = tempdir().unwrap();
        let mut linked = Profile::link_to_profile(
            linked_dir.path(),
            owner.state.profile_id,
            owner.state.app_key_pubkey.clone(),
            Some("Phone".into()),
        )
        .unwrap();
        linked
            .state
            .queue_outbound_app_key_link_request(
                owner.state.app_key_pubkey.clone(),
                &crate::profile::app_key_link_invite_pubkey(&owner.state.app_key_link_secret)
                    .unwrap(),
                123,
            )
            .unwrap();

        let frame = pending_app_key_link_request_frame(&linked.state).expect("pending frame");

        assert_eq!(frame.profile_id, owner.state.profile_id);
        assert!(
            frame
                .url
                .contains(&format!("profile={}", owner.state.profile_id))
        );
    }

    #[test]
    fn approval_request_round_trips_profile_app_key_invite_and_label_without_owner() {
        let profile_id = IrisProfileId::new_v4();
        let app_key = nostr_sdk::Keys::generate().public_key();
        let invite = nostr_sdk::Keys::generate().public_key();

        let url = encode_app_key_approval_request(
            profile_id,
            &app_key.to_hex(),
            &invite.to_hex(),
            Some("Web + Native"),
        );
        let parsed = parse_app_key_approval_request(&url)
            .expect("parse request")
            .expect("request");

        assert_eq!(parsed.profile_id, Some(profile_id));
        assert_eq!(parsed.app_key_hex, app_key.to_hex());
        assert_eq!(parsed.invite_pubkey, invite.to_hex());
        assert_eq!(parsed.label.as_deref(), Some("Web + Native"));
        assert!(!url.contains("owner="));
    }

    #[test]
    fn approval_request_parser_accepts_aliases_and_rejects_nearby_routes() {
        let profile_id = IrisProfileId::new_v4();
        let app_key = nostr_sdk::Keys::generate().public_key();
        let invite = nostr_sdk::Keys::generate().public_key();
        let url = format!(
            "iris-drive:/app-key-link?profile_id={profile_id}&app_key={}&invite={}&label=Phone+Browser",
            app_key.to_hex(),
            invite.to_hex()
        );
        let parsed = parse_app_key_approval_request(&url)
            .expect("parse request")
            .expect("request");

        assert_eq!(parsed.profile_id, Some(profile_id));
        assert_eq!(parsed.app_key_hex, app_key.to_hex());
        assert_eq!(parsed.invite_pubkey, invite.to_hex());
        assert_eq!(parsed.label.as_deref(), Some("Phone Browser"));
        assert!(
            parse_app_key_approval_request("https://drive.iris.to/app-key-linker?owner=x")
                .unwrap()
                .is_none()
        );
    }
}
