use nostr_sdk::{Event, JsonUtil};

use super::RelayError;
use crate::app_key_link_transport::parse_pending_app_key_approval_receipt_event;
use crate::{NostrIdentityRosterProjection, ProfileState};

pub(super) fn ensure_profile_roster_bootstrap_unchanged(
    account: &ProfileState,
    merged: &NostrIdentityRosterProjection,
) -> Result<(), RelayError> {
    // A profile UUID is public and is not proof of control. The protocol
    // projector chooses its first self-signed admin op by timestamp, so an
    // untrusted, backdated bootstrap must never replace the established one.
    let current = account.profile_projection();
    if let Some(bootstrap_id) = current.accepted_op_ids.first()
        && merged.accepted_op_ids.first() != Some(bootstrap_id)
    {
        return Err(RelayError::AppKeyLinkRoster(
            "incoming roster changes the established profile bootstrap".into(),
        ));
    }
    Ok(())
}

pub(super) fn pending_device_approval_receipt_is_valid(account: &ProfileState) -> bool {
    let Some(pending) = account.outbound_app_key_link_request.as_ref() else {
        return false;
    };
    pending.approval_receipt_event.iter().any(|event_json| {
        Event::from_json(event_json).is_ok_and(|event| {
            parse_pending_app_key_approval_receipt_event(pending, &event).is_ok()
        })
    })
}
