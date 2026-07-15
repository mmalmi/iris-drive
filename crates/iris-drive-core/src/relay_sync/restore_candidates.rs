use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use nostr_sdk::{Client, Event, Filter, PublicKey};

use super::{RelayError, fetch_events};
use crate::relay_filters::nostr_identity_roster_op_filter;
use crate::{
    KIND_NOSTR_IDENTITY_ROSTER_OP, NostrIdentityId, SignedNostrIdentityRosterOp,
    nostr_identity_candidate_ids_for_pubkey_from_events, parse_nostr_identity_roster_op_event,
    project_nostr_identity_roster,
};

/// Verified roster evidence for an `NostrIdentity` that a recovery/NIP-46
/// pubkey can use to admit a fresh local `AppKey`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NostrIdentityRestoreCandidate {
    pub profile_id: NostrIdentityId,
    pub recovery_pubkey: String,
    pub can_decrypt_secret_epochs: bool,
    pub accepted_roster_op_count: usize,
    pub active_app_key_count: usize,
    pub latest_roster_op_created_at: Option<i64>,
    pub profile_roster_ops: Vec<SignedNostrIdentityRosterOp>,
}

/// Relay filters for finding `NostrIdentity` evidence involving a recovery key.
///
/// The `#p` filter catches roster ops that mention the key and self-signed
/// acceptance breadcrumbs. The author filter catches events signed by the key.
/// Matching events are discovery hints; callers must still fetch/project the
/// profile roster log before trusting them.
pub fn nostr_identity_restore_candidate_filters(
    recovery_pubkey_hex: &str,
) -> Result<Vec<Filter>, RelayError> {
    let recovery_pubkey = PublicKey::from_hex(recovery_pubkey_hex)
        .map_err(|e| RelayError::InvalidPubkey(e.to_string()))?;
    Ok(vec![
        Filter::new()
            .kind(nostr_sdk::Kind::from(KIND_NOSTR_IDENTITY_ROSTER_OP))
            .pubkey(recovery_pubkey),
        Filter::new()
            .kind(nostr_sdk::Kind::from(KIND_NOSTR_IDENTITY_ROSTER_OP))
            .author(recovery_pubkey),
    ])
}

/// Project fetched relay events into verified restore candidates for a
/// recovery/NIP-46 pubkey. Acceptance events and `p` tags only discover
/// candidate UUIDs; a candidate is returned only when the authoritative roster
/// projection has the pubkey as an active facet that can recover `AppKeys`.
pub fn nostr_identity_restore_candidates_from_events(
    recovery_pubkey_hex: &str,
    events: &[Event],
) -> Result<Vec<NostrIdentityRestoreCandidate>, RelayError> {
    let candidate_ids: BTreeSet<_> =
        nostr_identity_candidate_ids_for_pubkey_from_events(recovery_pubkey_hex, events)?
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
        let Some(facet) = projection.active_facets.get(recovery_pubkey_hex) else {
            continue;
        };
        if !facet.capabilities.can_recover_app_keys {
            continue;
        }
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
        candidates.push(NostrIdentityRestoreCandidate {
            profile_id,
            recovery_pubkey: recovery_pubkey_hex.to_string(),
            can_decrypt_secret_epochs: facet.capabilities.can_decrypt_secret_epochs,
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

/// Fetch restore candidates for a recovery/NIP-46 pubkey from relays.
///
/// This is a two-step query: first find profile IDs through events that mention
/// or are authored by the key, then fetch the full roster op log for each
/// discovered profile ID and project the logs locally.
pub async fn fetch_nostr_identity_restore_candidates(
    client: &Client,
    recovery_pubkey_hex: &str,
    timeout: Duration,
) -> Result<Vec<NostrIdentityRestoreCandidate>, RelayError> {
    let discovery_events = fetch_events(
        client,
        nostr_identity_restore_candidate_filters(recovery_pubkey_hex)?,
        timeout,
    )
    .await?;
    let candidate_ids = nostr_identity_candidate_ids_for_pubkey_from_events(
        recovery_pubkey_hex,
        &discovery_events,
    )?;
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
    nostr_identity_restore_candidates_from_events(recovery_pubkey_hex, &events)
}
