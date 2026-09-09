use nostr_sdk::{Client, SubscriptionId};

use super::RelayError;
use super::approval_ack::pending_device_approval_applied_ack_subscription;
use crate::NostrIdentityId;
use crate::PRIMARY_DRIVE_ID;
use crate::relay_filters::{
    device_approval_receipt_subscription, drive_root_filter, nostr_identity_roster_op_filter,
};

const DEVICE_APPROVAL_SUBSCRIPTION_ID: &str = "iris-drive-device-approval";
const DEVICE_APPROVAL_ACK_SUBSCRIPTION_ID: &str = "iris-drive-device-approval-ack";
const PROFILE_ROSTER_SUBSCRIPTION_ID: &str = "iris-drive-profile-roster";
const PRIMARY_DRIVE_ROOT_SUBSCRIPTION_ID: &str = "iris-drive-primary-root";

/// Tracks the identity-scoped relay subscriptions that must follow config
/// changes while a daemon or mobile sync worker stays alive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppKeyLinkRelaySubscriptionState {
    approval_request_pubkey: Option<String>,
    profile_id: Option<NostrIdentityId>,
    approval_uses_dynamic_id: bool,
    approval_ack_coordinates: Vec<(String, String)>,
    approval_ack_uses_dynamic_id: bool,
    profile_was_awaiting_approval: bool,
    drive_root_scope_id: String,
}

impl AppKeyLinkRelaySubscriptionState {
    #[must_use]
    pub fn from_profile(state: &crate::ProfileState) -> Self {
        Self {
            approval_request_pubkey: device_approval_receipt_subscription(state)
                .map(|(request_pubkey, _)| request_pubkey),
            profile_id: Some(state.profile_id),
            approval_uses_dynamic_id: false,
            approval_ack_coordinates: Vec::new(),
            approval_ack_uses_dynamic_id: false,
            profile_was_awaiting_approval: state.authorization_state
                == crate::AppKeyAuthorizationState::AwaitingApproval,
            drive_root_scope_id: state.root_scope_id(),
        }
    }

    fn roster_subscription_needs_refresh(&self, state: &crate::ProfileState) -> bool {
        self.profile_id != Some(state.profile_id)
            || (self.profile_was_awaiting_approval
                && state.authorization_state != crate::AppKeyAuthorizationState::AwaitingApproval)
    }

    fn drive_root_subscription_needs_refresh(&self, state: &crate::ProfileState) -> bool {
        self.drive_root_scope_id != state.root_scope_id()
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
    if client.relays().await.is_empty() {
        return Ok(false);
    }
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

    let approval_ack = pending_device_approval_applied_ack_subscription(state)?;
    let approval_ack_coordinates = approval_ack
        .as_ref()
        .map(|(coordinates, _)| coordinates.clone())
        .unwrap_or_default();
    if approval_ack_coordinates != subscriptions.approval_ack_coordinates {
        let subscription_id = SubscriptionId::new(DEVICE_APPROVAL_ACK_SUBSCRIPTION_ID);
        if let Some((_, filter)) = approval_ack {
            client
                .subscribe_with_id(subscription_id, filter, None)
                .await
                .map_err(|error| RelayError::Client(error.to_string()))?;
            subscriptions.approval_ack_uses_dynamic_id = true;
        } else if subscriptions.approval_ack_uses_dynamic_id {
            client.unsubscribe(&subscription_id).await;
            subscriptions.approval_ack_uses_dynamic_id = false;
        }
        subscriptions.approval_ack_coordinates = approval_ack_coordinates;
        changed = true;
    }

    if subscriptions.roster_subscription_needs_refresh(state) {
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
    let drive_root_scope_id = state.root_scope_id();
    if subscriptions.drive_root_subscription_needs_refresh(state) {
        client
            .subscribe_with_id(
                SubscriptionId::new(PRIMARY_DRIVE_ROOT_SUBSCRIPTION_ID),
                drive_root_filter(&drive_root_scope_id, PRIMARY_DRIVE_ID),
                None,
            )
            .await
            .map_err(|error| RelayError::Client(error.to_string()))?;
        subscriptions.drive_root_scope_id = drive_root_scope_id;
        changed = true;
    }
    subscriptions.profile_was_awaiting_approval =
        state.authorization_state == crate::AppKeyAuthorizationState::AwaitingApproval;
    Ok(changed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Profile;
    use tempfile::tempdir;

    #[tokio::test]
    async fn empty_relay_pool_keeps_subscription_state_pending() {
        let config_dir = tempdir().unwrap();
        let profile = Profile::create(config_dir.path(), None).unwrap();
        let client = super::super::connect(&[]).await.unwrap();
        let mut subscriptions = AppKeyLinkRelaySubscriptionState::default();
        let before = subscriptions.clone();
        assert!(
            !refresh_app_key_link_relay_subscriptions(&client, &profile.state, &mut subscriptions)
                .await
                .unwrap()
        );
        assert_eq!(subscriptions, before);
        super::super::shutdown_client(&client).await;
    }

    #[test]
    fn approval_completion_requires_roster_backfill_subscription() {
        let owner_dir = tempdir().unwrap();
        let linked_dir = tempdir().unwrap();
        let owner = Profile::create(owner_dir.path(), Some("owner".into())).unwrap();
        let linked = Profile::link_to_profile(
            linked_dir.path(),
            owner.state.profile_id,
            owner.state.app_key_pubkey,
            Some("linked".into()),
        )
        .unwrap();
        let subscriptions = AppKeyLinkRelaySubscriptionState::from_profile(&linked.state);
        let mut approved = linked.state;
        approved.authorization_state = crate::AppKeyAuthorizationState::Authorized;

        assert!(subscriptions.roster_subscription_needs_refresh(&approved));
    }

    #[test]
    fn unbound_approval_requires_drive_root_subscription_refresh() {
        let owner_dir = tempdir().unwrap();
        let linked_dir = tempdir().unwrap();
        let owner = Profile::create(owner_dir.path(), Some("owner".into())).unwrap();
        let linked = Profile::start_join_request(linked_dir.path(), Some("linked".into())).unwrap();
        let subscriptions = AppKeyLinkRelaySubscriptionState::from_profile(&linked.state);
        let mut approved = linked.state;
        approved.profile_id = owner.state.profile_id;

        assert!(subscriptions.drive_root_subscription_needs_refresh(&approved));
    }
}
