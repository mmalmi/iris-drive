use std::time::Duration;

use nostr_sdk::{Client, Event, JsonUtil};

use super::{
    NostrIdentityRosterOpApply, RelayError, apply_remote_nostr_identity_roster_op_event,
    fetch_nostr_identity_roster_ops,
};
use crate::AppConfig;

pub const DEVICE_APPROVAL_ROSTER_BACKFILL_TIMEOUT: Duration = Duration::from_secs(5);

/// Apply the relay roster history that may have arrived just before an
/// approval receipt. The linked device is not ready to acknowledge approval
/// until the causal roster projects both the approving and joining devices.
pub fn apply_device_approval_roster_backfill_events(
    config: &mut AppConfig,
    events: &[Event],
) -> Result<usize, RelayError> {
    let mut applied = 0;
    for event in events {
        if matches!(
            apply_remote_nostr_identity_roster_op_event(config, event)?,
            NostrIdentityRosterOpApply::Applied
        ) {
            applied += 1;
        }
    }
    let state = config.profile.as_ref().ok_or(RelayError::NoAccount)?;
    let complete = approval_roster_is_causally_complete(state);
    if state.authorization_state == crate::AppKeyAuthorizationState::Authorized && !complete {
        return Err(RelayError::AppKeyLinkRoster(
            "device approval roster backfill is incomplete".to_string(),
        ));
    }
    Ok(applied)
}

fn approval_roster_is_causally_complete(state: &crate::ProfileState) -> bool {
    let Some(pending) = state.outbound_app_key_link_request.as_ref() else {
        return false;
    };
    if pending.approval_receipt_event.is_empty() {
        return false;
    }
    let projection = state.profile_projection();
    pending.approval_receipt_event.iter().all(|receipt_json| {
        let Ok(receipt_event) = Event::from_json(receipt_json) else {
            return false;
        };
        let Ok(receipt) =
            crate::app_key_link_transport::parse_pending_app_key_approval_receipt_event(
                pending,
                &receipt_event,
            )
        else {
            return false;
        };
        let Ok(receipt_op) =
            crate::nostr_identity::parse_nostr_identity_device_approval_receipt_roster_op(&receipt)
        else {
            return false;
        };
        projection.accepted_op_ids.contains(&receipt_op.op_id)
            && projection.can_admin_profile(&receipt.approved_by_pubkey)
    }) && projection.can_write_roots(&state.app_key_pubkey)
        && projection
            .secret_epochs
            .values()
            .next_back()
            .is_some_and(|epoch| epoch.wrapped_secrets.contains_key(&state.app_key_pubkey))
}

/// Fetch and apply a newly approved device's causal roster history under one
/// bounded relay deadline.
pub async fn backfill_device_approval_roster(
    client: &Client,
    config: &mut AppConfig,
    timeout: Duration,
) -> Result<usize, RelayError> {
    let profile_id = config
        .profile
        .as_ref()
        .ok_or(RelayError::NoAccount)?
        .profile_id;
    let events = fetch_nostr_identity_roster_ops(client, profile_id, timeout).await?;
    apply_device_approval_roster_backfill_events(config, &events)
}
