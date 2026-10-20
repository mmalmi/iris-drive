use super::*;
use crate::nostr_identity::{
    NOSTR_IDENTITY_DEVICE_APPROVAL_APPLIED_ACK_SCHEMA, NostrIdentityDeviceApprovalAppliedAck,
    build_nostr_identity_device_approval_applied_ack_event,
};
use nostr_sdk::{EventBuilder, Keys, Kind};
use tempfile::tempdir;

use super::tests::{
    approve_pending_request, config_with_owner_account, filter_matches, queue_link_request,
};

#[test]
fn device_approval_ack_filter_is_limited_to_the_pending_device_and_receipt() {
    let dir = tempdir().unwrap();
    let (_cfg, mut admin) = config_with_owner_account(dir.path());
    let linked_dir = tempdir().unwrap();
    let mut linked = crate::Profile::link_to_profile(
        linked_dir.path(),
        admin.state.profile_id,
        admin.state.app_key_pubkey.clone(),
        Some("Phone".into()),
    )
    .unwrap();
    queue_link_request(&mut linked, &admin, 1);
    let receipt = approve_pending_request(&mut admin, &linked);
    let pending = &admin.state.pending_device_approval_receipts[0];
    // ACK filtering must not depend on synchronized device clocks.
    let ack_time = 1;
    let ack = build_nostr_identity_device_approval_applied_ack_event(
        linked.app_key.keys(),
        NostrIdentityDeviceApprovalAppliedAck {
            schema: NOSTR_IDENTITY_DEVICE_APPROVAL_APPLIED_ACK_SCHEMA,
            request_pubkey: pending.request_pubkey.clone(),
            device_app_key_pubkey: pending.device_app_key_pubkey.clone(),
            approval_event_id: receipt.id.to_hex(),
            approved_by_pubkey: admin.state.app_key_pubkey.clone(),
            applied_at: ack_time,
        },
    )
    .unwrap();
    let generic_policy = event_retention_policy(subscription_filters(
        &admin.state.app_key_pubkey,
        &admin.state.root_scope_id(),
        crate::PRIMARY_DRIVE_ID,
    ));
    assert!(!relay_event_matches_policy(&generic_policy, &ack));

    let filter = pending_device_approval_applied_ack_filter(&admin.state)
        .unwrap()
        .unwrap();
    assert!(filter_matches(&filter, &ack));

    let outsider = Keys::generate();
    let outsider_ack = build_nostr_identity_device_approval_applied_ack_event(
        &outsider,
        NostrIdentityDeviceApprovalAppliedAck {
            schema: NOSTR_IDENTITY_DEVICE_APPROVAL_APPLIED_ACK_SCHEMA,
            request_pubkey: pending.request_pubkey.clone(),
            device_app_key_pubkey: outsider.public_key().to_hex(),
            approval_event_id: receipt.id.to_hex(),
            approved_by_pubkey: admin.state.app_key_pubkey.clone(),
            applied_at: ack_time,
        },
    )
    .unwrap();
    assert!(!filter_matches(&filter, &outsider_ack));

    let unrelated = EventBuilder::new(Kind::TextNote, "")
        .sign_with_keys(admin.app_key.keys())
        .unwrap();
    let wrong_receipt_ack = build_nostr_identity_device_approval_applied_ack_event(
        linked.app_key.keys(),
        NostrIdentityDeviceApprovalAppliedAck {
            schema: NOSTR_IDENTITY_DEVICE_APPROVAL_APPLIED_ACK_SCHEMA,
            request_pubkey: pending.request_pubkey.clone(),
            device_app_key_pubkey: pending.device_app_key_pubkey.clone(),
            approval_event_id: unrelated.id.to_hex(),
            approved_by_pubkey: admin.state.app_key_pubkey.clone(),
            applied_at: ack_time,
        },
    )
    .unwrap();
    assert!(!filter_matches(&filter, &wrong_receipt_ack));

    let policy = event_retention_policy(vec![filter]);
    assert!(relay_event_matches_policy(&policy, &ack));
}
