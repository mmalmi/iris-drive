use std::time::Duration;

use nostr_sdk::{Event, Filter, JsonUtil, PublicKey, SingleLetterTag};

use super::{RelayError, fetch_events};
use crate::nostr_identity::parse_nostr_identity_device_approval_applied_ack_event;

pub(super) type PendingDeviceApprovalAckSubscription = (Vec<(String, String)>, Filter);

pub async fn fetch_device_approval_applied_ack_events(
    client: &nostr_sdk::Client,
    state: &crate::ProfileState,
    timeout: Duration,
) -> Result<Vec<Event>, RelayError> {
    let Some(filter) = pending_device_approval_applied_ack_filter(state)? else {
        return Ok(Vec::new());
    };
    let events = fetch_events(client, vec![filter], timeout).await?;
    Ok(events
        .into_iter()
        .filter(|event| parse_nostr_identity_device_approval_applied_ack_event(event).is_ok())
        .collect())
}

/// Build one bounded relay filter for ACKs matching the currently pending
/// approval signers and receipt event IDs. Exact signer/receipt pairing is
/// validated again when each ACK is applied.
pub fn pending_device_approval_applied_ack_filter(
    state: &crate::ProfileState,
) -> Result<Option<Filter>, RelayError> {
    pending_device_approval_applied_ack_subscription(state)
        .map(|subscription| subscription.map(|(_, filter)| filter))
}

pub(super) fn pending_device_approval_applied_ack_subscription(
    state: &crate::ProfileState,
) -> Result<Option<PendingDeviceApprovalAckSubscription>, RelayError> {
    if state.pending_device_approval_receipts.is_empty() || !state.can_admin_profile() {
        return Ok(None);
    }
    let admin_app_key = PublicKey::from_hex(&state.app_key_pubkey)
        .map_err(|error| RelayError::InvalidPubkey(error.to_string()))?;
    let mut coordinates = Vec::with_capacity(state.pending_device_approval_receipts.len());
    let mut authors = Vec::with_capacity(coordinates.capacity());
    let mut approval_event_ids = Vec::with_capacity(coordinates.capacity());
    for pending in &state.pending_device_approval_receipts {
        let approval_event = Event::from_json(&pending.event_json)
            .map_err(|error| RelayError::AppKeyLinkRoster(error.to_string()))?;
        let device_app_key = PublicKey::from_hex(&pending.device_app_key_pubkey)
            .map_err(|error| RelayError::InvalidPubkey(error.to_string()))?;
        coordinates.push((device_app_key.to_hex(), approval_event.id.to_hex()));
        authors.push(device_app_key);
        approval_event_ids.push(approval_event.id);
    }
    coordinates.sort_unstable();
    coordinates.dedup();
    let filter = Filter::new()
        .kind(nostr_sdk::Kind::from(crate::KIND_NOSTR_IDENTITY_ROSTER_OP))
        .custom_tag(
            SingleLetterTag::lowercase(nostr_sdk::Alphabet::P),
            admin_app_key.to_hex(),
        )
        .authors(authors)
        .events(approval_event_ids)
        .limit(crate::profile::MAX_PENDING_DEVICE_APPROVAL_RECEIPTS * 2);
    Ok(Some((coordinates, filter)))
}
