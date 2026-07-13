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
            label: frame.label.clone(),
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
pub fn device_approval_applied_ack_event(
    state: &ProfileState,
    device_app_key_keys: &Keys,
    approval_event: &Event,
    applied_at: u64,
) -> Result<Event> {
    let pending = state
        .outbound_app_key_link_request
        .as_ref()
        .context("device approval request is no longer pending")?;
    let receipt = parse_pending_app_key_approval_receipt_event(pending, approval_event)?;
    if state.authorization_state != AppKeyAuthorizationState::Authorized {
        anyhow::bail!("device approval was not durably applied as authorized");
    }
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
    let Some(event_json) = pending.approval_receipt_event.as_deref() else {
        return false;
    };
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
    (!compact.is_empty()).then_some(compact)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppConfig, Profile};
    use nostr_sdk::ToBech32;
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
    fn applied_ack_clears_only_the_exact_durably_applied_approval() {
        let owner_dir = tempdir().unwrap();
        let mut owner = Profile::create(owner_dir.path(), Some("iPhone".into())).unwrap();
        let linked_dir = tempdir().unwrap();
        let mut linked =
            Profile::start_join_request(linked_dir.path(), Some("Mac".into())).unwrap();
        let approval = create_app_key_approval_bootstrap(
            linked.app_key.keys(),
            linked.state.app_key_label.as_deref(),
        )
        .unwrap();
        linked.state.queue_unbound_app_key_join_request(
            10,
            approval.url,
            approval.request_keys.secret_key().to_secret_hex(),
        );
        owner
            .approve_device_bootstrap(&approval.bootstrap, Some("Mac".into()))
            .unwrap();
        let receipt =
            Event::from_json(&owner.state.pending_device_approval_receipts[0].event_json).unwrap();
        let mut linked_config = AppConfig {
            profile: Some(linked.state),
            ..AppConfig::default()
        };
        assert_eq!(
            crate::relay_sync::apply_remote_device_approval_receipt_event(
                &mut linked_config,
                &receipt,
            )
            .unwrap(),
            crate::relay_sync::NostrIdentityRosterOpApply::Applied,
        );
        let linked_state = linked_config.profile.as_ref().unwrap();
        let ack =
            device_approval_applied_ack_event(linked_state, linked.app_key.keys(), &receipt, 11)
                .unwrap();

        assert!(apply_device_approval_applied_ack_event(&mut owner.state, &ack).unwrap());
        assert!(owner.state.pending_device_approval_receipts.is_empty());
        assert!(!apply_device_approval_applied_ack_event(&mut owner.state, &ack).unwrap());
    }

    #[test]
    fn lost_applied_ack_is_rebuilt_after_full_roster_apply() {
        let owner_dir = tempdir().unwrap();
        let mut owner = Profile::create(owner_dir.path(), Some("iPhone".into())).unwrap();
        let linked_dir = tempdir().unwrap();
        let mut linked =
            Profile::start_join_request(linked_dir.path(), Some("Mac".into())).unwrap();
        let approval = create_app_key_approval_bootstrap(
            linked.app_key.keys(),
            linked.state.app_key_label.as_deref(),
        )
        .unwrap();
        linked.state.queue_unbound_app_key_join_request(
            10,
            approval.url,
            approval.request_keys.secret_key().to_secret_hex(),
        );
        owner
            .approve_device_bootstrap(&approval.bootstrap, Some("Mac".into()))
            .unwrap();
        let receipt =
            Event::from_json(&owner.state.pending_device_approval_receipts[0].event_json).unwrap();
        let mut linked_config = AppConfig {
            profile: Some(linked.state),
            ..AppConfig::default()
        };
        assert_eq!(
            crate::relay_sync::apply_remote_device_approval_receipt_event(
                &mut linked_config,
                &receipt,
            )
            .unwrap(),
            crate::relay_sync::NostrIdentityRosterOpApply::Applied,
        );
        let first_ack = device_approval_applied_ack_event(
            linked_config.profile.as_ref().unwrap(),
            linked.app_key.keys(),
            &receipt,
            11,
        )
        .unwrap();

        let roster = app_key_link_roster_frame(&owner.state, 12).unwrap();
        assert!(matches!(
            crate::relay_sync::apply_app_key_link_roster_frame(
                &mut linked_config,
                &roster,
                &owner.state.app_key_pubkey,
            )
            .unwrap(),
            crate::relay_sync::AppKeyLinkRosterApply::Applied(_)
        ));
        let linked_config_path = linked_dir.path().join("config.toml");
        linked_config.save(&linked_config_path).unwrap();
        let mut linked_config = AppConfig::load_or_default(&linked_config_path).unwrap();

        // The first ACK is lost. The owner therefore resends the exact receipt
        // after the linked device has restarted with the full roster applied.
        assert_eq!(
            crate::relay_sync::apply_remote_device_approval_receipt_event(
                &mut linked_config,
                &receipt,
            )
            .unwrap(),
            crate::relay_sync::NostrIdentityRosterOpApply::Current,
        );
        let replayed_ack = device_approval_applied_ack_event(
            linked_config.profile.as_ref().unwrap(),
            linked.app_key.keys(),
            &receipt,
            13,
        )
        .unwrap();
        let first = parse_nostr_identity_device_approval_applied_ack_event(&first_ack).unwrap();
        let replayed =
            parse_nostr_identity_device_approval_applied_ack_event(&replayed_ack).unwrap();
        assert_eq!(replayed.request_pubkey, first.request_pubkey);
        assert_eq!(replayed.device_app_key_pubkey, first.device_app_key_pubkey);
        assert_eq!(replayed.approval_event_id, first.approval_event_id);
        assert_eq!(replayed.approved_by_pubkey, first.approved_by_pubkey);

        assert!(apply_device_approval_applied_ack_event(&mut owner.state, &replayed_ack).unwrap());
        assert!(owner.state.pending_device_approval_receipts.is_empty());
    }

    #[test]
    fn pending_request_frame_carries_only_compact_bootstrap_material() {
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
        let approval_request = create_app_key_approval_bootstrap(
            linked.app_key.keys(),
            linked.state.app_key_label.as_deref(),
        )
        .unwrap();
        assert_ne!(
            approval_request.bootstrap.request_secret,
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
        let (persisted_bootstrap, persisted_request_keys) =
            parse_pending_app_key_approval_bootstrap(pending)
                .expect("persisted bootstrap material");
        let frame_url = app_key_link_request_frame_url(&frame, &linked.state.app_key_pubkey)
            .expect("frame URL");
        let bootstrap = parse_app_key_approval_bootstrap(&frame_url)
            .expect("parse bootstrap")
            .expect("bootstrap");
        assert_eq!(bootstrap, approval_request.bootstrap);
        assert_eq!(bootstrap, persisted_bootstrap);
        assert_eq!(
            bootstrap.request_npub,
            persisted_request_keys.public_key().to_bech32().unwrap()
        );
        assert_eq!(
            bootstrap.device_app_key_npub,
            linked.app_key.keys().public_key().to_bech32().unwrap()
        );
        assert_ne!(bootstrap.device_app_key_npub, bootstrap.request_npub);
        assert_eq!(
            serde_json::to_value(&frame)
                .unwrap()
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            ["i", "l", "r", "s", "v"]
                .into_iter()
                .map(str::to_string)
                .collect()
        );
        assert!(frame_url.starts_with(APP_KEY_APPROVAL_REQUEST_PREFIX));
        assert!(
            frame_url.len()
                <= nostr_identity::NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_MAX_URI_LENGTH,
            "bootstrap URL was {}",
            frame_url.len()
        );
    }

    #[test]
    fn approval_bootstrap_has_stable_app_npub_distinct_request_npub_and_32_byte_secret() {
        let app_key = nostr_sdk::Keys::generate();
        let local = create_app_key_approval_bootstrap(&app_key, Some("Web + Native"))
            .expect("encode bootstrap");
        let bootstrap = parse_app_key_approval_bootstrap(&local.url)
            .unwrap()
            .expect("bootstrap");
        assert_eq!(bootstrap, local.bootstrap);
        assert_eq!(bootstrap.label.as_deref(), Some("Web + Native"));
        assert_eq!(
            bootstrap.device_app_key_npub,
            app_key.public_key().to_bech32().unwrap()
        );
        assert_eq!(
            bootstrap.request_npub,
            local.request_keys.public_key().to_bech32().unwrap()
        );
        assert_ne!(bootstrap.device_app_key_npub, bootstrap.request_npub);
        assert_eq!(
            URL_SAFE_NO_PAD
                .decode(&bootstrap.request_secret)
                .unwrap()
                .len(),
            32
        );
        let payload = &local.url[APP_KEY_APPROVAL_REQUEST_PREFIX.len()..];
        let payload: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        assert_eq!(
            payload
                .as_object()
                .unwrap()
                .keys()
                .cloned()
                .collect::<std::collections::BTreeSet<_>>(),
            ["deviceAppKeyNpub", "label", "requestNpub", "requestSecret"]
                .into_iter()
                .map(str::to_string)
                .collect()
        );
        assert!(
            local.url.len()
                <= nostr_identity::NOSTR_IDENTITY_DEVICE_APPROVAL_BOOTSTRAP_MAX_URI_LENGTH,
            "bootstrap URL was {}",
            local.url.len()
        );
    }

    #[test]
    fn approval_parser_rejects_legacy_full_request_app_key_only_and_suffix_fallbacks() {
        let app_key = nostr_sdk::Keys::generate();
        let app_key_hex = app_key.public_key().to_hex();
        let legacy = format!("iris-drive://app-key-link?app_key={app_key_hex}&ignored=yes");
        assert!(!app_key_approval_input_has_prefix(&legacy));
        assert!(parse_app_key_approval_bootstrap(&legacy).unwrap().is_none());

        let local = create_app_key_approval_bootstrap(&app_key, Some("Phone")).unwrap();
        assert!(
            parse_app_key_approval_bootstrap(&format!("nostr:{}", local.url))
                .unwrap()
                .is_none()
        );
        assert!(
            parse_app_key_approval_bootstrap(&format!("{}?relay=wss://example.test", local.url))
                .is_err()
        );
        assert!(parse_app_key_approval_bootstrap(&format!("{}#scan", local.url)).is_err());

        let mut payload = serde_json::to_value(&local.bootstrap).unwrap();
        payload["requestedAt"] = 123.into();
        let full_request_url = format!(
            "{APP_KEY_APPROVAL_REQUEST_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&payload).unwrap())
        );
        assert!(parse_app_key_approval_bootstrap(&full_request_url).is_err());

        let mut same_key = serde_json::to_value(&local.bootstrap).unwrap();
        same_key["requestNpub"] = same_key["deviceAppKeyNpub"].clone();
        let same_key_url = format!(
            "{APP_KEY_APPROVAL_REQUEST_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&same_key).unwrap())
        );
        assert!(parse_app_key_approval_bootstrap(&same_key_url).is_err());
        assert!(
            parse_app_key_approval_bootstrap("https://drive.iris.to/app-key-linker?owner=x")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn request_frame_rejects_legacy_fields_instead_of_falling_back() {
        let local = create_app_key_approval_bootstrap(&Keys::generate(), Some("Phone")).unwrap();
        let frame = AppKeyLinkRequestFrame {
            schema: 1,
            invite_pubkey: Keys::generate().public_key().to_hex(),
            label: local.bootstrap.label.clone(),
            request_npub: local.bootstrap.request_npub,
            request_secret: local.bootstrap.request_secret,
        };
        let mut value = serde_json::to_value(frame).unwrap();
        value["url"] = local.url.into();
        assert!(serde_json::from_value::<AppKeyLinkRequestFrame>(value).is_err());
    }
}
