use nostr_sdk::{Client, SubscriptionId};

use super::RelayError;
use crate::NostrIdentityId;
use crate::relay_filters::{device_approval_receipt_subscription, nostr_identity_roster_op_filter};

const DEVICE_APPROVAL_SUBSCRIPTION_ID: &str = "iris-drive-device-approval";
const PROFILE_ROSTER_SUBSCRIPTION_ID: &str = "iris-drive-profile-roster";

/// Tracks the identity-scoped relay subscriptions that must follow config
/// changes while a daemon or mobile sync worker stays alive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppKeyLinkRelaySubscriptionState {
    approval_request_pubkey: Option<String>,
    profile_id: Option<NostrIdentityId>,
    approval_uses_dynamic_id: bool,
}

impl AppKeyLinkRelaySubscriptionState {
    #[must_use]
    pub fn from_profile(state: &crate::ProfileState) -> Self {
        Self {
            approval_request_pubkey: device_approval_receipt_subscription(state)
                .map(|(request_pubkey, _)| request_pubkey),
            profile_id: Some(state.profile_id),
            approval_uses_dynamic_id: false,
        }
    }
}

/// Refresh request- and profile-scoped subscriptions after an in-process
/// identity mutation. This lets a daemon started before a join request receive
/// its approval, then backfill the complete bound profile roster.
pub async fn refresh_app_key_link_relay_subscriptions(
    client: &Client,
    state: &crate::ProfileState,
    subscriptions: &mut AppKeyLinkRelaySubscriptionState,
) -> Result<bool, RelayError> {
    let mut changed = false;
    let approval = device_approval_receipt_subscription(state);
    let approval_request_pubkey = approval
        .as_ref()
        .map(|(request_pubkey, _)| request_pubkey.clone());
    if approval_request_pubkey != subscriptions.approval_request_pubkey {
        let subscription_id = SubscriptionId::new(DEVICE_APPROVAL_SUBSCRIPTION_ID);
        if let Some((_, filter)) = approval {
            client
                .subscribe_with_id(subscription_id, filter, None)
                .await
                .map_err(|error| RelayError::Client(error.to_string()))?;
            subscriptions.approval_uses_dynamic_id = true;
        } else if subscriptions.approval_uses_dynamic_id {
            client.unsubscribe(&subscription_id).await;
            subscriptions.approval_uses_dynamic_id = false;
        }
        subscriptions.approval_request_pubkey = approval_request_pubkey;
        changed = true;
    }

    if subscriptions.profile_id != Some(state.profile_id) {
        client
            .subscribe_with_id(
                SubscriptionId::new(PROFILE_ROSTER_SUBSCRIPTION_ID),
                nostr_identity_roster_op_filter(state.profile_id),
                None,
            )
            .await
            .map_err(|error| RelayError::Client(error.to_string()))?;
        subscriptions.profile_id = Some(state.profile_id);
        changed = true;
    }
    Ok(changed)
}
