use std::time::Duration;

use nostr_sdk::{Client, Event, Filter};

use super::{RelayError, fetch_events};
use crate::nostr_identity_candidate_ids_for_pubkey_from_events;
use crate::relay_filters::nostr_identity_roster_op_filter;
pub use nostr_identity::NostrIdentityAppKeyApprovalCandidate;

/// Relay filters for finding roster evidence that mentions a joining `AppKey`.
pub fn nostr_identity_app_key_approval_candidate_filters(
    app_key_pubkey_hex: &str,
) -> Result<Vec<Filter>, RelayError> {
    nostr_identity::nostr_identity_app_key_approval_candidate_filters(app_key_pubkey_hex)
        .map_err(RelayError::from)
}

/// Project fetched relay events into profile rosters that authorize `app_key`.
pub fn nostr_identity_app_key_approval_candidates_from_events(
    app_key_pubkey_hex: &str,
    events: &[Event],
) -> Result<Vec<NostrIdentityAppKeyApprovalCandidate>, RelayError> {
    nostr_identity::nostr_identity_app_key_approval_candidates_from_events(
        app_key_pubkey_hex,
        events,
    )
    .map_err(RelayError::from)
}

/// Fetch profile rosters that approve a waiting `AppKey` from relays.
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
