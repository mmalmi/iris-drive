use anyhow::{Context, Result};
use nostr_identity::{
    CreateNostrIdentityDeviceApprovalRequestOptions, NostrIdentityDeviceApprovalReceipt,
    NostrIdentityDeviceApprovalRequest, NostrIdentityDeviceApprovalRequestedResource,
    ParseNostrIdentityDeviceApprovalReceiptForBootstrapOptions,
    create_nostr_identity_device_approval_request, encode_nostr_identity_device_approval_bootstrap,
    nostr_identity_device_approval_bootstrap, nostr_identity_device_approval_bootstrap_has_prefix,
    parse_nostr_identity_device_approval_bootstrap,
    parse_nostr_identity_device_approval_receipt_event_for_bootstrap_with_options,
};
use nostr_sdk::{Event, Keys, PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{AppKeyAuthorizationState, NostrIdentityId, ProfileState, SignedNostrIdentityRosterOp};

pub const APP_KEY_LINK_REQUEST_APP_TOPIC: &str = "iris-drive/app-key-link/v1/request";
pub const APP_KEY_LINK_ROSTER_APP_TOPIC: &str = "iris-drive/app-key-link/v1/roster";
pub const APP_KEY_LINK_ROSTER_ACK_APP_TOPIC: &str = "iris-drive/app-key-link/v1/roster-ack";
pub const APP_KEY_APPROVAL_RECEIPT_APP_TOPIC: &str = "iris-drive/device-approval/v1/receipt";
pub const APP_KEY_APPROVAL_REQUEST_PREFIX: &str = "https://drive.iris.to/approve-device/";
pub const APP_KEY_APPROVAL_RELAY_URL: &str = "wss://temp.iris.to";
pub const APP_KEY_APPROVAL_REQUEST_TYPE: &str = "device_link";
pub const APP_KEY_APPROVAL_RESOURCE_TYPE: &str = "iris_drive";
pub const APP_KEY_APPROVAL_RESOURCE_ID: &str = "drive.iris.to";
pub const APP_KEY_APPROVAL_RESOURCE_SCOPES: &[&str] = &[
    "app_key",
    "write_roots",
    "receive_secret_wraps",
    "decrypt_secret_epochs",
];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AppKeyLinkRequestFrame {
    pub schema: u32,
    pub profile_id: NostrIdentityId,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub admin_app_key_pubkey: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub app_key_pubkey: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub invite_pubkey: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub requested_at: u64,
    pub url: String,
}

pub type AppKeyApprovalRequest = NostrIdentityDeviceApprovalRequest;
pub type AppKeyApprovalBootstrap = nostr_identity::NostrIdentityDeviceApprovalBootstrap;

#[derive(Debug, Clone)]
pub struct LocalAppKeyApprovalRequest {
    pub request: AppKeyApprovalRequest,
    pub request_keys: Keys,
    pub url: String,
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
) -> Result<Option<AppKeyLinkRequestFrame>> {
    if state.can_admin_profile()
        || state.authorization_state != AppKeyAuthorizationState::AwaitingApproval
    {
        return Ok(None);
    }
    let Some(pending) = state.outbound_app_key_link_request.as_ref() else {
        return Ok(None);
    };
    let (request, _) = parse_pending_app_key_approval_request(pending)?;
    if request.device_app_key_pubkey != state.app_key_pubkey {
        anyhow::bail!("pending app-key approval request AppKey mismatch");
    }
    if request
        .profile_id
        .is_some_and(|profile_id| Some(profile_id) != pending_request_profile_id(state, pending))
    {
        anyhow::bail!("pending app-key approval request profile mismatch");
    }
    if request.admin_app_key_pubkey.as_deref() != pending_admin_app_key_pubkey(pending) {
        anyhow::bail!("pending app-key approval request admin mismatch");
    }
    if u64::try_from(request.requested_at).ok() != Some(pending.requested_at) {
        anyhow::bail!("pending app-key approval request timestamp mismatch");
    }
    let url = pending.request_url.clone();
    Ok(Some(AppKeyLinkRequestFrame {
        schema: 1,
        profile_id: state.profile_id,
        admin_app_key_pubkey: String::new(),
        app_key_pubkey: String::new(),
        invite_pubkey: pending.invite_pubkey.clone(),
        label: state.app_key_label.clone(),
        requested_at: pending.requested_at,
        url,
    }))
}

fn pending_request_profile_id(
    state: &ProfileState,
    pending: &crate::profile::PendingAppKeyLinkRequest,
) -> Option<NostrIdentityId> {
    (!pending.admin_app_key_pubkey.trim().is_empty()).then_some(state.profile_id)
}

fn pending_admin_app_key_pubkey(
    pending: &crate::profile::PendingAppKeyLinkRequest,
) -> Option<&str> {
    let admin = pending.admin_app_key_pubkey.trim();
    (!admin.is_empty()).then_some(admin)
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

pub fn create_app_key_approval_request(
    device_app_key_keys: &Keys,
    profile_id: Option<NostrIdentityId>,
    admin_app_key_pubkey: Option<&str>,
    label: Option<&str>,
    requested_at: u64,
) -> Result<LocalAppKeyApprovalRequest> {
    create_app_key_approval_request_for_relay(
        device_app_key_keys,
        profile_id,
        admin_app_key_pubkey,
        label,
        requested_at,
        APP_KEY_APPROVAL_RELAY_URL,
    )
}

pub fn create_app_key_approval_request_for_relay(
    device_app_key_keys: &Keys,
    profile_id: Option<NostrIdentityId>,
    admin_app_key_pubkey: Option<&str>,
    label: Option<&str>,
    requested_at: u64,
    request_relay: &str,
) -> Result<LocalAppKeyApprovalRequest> {
    let requested_at =
        i64::try_from(requested_at).context("app-key approval request timestamp overflows i64")?;
    let resources = drive_device_approval_resources_without_relay();
    let _ = request_relay;
    let local = create_nostr_identity_device_approval_request(
        device_app_key_keys,
        CreateNostrIdentityDeviceApprovalRequestOptions {
            request_keys: None,
            request_secret: None,
            requested_at,
            request_type: Some(APP_KEY_APPROVAL_REQUEST_TYPE.to_owned()),
            resources,
            expires_at: None,
            profile_id,
            admin_app_key_pubkey: admin_app_key_pubkey.map(str::to_owned),
            label: normalize_approval_label(label),
        },
    )
    .context("creating full app-key approval request")?;
    let bootstrap = nostr_identity_device_approval_bootstrap(&local.request)
        .context("building app-key approval bootstrap")?;
    let url = encode_nostr_identity_device_approval_bootstrap(
        &bootstrap,
        Some(APP_KEY_APPROVAL_REQUEST_PREFIX),
    )
    .context("encoding app-key approval bootstrap")?;
    Ok(LocalAppKeyApprovalRequest {
        request: local.request,
        request_keys: local.request_keys,
        url,
    })
}

pub fn parse_pending_app_key_approval_request(
    pending: &crate::profile::PendingAppKeyLinkRequest,
) -> Result<(AppKeyApprovalRequest, Keys)> {
    let bootstrap = parse_app_key_approval_bootstrap(&pending.request_url)?
        .context("pending app-key approval bootstrap is missing or invalid")?;
    let request_secret = SecretKey::from_hex(pending.request_key_secret.trim())
        .context("pending app-key approval request key is missing or invalid")?;
    let request_keys = Keys::new(request_secret);
    let request = approval_request_from_bootstrap(
        &bootstrap,
        None,
        pending_admin_app_key_pubkey(pending),
        pending.requested_at,
    )?;
    if request.request_pubkey != request_keys.public_key().to_hex() {
        anyhow::bail!("pending app-key approval request key mismatch");
    }
    Ok((request, request_keys))
}

pub fn parse_app_key_approval_bootstrap(input: &str) -> Result<Option<AppKeyApprovalBootstrap>> {
    parse_nostr_identity_device_approval_bootstrap(input.trim(), &[APP_KEY_APPROVAL_REQUEST_PREFIX])
        .context("parsing app-key approval bootstrap")
}

pub fn parse_app_key_approval_request(
    input: &str,
    profile_id: NostrIdentityId,
    admin_app_key_pubkey: &str,
    requested_at: u64,
) -> Result<AppKeyApprovalRequest> {
    let bootstrap = parse_app_key_approval_bootstrap(input)?
        .context("app-key approval bootstrap is missing or invalid")?;
    approval_request_from_bootstrap(
        &bootstrap,
        Some(profile_id),
        Some(admin_app_key_pubkey),
        requested_at,
    )
}

fn approval_request_from_bootstrap(
    bootstrap: &AppKeyApprovalBootstrap,
    profile_id: Option<NostrIdentityId>,
    admin_app_key_pubkey: Option<&str>,
    requested_at: u64,
) -> Result<AppKeyApprovalRequest> {
    let request_pubkey = PublicKey::parse(&bootstrap.request_npub)
        .context("invalid device approval request npub")?
        .to_hex();
    let device_app_key_pubkey = PublicKey::parse(&bootstrap.device_app_key_npub)
        .context("invalid device approval app-key npub")?
        .to_hex();
    let requested_at =
        i64::try_from(requested_at).context("app-key approval request timestamp overflows i64")?;
    Ok(NostrIdentityDeviceApprovalRequest {
        request_pubkey,
        device_app_key_pubkey,
        request_secret: bootstrap.request_secret.clone(),
        device_app_key_proof: String::new(),
        requested_at,
        request_type: Some(APP_KEY_APPROVAL_REQUEST_TYPE.to_string()),
        resources: drive_device_approval_resources_without_relay(),
        expires_at: None,
        profile_id,
        admin_app_key_pubkey: admin_app_key_pubkey.map(str::to_string),
        label: bootstrap.label.clone(),
    })
}

pub fn parse_pending_app_key_approval_receipt_event(
    pending: &crate::profile::PendingAppKeyLinkRequest,
    event: &Event,
) -> Result<NostrIdentityDeviceApprovalReceipt> {
    let bootstrap = parse_app_key_approval_bootstrap(&pending.request_url)?
        .context("pending app-key approval bootstrap is missing or invalid")?;
    let request_secret = SecretKey::from_hex(pending.request_key_secret.trim())
        .context("pending app-key approval request key is missing or invalid")?;
    let request_keys = Keys::new(request_secret);
    parse_nostr_identity_device_approval_receipt_event_for_bootstrap_with_options(
        event,
        &request_keys,
        &bootstrap,
        ParseNostrIdentityDeviceApprovalReceiptForBootstrapOptions {
            expected_profile_id: None,
            expected_admin_app_key_pubkey: pending_admin_app_key_pubkey(pending).map(str::to_owned),
        },
    )
    .context("validating device approval receipt")
}

pub fn validate_app_key_approval_request_policy(
    request: &AppKeyApprovalRequest,
    expected_profile_id: NostrIdentityId,
    expected_admin_app_key_pubkey: &str,
    now: u64,
) -> Result<()> {
    if request.request_type.as_deref() != Some(APP_KEY_APPROVAL_REQUEST_TYPE) {
        anyhow::bail!("device approval request is not for Drive device linking");
    }
    if request
        .profile_id
        .is_some_and(|profile_id| profile_id != expected_profile_id)
    {
        anyhow::bail!("device approval request belongs to a different profile");
    }
    if request
        .admin_app_key_pubkey
        .as_deref()
        .is_some_and(|admin| admin != expected_admin_app_key_pubkey)
    {
        anyhow::bail!("device approval request targets a different admin AppKey");
    }
    let now = i64::try_from(now).context("current timestamp overflows i64")?;
    if request.requested_at > now.saturating_add(300) {
        anyhow::bail!("device approval request timestamp is in the future");
    }
    if request
        .expires_at
        .is_some_and(|expires_at| expires_at < now)
    {
        anyhow::bail!("device approval request has expired");
    }
    let has_drive_resource = request.resources.iter().any(|resource| {
        resource.resource_type == APP_KEY_APPROVAL_RESOURCE_TYPE
            && resource.id == APP_KEY_APPROVAL_RESOURCE_ID
            && APP_KEY_APPROVAL_RESOURCE_SCOPES
                .iter()
                .all(|scope| resource.scopes.iter().any(|candidate| candidate == scope))
    });
    if !has_drive_resource {
        anyhow::bail!("device approval request is missing Drive access scopes");
    }
    Ok(())
}

#[must_use]
pub fn app_key_approval_input_has_prefix(input: &str) -> bool {
    nostr_identity_device_approval_bootstrap_has_prefix(
        input.trim(),
        &[APP_KEY_APPROVAL_REQUEST_PREFIX],
    )
}

#[must_use]
pub fn drive_device_approval_resources() -> Vec<NostrIdentityDeviceApprovalRequestedResource> {
    drive_device_approval_resources_without_relay()
}

fn drive_device_approval_resources_without_relay()
-> Vec<NostrIdentityDeviceApprovalRequestedResource> {
    vec![NostrIdentityDeviceApprovalRequestedResource {
        resource_type: APP_KEY_APPROVAL_RESOURCE_TYPE.to_owned(),
        id: APP_KEY_APPROVAL_RESOURCE_ID.to_owned(),
        scopes: APP_KEY_APPROVAL_RESOURCE_SCOPES
            .iter()
            .map(|scope| (*scope).to_owned())
            .collect(),
    }]
}

fn normalize_approval_label(label: Option<&str>) -> Option<String> {
    let normalized = label?
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_matches(['.', '-'])
        .trim()
        .to_owned();
    if normalized.is_empty() {
        return None;
    }
    Some(normalized.chars().take(64).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Profile;
    use nostr_sdk::JsonUtil;
    use nostr_sdk::ToBech32;
    use tempfile::tempdir;

    fn assert_full_device_approval_request(request: &AppKeyApprovalRequest) {
        assert_eq!(request.request_secret.len(), 43);
        assert!(!request.request_pubkey.is_empty());
        assert!(!request.device_app_key_proof.is_empty());
        let proof =
            nostr_sdk::Event::from_json(&request.device_app_key_proof).expect("proof event JSON");
        proof.verify().expect("proof event verifies");
        assert_eq!(proof.pubkey.to_hex(), request.device_app_key_pubkey);
        assert!(proof.content.is_empty());
        assert!(
            proof.tags.iter().any(|tag| tag.as_slice()
                == ["request_pubkey".to_string(), request.request_pubkey.clone()])
        );
        assert!(
            !request
                .device_app_key_proof
                .contains(&request.request_secret)
        );
    }

    fn assert_drive_device_approval_resources(request: &AppKeyApprovalRequest) {
        assert_eq!(request.resources, drive_device_approval_resources());
    }

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
    fn pending_request_frame_carries_compact_bootstrap() {
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
        let approval_request = create_app_key_approval_request(
            linked.app_key.keys(),
            Some(owner.state.profile_id),
            Some(&owner.state.app_key_pubkey),
            linked.state.app_key_label.as_deref(),
            123,
        )
        .unwrap();
        assert_ne!(
            approval_request.request.request_secret,
            approval_request.request_keys.secret_key().to_secret_hex(),
            "the anti-spam request secret must be independent of the receipt key",
        );
        linked
            .state
            .queue_outbound_app_key_link_request(
                owner.state.app_key_pubkey.clone(),
                &crate::profile::app_key_link_invite_pubkey(&owner.state.app_key_link_secret)
                    .unwrap(),
                123,
                approval_request.url.clone(),
                approval_request.request_keys.secret_key().to_secret_hex(),
            )
            .unwrap();

        let frame = pending_app_key_link_request_frame(&linked.state)
            .expect("build pending frame")
            .expect("pending frame");
        let pending = linked
            .state
            .outbound_app_key_link_request
            .as_ref()
            .expect("persisted pending request");
        let (persisted_request, persisted_request_keys) =
            parse_pending_app_key_approval_request(pending).expect("persisted request material");
        let bootstrap = parse_app_key_approval_bootstrap(&frame.url)
            .expect("parse bootstrap")
            .expect("bootstrap");
        assert_eq!(
            bootstrap.request_npub,
            persisted_request_keys.public_key().to_bech32().unwrap()
        );
        assert_eq!(
            persisted_request_keys.public_key().to_hex(),
            persisted_request.request_pubkey,
        );

        assert_eq!(frame.profile_id, owner.state.profile_id);
        let parsed = approval_request.request.clone();
        assert_eq!(parsed.profile_id, Some(owner.state.profile_id));
        assert_eq!(
            parsed.admin_app_key_pubkey.as_deref(),
            Some(owner.state.app_key_pubkey.as_str())
        );
        assert_eq!(parsed.device_app_key_pubkey, linked.state.app_key_pubkey);
        assert_eq!(parsed.requested_at, 123);
        assert_eq!(parsed.label.as_deref(), Some("Phone"));
        assert_full_device_approval_request(&parsed);
        assert_drive_device_approval_resources(&parsed);
        assert!(frame.url.starts_with(APP_KEY_APPROVAL_REQUEST_PREFIX));
        assert!(
            frame.url.len()
                <= nostr_identity::NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_MAX_URI_LENGTH,
            "bootstrap URL was {}",
            frame.url.len()
        );
    }

    #[test]
    fn approval_request_encodes_compact_bootstrap_flow() {
        let profile_id = NostrIdentityId::new_v4();
        let app_key = nostr_sdk::Keys::generate();
        let admin = nostr_sdk::Keys::generate().public_key();

        let request = create_app_key_approval_request(
            &app_key,
            Some(profile_id),
            Some(&admin.to_hex()),
            Some("Web + Native"),
            123,
        )
        .expect("encode request");
        let parsed = request.request.clone();

        assert_eq!(parsed.profile_id, Some(profile_id));
        let admin_hex = admin.to_hex();
        assert_eq!(
            parsed.admin_app_key_pubkey.as_deref(),
            Some(admin_hex.as_str())
        );
        assert_eq!(parsed.device_app_key_pubkey, app_key.public_key().to_hex());
        assert_eq!(parsed.requested_at, 123);
        assert_eq!(parsed.label.as_deref(), Some("Web + Native"));
        assert_full_device_approval_request(&parsed);
        assert_drive_device_approval_resources(&parsed);
        assert!(request.url.starts_with(APP_KEY_APPROVAL_REQUEST_PREFIX));
        let bootstrap = parse_app_key_approval_bootstrap(&request.url)
            .unwrap()
            .expect("bootstrap");
        assert_eq!(bootstrap.label.as_deref(), Some("Web + Native"));
        assert_eq!(
            bootstrap.device_app_key_npub,
            app_key.public_key().to_bech32().unwrap()
        );
        assert_eq!(
            bootstrap.request_npub,
            request.request_keys.public_key().to_bech32().unwrap()
        );
        assert!(
            request.url.len()
                <= nostr_identity::NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_MAX_URI_LENGTH,
            "bootstrap URL was {}",
            request.url.len()
        );
    }

    #[test]
    fn approval_request_round_trips_label_without_profile_for_manual_join() {
        let app_key = nostr_sdk::Keys::generate();

        let request = create_app_key_approval_request(&app_key, None, None, Some("iPhone"), 123)
            .expect("encode manual join request");
        let parsed = request.request.clone();

        assert_eq!(parsed.profile_id, None);
        assert_eq!(parsed.device_app_key_pubkey, app_key.public_key().to_hex());
        assert_eq!(parsed.requested_at, 123);
        assert_eq!(parsed.label.as_deref(), Some("iPhone"));
        assert_full_device_approval_request(&parsed);
        assert_drive_device_approval_resources(&parsed);
        assert!(request.url.starts_with(APP_KEY_APPROVAL_REQUEST_PREFIX));
        assert_eq!(
            parse_app_key_approval_bootstrap(&request.url)
                .unwrap()
                .expect("bootstrap")
                .label
                .as_deref(),
            Some("iPhone")
        );
    }

    #[test]
    fn approval_policy_accepts_compact_bootstrap_request_without_relay() {
        let profile_id = NostrIdentityId::new_v4();
        let app_key = nostr_sdk::Keys::generate();
        let admin = nostr_sdk::Keys::generate();
        let request = create_app_key_approval_request(
            &app_key,
            Some(profile_id),
            Some(&admin.public_key().to_hex()),
            None,
            123,
        )
        .expect("request")
        .request;

        validate_app_key_approval_request_policy(
            &request,
            profile_id,
            &admin.public_key().to_hex(),
            123,
        )
        .expect("compact bootstrap request");
    }

    #[test]
    fn approval_request_parser_rejects_legacy_compact_app_key_link_route() {
        let app_key = nostr_sdk::Keys::generate();
        let app_key_hex = app_key.public_key().to_hex();
        let url = format!("iris-drive://app-key-link?app_key={app_key_hex}&ignored=yes");
        assert!(!app_key_approval_input_has_prefix(&url));
        assert!(parse_app_key_approval_bootstrap(&url).unwrap().is_none());
    }

    #[test]
    fn approval_bootstrap_parser_accepts_shared_prefix_and_rejects_nearby_routes() {
        let profile_id = NostrIdentityId::new_v4();
        let app_key = nostr_sdk::Keys::generate();
        let local = nostr_identity::create_nostr_identity_device_approval_request(
            &app_key,
            nostr_identity::CreateNostrIdentityDeviceApprovalRequestOptions {
                request_keys: None,
                request_secret: None,
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
        let bootstrap = nostr_identity_device_approval_bootstrap(&local.request).unwrap();
        let url = encode_nostr_identity_device_approval_bootstrap(
            &bootstrap,
            Some(APP_KEY_APPROVAL_REQUEST_PREFIX),
        )
        .expect("encode bootstrap");
        let parsed = parse_app_key_approval_bootstrap(&url)
            .expect("parse bootstrap")
            .expect("bootstrap");

        assert_eq!(
            parsed.device_app_key_npub,
            app_key.public_key().to_bech32().unwrap()
        );
        assert!(
            parse_app_key_approval_bootstrap("https://drive.iris.to/app-key-linker?owner=x")
                .unwrap()
                .is_none()
        );
    }
}
