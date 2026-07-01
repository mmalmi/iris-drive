use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use nostr_sdk::{Client, Event, Filter, PublicKey};

use super::{RelayError, fetch_events};
use crate::app_keys::AppKeysProjection;
use crate::profile::app_keys_from_profile_projection;
use crate::relay_filters::nostr_identity_roster_op_filter;
use crate::{
    KIND_NOSTR_IDENTITY_ROSTER_OP, NostrIdentityId, SignedNostrIdentityRosterOp,
    nostr_identity_candidate_ids_for_pubkey_from_events, parse_nostr_identity_roster_op_event,
    project_nostr_identity_roster,
};

/// Verified roster evidence that can approve a waiting manual AppKey join.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrIdentityAppKeyApprovalCandidate {
    pub profile_id: NostrIdentityId,
    pub app_key_pubkey: String,
    pub admin_app_key_pubkey: String,
    pub accepted_roster_op_count: usize,
    pub active_app_key_count: usize,
    pub latest_roster_op_created_at: Option<i64>,
    pub profile_roster_ops: Vec<SignedNostrIdentityRosterOp>,
}

/// Relay filters for finding roster evidence that mentions a joining AppKey.
pub fn nostr_identity_app_key_approval_candidate_filters(
    app_key_pubkey_hex: &str,
) -> Result<Vec<Filter>, RelayError> {
    let app_key = PublicKey::from_hex(app_key_pubkey_hex)
        .map_err(|e| RelayError::InvalidPubkey(e.to_string()))?;
    Ok(vec![
        Filter::new()
            .kind(nostr_sdk::Kind::from(KIND_NOSTR_IDENTITY_ROSTER_OP))
            .pubkey(app_key),
    ])
}

/// Project fetched relay events into profile rosters that authorize `app_key`.
pub fn nostr_identity_app_key_approval_candidates_from_events(
    app_key_pubkey_hex: &str,
    events: &[Event],
) -> Result<Vec<NostrIdentityAppKeyApprovalCandidate>, RelayError> {
    let candidate_ids: BTreeSet<_> =
        nostr_identity_candidate_ids_for_pubkey_from_events(app_key_pubkey_hex, events)?
            .into_iter()
            .collect();
    let mut roster_ops_by_profile =
        BTreeMap::<NostrIdentityId, BTreeMap<String, SignedNostrIdentityRosterOp>>::new();
    for event in events {
        let Ok(op) = parse_nostr_identity_roster_op_event(event) else {
            continue;
        };
        if candidate_ids.contains(&op.content.profile_id) {
            roster_ops_by_profile
                .entry(op.content.profile_id)
                .or_default()
                .insert(op.op_id.clone(), op);
        }
    }

    let mut candidates = Vec::new();
    for profile_id in candidate_ids {
        let profile_roster_ops = roster_ops_by_profile
            .remove(&profile_id)
            .unwrap_or_default()
            .into_values()
            .collect::<Vec<_>>();
        let projection = project_nostr_identity_roster(profile_id, profile_roster_ops.clone());
        if !projection.can_write_roots(app_key_pubkey_hex) {
            continue;
        }
        let Some(app_keys) = app_keys_from_profile_projection(&projection) else {
            continue;
        };
        if !app_keys.contains(app_key_pubkey_hex) {
            continue;
        }
        let Some(admin_app_key_pubkey) = app_key_approval_candidate_admin(&app_keys) else {
            continue;
        };
        let accepted_op_ids = projection
            .accepted_op_ids
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let latest_roster_op_created_at = profile_roster_ops
            .iter()
            .filter(|op| accepted_op_ids.contains(op.op_id.as_str()))
            .map(|op| op.content.created_at)
            .max();
        candidates.push(NostrIdentityAppKeyApprovalCandidate {
            profile_id,
            app_key_pubkey: app_key_pubkey_hex.to_string(),
            admin_app_key_pubkey,
            accepted_roster_op_count: projection.accepted_op_ids.len(),
            active_app_key_count: projection.active_app_key_pubkeys().len(),
            latest_roster_op_created_at,
            profile_roster_ops,
        });
    }
    candidates.sort_by(|left, right| {
        right
            .latest_roster_op_created_at
            .cmp(&left.latest_roster_op_created_at)
            .then_with(|| {
                right
                    .accepted_roster_op_count
                    .cmp(&left.accepted_roster_op_count)
            })
            .then_with(|| right.active_app_key_count.cmp(&left.active_app_key_count))
            .then_with(|| left.profile_id.cmp(&right.profile_id))
    });
    Ok(candidates)
}

fn app_key_approval_candidate_admin(app_keys: &AppKeysProjection) -> Option<String> {
    app_keys
        .signer_pubkey()
        .filter(|signer| app_keys.is_admin(signer))
        .map(ToOwned::to_owned)
        .or_else(|| {
            app_keys
                .app_actors
                .iter()
                .find(|actor| actor.is_admin())
                .map(|actor| actor.pubkey.clone())
        })
}

/// Fetch profile rosters that approve a waiting AppKey from relays.
pub async fn fetch_nostr_identity_app_key_approval_candidates(
    client: &Client,
    app_key_pubkey_hex: &str,
    timeout: Duration,
) -> Result<Vec<NostrIdentityAppKeyApprovalCandidate>, RelayError> {
    let discovery_events = fetch_events(
        client,
        nostr_identity_app_key_approval_candidate_filters(app_key_pubkey_hex)?,
        timeout,
    )
    .await?;
    let candidate_ids =
        nostr_identity_candidate_ids_for_pubkey_from_events(app_key_pubkey_hex, &discovery_events)?;
    if candidate_ids.is_empty() {
        return Ok(Vec::new());
    }
    let roster_events = fetch_events(
        client,
        candidate_ids
            .into_iter()
            .map(nostr_identity_roster_op_filter)
            .collect::<Vec<_>>(),
        timeout,
    )
    .await?;
    let mut events = discovery_events;
    events.extend(roster_events);
    nostr_identity_app_key_approval_candidates_from_events(app_key_pubkey_hex, &events)
}
