use anyhow::{Context, Result};
use nostr_identity::{
    CreateNostrIdentityDeviceApprovalRequestOptions, NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_PREFIX,
    NostrIdentityDeviceApprovalRequestedResource, create_nostr_identity_device_approval_request,
    encode_nostr_identity_device_approval_request, parse_nostr_identity_device_approval_request,
};
use nostr_sdk::Keys;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{AppKeyAuthorizationState, NostrIdentityId, ProfileState, SignedNostrIdentityRosterOp};

pub const APP_KEY_LINK_REQUEST_APP_TOPIC: &str = "iris-drive/app-key-link/v1/request";
pub const APP_KEY_LINK_ROSTER_APP_TOPIC: &str = "iris-drive/app-key-link/v1/roster";
pub const APP_KEY_LINK_ROSTER_ACK_APP_TOPIC: &str = "iris-drive/app-key-link/v1/roster-ack";
pub const APP_KEY_APPROVAL_REQUEST_PREFIX: &str = "https://drive.iris.to/approve-device/";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppKeyLinkRequestFrame {
    pub schema: u32,
    pub profile_id: NostrIdentityId,
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
    pub profile_id: Option<NostrIdentityId>,
    pub app_key_hex: String,
    pub invite_pubkey: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppKeyLinkRosterFrame {
    pub schema: u32,
    pub profile_id: NostrIdentityId,
    pub admin_app_key_pubkey: String,
    pub profile_roster_ops: Vec<SignedNostrIdentityRosterOp>,
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

pub fn pending_app_key_link_request_frame(
    state: &ProfileState,
    app_key_keys: &Keys,
) -> Result<Option<AppKeyLinkRequestFrame>> {
    if state.can_admin_profile()
        || state.authorization_state != AppKeyAuthorizationState::AwaitingApproval
    {
        return Ok(None);
    }
    let Some(pending) = state.outbound_app_key_link_request.as_ref() else {
        return Ok(None);
    };
    let url = encode_app_key_approval_request(
        app_key_keys,
        state.profile_id,
        Some(&pending.admin_app_key_pubkey),
        state.app_key_label.as_deref(),
        pending.requested_at,
    )?;
    Ok(Some(AppKeyLinkRequestFrame {
        schema: 1,
        profile_id: state.profile_id,
        admin_app_key_pubkey: pending.admin_app_key_pubkey.clone(),
        app_key_pubkey: state.app_key_pubkey.clone(),
        invite_pubkey: pending.invite_pubkey.clone(),
        label: state.app_key_label.clone(),
        requested_at: pending.requested_at,
        url,
    }))
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
    profile_id: NostrIdentityId,
    profile_roster_ops: &[SignedNostrIdentityRosterOp],
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

pub fn encode_app_key_approval_request(
    device_app_key_keys: &Keys,
    profile_id: NostrIdentityId,
    admin_app_key_pubkey: Option<&str>,
    label: Option<&str>,
    requested_at: u64,
) -> Result<String> {
    let requested_at_i64 =
        i64::try_from(requested_at).context("app-key approval requested_at overflows i64")?;
    let local = create_nostr_identity_device_approval_request(
        device_app_key_keys,
        CreateNostrIdentityDeviceApprovalRequestOptions {
            request_keys: None,
            request_secret: None,
            requested_at: requested_at_i64,
            request_type: Some("device_link".to_owned()),
            resources: vec![
                NostrIdentityDeviceApprovalRequestedResource {
                    resource_type: "nostr_identity_profile".to_owned(),
                    id: profile_id.to_string(),
                    scopes: vec!["app_key".to_owned()],
                },
                NostrIdentityDeviceApprovalRequestedResource {
                    resource_type: "iris_drive".to_owned(),
                    id: "drive.iris.to".to_owned(),
                    scopes: vec![
                        "app_key".to_owned(),
                        "write_roots".to_owned(),
                        "receive_secret_wraps".to_owned(),
                        "decrypt_secret_epochs".to_owned(),
                    ],
                },
            ],
            expires_at: None,
            profile_id: Some(profile_id),
            admin_app_key_pubkey: admin_app_key_pubkey.map(str::to_owned),
            label: label.map(str::to_owned),
        },
    )
    .context("building app-key approval request")?;
    encode_nostr_identity_device_approval_request(
        &local.request,
        Some(APP_KEY_APPROVAL_REQUEST_PREFIX),
    )
    .context("encoding app-key approval request")
}

pub fn parse_app_key_approval_request(input: &str) -> Result<Option<AppKeyApprovalRequest>> {
    let Some(request) = parse_nostr_identity_device_approval_request(
        input.trim(),
        &[APP_KEY_APPROVAL_REQUEST_PREFIX],
    )
    .context("parsing app-key approval request")?
    else {
        return Ok(None);
    };

    Ok(Some(AppKeyApprovalRequest {
        profile_id: request.profile_id,
        app_key_hex: request.device_app_key_pubkey,
        invite_pubkey: String::new(),
        label: request.label,
    }))
}

#[must_use]
pub fn app_key_approval_input_has_prefix(input: &str) -> bool {
    let value = input.trim();
    let value = value.strip_prefix("nostr:").unwrap_or(value);
    starts_with_ignore_ascii_case(value, APP_KEY_APPROVAL_REQUEST_PREFIX)
        || starts_with_ignore_ascii_case(value, NOSTR_IDENTITY_DEVICE_APPROVAL_REQUEST_PREFIX)
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value.len() >= prefix.len() && value[..prefix.len()].eq_ignore_ascii_case(prefix)
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

        let frame = pending_app_key_link_request_frame(&linked.state, linked.app_key.keys())
            .expect("build pending frame")
            .expect("pending frame");

        assert_eq!(frame.profile_id, owner.state.profile_id);
        let parsed = parse_app_key_approval_request(&frame.url)
            .expect("parse request")
            .expect("request");
        assert_eq!(parsed.profile_id, Some(owner.state.profile_id));
        assert_eq!(parsed.app_key_hex, linked.state.app_key_pubkey);
        assert!(frame.url.starts_with(APP_KEY_APPROVAL_REQUEST_PREFIX));
    }

    #[test]
    fn approval_request_round_trips_profile_app_key_invite_and_label_without_owner() {
        let profile_id = NostrIdentityId::new_v4();
        let app_key = nostr_sdk::Keys::generate();
        let admin = nostr_sdk::Keys::generate().public_key();

        let url = encode_app_key_approval_request(
            &app_key,
            profile_id,
            Some(&admin.to_hex()),
            Some("Web + Native"),
            123,
        )
        .expect("encode request");
        let parsed = parse_app_key_approval_request(&url)
            .expect("parse request")
            .expect("request");

        assert_eq!(parsed.profile_id, Some(profile_id));
        assert_eq!(parsed.app_key_hex, app_key.public_key().to_hex());
        assert!(parsed.invite_pubkey.is_empty());
        assert_eq!(parsed.label.as_deref(), Some("Web + Native"));
        let raw = nostr_identity::parse_nostr_identity_device_approval_request(
            &url,
            &[APP_KEY_APPROVAL_REQUEST_PREFIX],
        )
        .expect("parse raw approval request")
        .expect("raw approval request");
        assert!(raw.resources.iter().any(|resource| {
            resource.resource_type == "iris_drive"
                && resource.id == "drive.iris.to"
                && [
                    "app_key",
                    "write_roots",
                    "receive_secret_wraps",
                    "decrypt_secret_epochs",
                ]
                .iter()
                .all(|scope| resource.scopes.iter().any(|candidate| candidate == scope))
        }));
        assert!(!url.contains("owner="));
    }

    #[test]
    fn approval_request_parser_accepts_shared_prefix_and_rejects_nearby_routes() {
        let profile_id = NostrIdentityId::new_v4();
        let app_key = nostr_sdk::Keys::generate();
        let local = nostr_identity::create_nostr_identity_device_approval_request(
            &app_key,
            nostr_identity::CreateNostrIdentityDeviceApprovalRequestOptions {
                request_keys: None,
                request_secret: Some("secret_abcdefghijklmnopqrstuvwxyz123456".to_owned()),
                requested_at: 123,
                request_type: Some("device_link".to_owned()),
                resources: Vec::new(),
                expires_at: None,
                profile_id: Some(profile_id),
                admin_app_key_pubkey: None,
                label: Some("Phone Browser".to_owned()),
            },
        )
        .expect("request");
        let url =
            nostr_identity::encode_nostr_identity_device_approval_request(&local.request, None)
                .expect("encode request");
        let parsed = parse_app_key_approval_request(&url)
            .expect("parse request")
            .expect("request");

        assert_eq!(parsed.profile_id, Some(profile_id));
        assert_eq!(parsed.app_key_hex, app_key.public_key().to_hex());
        assert!(parsed.invite_pubkey.is_empty());
        assert_eq!(parsed.label.as_deref(), Some("Phone Browser"));
        assert!(
            parse_app_key_approval_request("https://drive.iris.to/app-key-linker?owner=x")
                .unwrap()
                .is_none()
        );
    }
}
