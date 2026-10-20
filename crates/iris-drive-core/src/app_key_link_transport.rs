use anyhow::{Context, Result};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use nostr_identity::{
    NOSTR_IDENTITY_DEVICE_APPROVAL_APPLIED_ACK_SCHEMA, NostrIdentityDeviceApprovalAppliedAck,
    NostrIdentityDeviceApprovalReceipt, NostrIdentityRosterOp,
    ParseNostrIdentityDeviceApprovalReceiptForBootstrapOptions,
    build_nostr_identity_device_approval_applied_ack_event,
    encode_nostr_identity_device_approval_bootstrap,
    nostr_identity_device_approval_bootstrap_has_prefix,
    parse_nostr_identity_device_approval_applied_ack_event,
    parse_nostr_identity_device_approval_bootstrap,
    parse_nostr_identity_device_approval_receipt_event_for_bootstrap_with_options,
    parse_nostr_identity_device_approval_receipt_roster_op,
};
use nostr_sdk::nips::nip19::ToBech32;
use nostr_sdk::nips::nip44;
use nostr_sdk::{Event, JsonUtil, Keys, PublicKey, SecretKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{AppKeyAuthorizationState, NostrIdentityId, ProfileState, SignedNostrIdentityRosterOp};

pub const APP_KEY_LINK_REQUEST_APP_TOPIC: &str = "iris-drive/app-key-link/v1/request";
pub const APP_KEY_LINK_ROSTER_APP_TOPIC: &str = "iris-drive/app-key-link/v1/roster";
pub const APP_KEY_LINK_ROSTER_ACK_APP_TOPIC: &str = "iris-drive/app-key-link/v1/roster-ack";
pub const APP_KEY_APPROVAL_RECEIPT_APP_TOPIC: &str = "iris-drive/device-approval/v1/receipt";
pub const APP_KEY_APPROVAL_APPLIED_ACK_APP_TOPIC: &str =
    "iris-drive/device-approval/v1/applied-ack";
pub const APP_KEY_APPROVAL_REQUEST_PREFIX: &str = "https://drive.iris.to/approve-device/";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppKeyLinkRequestFrame {
    #[serde(rename = "v")]
    pub schema: u32,
    #[serde(rename = "i")]
    pub invite_pubkey: String,
    #[serde(default, rename = "l", skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(rename = "r")]
    pub request_npub: String,
    #[serde(rename = "s")]
    pub request_secret: String,
}

pub type AppKeyApprovalBootstrap = nostr_identity::NostrIdentityDeviceApprovalBootstrap;

#[derive(Debug, Clone)]
pub struct LocalAppKeyApprovalBootstrap {
    pub bootstrap: AppKeyApprovalBootstrap,
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
    let (bootstrap, _) = parse_pending_app_key_approval_bootstrap(pending)?;
    let device_app_key_pubkey = PublicKey::parse(&bootstrap.device_app_key_npub)
        .context("parsing pending approval bootstrap device AppKey")?
        .to_hex();
    if device_app_key_pubkey != state.app_key_pubkey {
        anyhow::bail!("pending app-key approval request AppKey mismatch");
    }
    Ok(Some(AppKeyLinkRequestFrame {
        schema: 1,
        invite_pubkey: pending.invite_pubkey.clone(),
        label: state.app_key_label.clone(),
        request_npub: bootstrap.request_npub,
        request_secret: bootstrap.request_secret,
    }))
}

pub fn app_key_link_request_frame_url(
    frame: &AppKeyLinkRequestFrame,
    app_key_pubkey: &str,
) -> Result<String> {
    let device_app_key_npub = PublicKey::from_hex(app_key_pubkey)
        .context("parsing compact app-key link frame device AppKey")?
        .to_bech32()
        .context("encoding compact app-key link frame device AppKey npub")?;
    encode_nostr_identity_device_approval_bootstrap(
        &AppKeyApprovalBootstrap {
            device_app_key_npub,
            request_npub: frame.request_npub.clone(),
            request_secret: frame.request_secret.clone(),
            label: normalize_approval_label(frame.label.as_deref()),
        },
        Some(APP_KEY_APPROVAL_REQUEST_PREFIX),
    )
    .context("encoding compact app-key link request URL")
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

pub fn create_app_key_approval_bootstrap(
    device_app_key_keys: &Keys,
    label: Option<&str>,
) -> Result<LocalAppKeyApprovalBootstrap> {
    let request_keys = loop {
        let keys = Keys::generate();
        if keys.public_key() != device_app_key_keys.public_key() {
            break keys;
        }
    };
    let request_secret_material = Keys::generate();
    let bootstrap = AppKeyApprovalBootstrap {
        device_app_key_npub: device_app_key_keys
            .public_key()
            .to_bech32()
            .context("encoding device AppKey npub")?,
        request_npub: request_keys
            .public_key()
            .to_bech32()
            .context("encoding approval request npub")?,
        request_secret: URL_SAFE_NO_PAD
            .encode(request_secret_material.secret_key().as_secret_bytes()),
        label: normalize_approval_label(label),
    };
    let url = encode_nostr_identity_device_approval_bootstrap(
        &bootstrap,
        Some(APP_KEY_APPROVAL_REQUEST_PREFIX),
    )
    .context("encoding app-key approval bootstrap")?;
    Ok(LocalAppKeyApprovalBootstrap {
        bootstrap,
        request_keys,
        url,
    })
}

pub fn parse_pending_app_key_approval_bootstrap(
    pending: &crate::profile::PendingAppKeyLinkRequest,
) -> Result<(AppKeyApprovalBootstrap, Keys)> {
    let bootstrap = parse_app_key_approval_bootstrap(&pending.request_url)?
        .context("pending app-key approval bootstrap is missing or invalid")?;
    let request_secret = SecretKey::from_hex(pending.request_key_secret.trim())
        .context("pending app-key approval request key is missing or invalid")?;
    let request_keys = Keys::new(request_secret);
    if PublicKey::parse(&bootstrap.request_npub)
        .context("parsing pending app-key approval request npub")?
        != request_keys.public_key()
    {
        anyhow::bail!("pending app-key approval request key mismatch");
    }
    Ok((bootstrap, request_keys))
}

pub fn parse_app_key_approval_bootstrap(input: &str) -> Result<Option<AppKeyApprovalBootstrap>> {
    parse_nostr_identity_device_approval_bootstrap(input.trim(), &[APP_KEY_APPROVAL_REQUEST_PREFIX])
        .context("parsing app-key approval bootstrap")
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

/// Build the shared, device-AppKey-signed proof that an exact approval receipt
/// has been durably applied locally.
pub fn device_approval_applied_ack_is_ready(
    state: &ProfileState,
    device_app_key_keys: &Keys,
    approval_event: &Event,
) -> Result<bool> {
    let pending = state
        .outbound_app_key_link_request
        .as_ref()
        .context("device approval request is no longer pending")?;
    let receipt_was_persisted = pending
        .approval_receipt_event
        .iter()
        .map(|json| Event::from_json(json).context("parsing persisted approval receipt"))
        .collect::<Result<Vec<_>>>()?
        .iter()
        .any(|persisted| persisted.id == approval_event.id);
    if !receipt_was_persisted {
        anyhow::bail!("approval receipt does not match the durably applied event");
    }
    let receipt = parse_pending_app_key_approval_receipt_event(pending, approval_event)?;
    let device_app_key_pubkey = device_app_key_keys.public_key().to_hex();
    if receipt.profile_id != state.profile_id
        || receipt.device_app_key_pubkey != state.app_key_pubkey
        || device_app_key_pubkey != state.app_key_pubkey
    {
        anyhow::bail!("approval receipt does not match the bound profile and device AppKey");
    }
    if state.authorization_state != AppKeyAuthorizationState::Authorized {
        return Ok(false);
    }
    let receipt_roster_op = parse_nostr_identity_device_approval_receipt_roster_op(&receipt)
        .context("parsing approval receipt roster op")?;
    let projection = state.profile_projection();
    if !projection
        .accepted_op_ids
        .contains(&receipt_roster_op.op_id)
        || !projection.can_write_roots(&state.app_key_pubkey)
        || !projection.can_admin_profile(&receipt.approved_by_pubkey)
    {
        return Ok(false);
    }
    let Some(epoch) = projection.secret_epochs.values().next_back() else {
        return Ok(false);
    };
    let Some(wrap) = epoch.wrapped_secrets.get(&state.app_key_pubkey) else {
        return Ok(false);
    };
    let signer = PublicKey::from_hex(&epoch.signed_by_pubkey)
        .context("parsing approval key-epoch signer")?;
    Ok(
        nip44::decrypt_to_bytes(device_app_key_keys.secret_key(), &signer, wrap)
            .ok()
            .and_then(|plaintext| crate::profile::decode_dck_plaintext(&plaintext).ok())
            .is_some(),
    )
}

pub fn device_approval_applied_ack_event(
    state: &ProfileState,
    device_app_key_keys: &Keys,
    approval_event: &Event,
    applied_at: u64,
) -> Result<Event> {
    if !device_approval_applied_ack_is_ready(state, device_app_key_keys, approval_event)? {
        anyhow::bail!("device approval is not causally complete and decryptable");
    }
    let pending = state
        .outbound_app_key_link_request
        .as_ref()
        .context("device approval request is no longer pending")?;
    let receipt = parse_pending_app_key_approval_receipt_event(pending, approval_event)?;
    build_nostr_identity_device_approval_applied_ack_event(
        device_app_key_keys,
        NostrIdentityDeviceApprovalAppliedAck {
            schema: NOSTR_IDENTITY_DEVICE_APPROVAL_APPLIED_ACK_SCHEMA,
            request_pubkey: receipt.request_pubkey,
            device_app_key_pubkey: receipt.device_app_key_pubkey,
            approval_event_id: approval_event.id.to_hex(),
            approved_by_pubkey: receipt.approved_by_pubkey,
            applied_at: i64::try_from(applied_at).context("approval ACK timestamp overflow")?,
        },
    )
    .context("building device approval applied ACK")
}

/// Build applied-ACKs for every retained receipt whose exact roster op and
/// current DCK wrap are durably usable. Multiple admins may validly approve
/// the same unbound request before their roster branches merge.
pub fn device_approval_applied_ack_events(
    state: &ProfileState,
    device_app_key_keys: &Keys,
    applied_at: u64,
) -> Result<Vec<Event>> {
    let pending = state
        .outbound_app_key_link_request
        .as_ref()
        .context("device approval request is no longer pending")?;
    let approval_events = pending
        .approval_receipt_event
        .iter()
        .map(|json| Event::from_json(json).context("parsing persisted approval receipt"))
        .collect::<Result<Vec<_>>>()?;
    let mut acknowledgements = Vec::with_capacity(approval_events.len());
    for approval in approval_events {
        if device_approval_applied_ack_is_ready(state, device_app_key_keys, &approval)? {
            acknowledgements.push(device_approval_applied_ack_event(
                state,
                device_app_key_keys,
                &approval,
                applied_at,
            )?);
        }
    }
    Ok(acknowledgements)
}

/// Remove a pending approval receipt only after validating the shared signed
/// durable-apply ACK against every exact receipt coordinate.
pub fn apply_device_approval_applied_ack_event(
    state: &mut ProfileState,
    event: &Event,
) -> Result<bool> {
    let ack = parse_nostr_identity_device_approval_applied_ack_event(event)
        .context("validating device approval applied ACK")?;
    if !state.can_admin_profile() || ack.approved_by_pubkey != state.app_key_pubkey {
        return Ok(false);
    }
    let before = state.pending_device_approval_receipts.len();
    state.pending_device_approval_receipts.retain(|pending| {
        if pending.request_pubkey != ack.request_pubkey
            || pending.device_app_key_pubkey != ack.device_app_key_pubkey
        {
            return true;
        }
        Event::from_json(&pending.event_json)
            .map_or(true, |receipt| receipt.id.to_hex() != ack.approval_event_id)
    });
    Ok(state.pending_device_approval_receipts.len() != before)
}

#[must_use]
pub fn pending_app_key_approval_receipt_authorizes_app_key(
    pending: &crate::profile::PendingAppKeyLinkRequest,
    app_key_pubkey: &str,
) -> bool {
    pending.approval_receipt_event.iter().any(|event_json| {
        let Ok(event) = Event::from_json(event_json) else {
            return false;
        };
        let Ok(receipt) = parse_pending_app_key_approval_receipt_event(pending, &event) else {
            return false;
        };
        if receipt.device_app_key_pubkey != app_key_pubkey {
            return false;
        }
        let Ok(roster_op) = parse_nostr_identity_device_approval_receipt_roster_op(&receipt) else {
            return false;
        };
        matches!(
            &roster_op.content.op,
            NostrIdentityRosterOp::AddFacet { facet }
                if facet.pubkey == app_key_pubkey
                    && facet.is_app_key()
                    && facet.capabilities.can_write_roots
        )
    })
}

#[must_use]
pub fn app_key_approval_input_has_prefix(input: &str) -> bool {
    nostr_identity_device_approval_bootstrap_has_prefix(
        input.trim(),
        &[APP_KEY_APPROVAL_REQUEST_PREFIX],
    )
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
    let mut compact = String::new();
    for character in normalized.chars() {
        if compact.len() + character.len_utf8()
            > nostr_identity::NOSTR_IDENTITY_DEVICE_APPROVAL_LABEL_MAX_BYTES
        {
            break;
        }
        compact.push(character);
    }
    let compact = compact.trim_end().to_owned();
    (!compact.is_empty()).then_some(compact)
}

#[cfg(test)]
mod tests;
