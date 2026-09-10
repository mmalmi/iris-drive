use super::tests::{approve_pending_request, profile_event, queue_link_request};
use super::*;
use crate::profile::AppKeyAuthorizationState;
use crate::{AppConfig, Drive, Profile};
use nostr_sdk::Keys;
use tempfile::tempdir;

#[test]
fn retained_approval_receipt_cannot_override_later_roster_revocation() {
    let admin_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let mut admin = Profile::create(admin_dir.path(), Some("admin".into())).unwrap();
    let mut linked = Profile::link_to_profile(
        linked_dir.path(),
        admin.state.profile_id,
        admin.state.app_key_pubkey.clone(),
        Some("phone".into()),
    )
    .unwrap();
    queue_link_request(&mut linked, &admin, 123);
    let receipt = approve_pending_request(&mut admin, &linked);
    let mut config = AppConfig {
        profile: Some(linked.state),
        ..AppConfig::default()
    };
    apply_remote_device_approval_receipt_event(&mut config, &receipt).unwrap();
    let state = config.profile.as_mut().unwrap();

    state.recompute_authorization();
    assert!(state.is_authorized(), "the receipt enables roster backfill");
    assert!(state.app_keys.is_none());

    state.profile_roster_ops = admin.state.profile_roster_ops.clone();
    state.recompute_authorization();
    assert!(
        state.is_authorized(),
        "the complete roster authorizes the key"
    );

    admin.revoke_app_key(&state.app_key_pubkey).unwrap();
    state.profile_roster_ops = admin.state.profile_roster_ops.clone();
    state.recompute_authorization();
    assert_eq!(state.authorization_state, AppKeyAuthorizationState::Revoked);
    assert!(
        crate::app_key_link_transport::pending_app_key_approval_receipt_authorizes_app_key(
            state.outbound_app_key_link_request.as_ref().unwrap(),
            &state.app_key_pubkey,
        ),
        "the still-valid bootstrap receipt cannot supersede a signed tombstone"
    );
}

#[test]
fn device_approval_receipt_clears_awaiting_approval_before_full_roster_frame() {
    let admin_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let mut admin = Profile::create(admin_dir.path(), Some("admin".into())).unwrap();
    let mut linked = Profile::link_to_profile(
        linked_dir.path(),
        admin.state.profile_id,
        admin.state.app_key_pubkey.clone(),
        Some("phone".into()),
    )
    .unwrap();
    queue_link_request(&mut linked, &admin, 123);
    let receipt_event = approve_pending_request(&mut admin, &linked);
    let mut cfg = AppConfig {
        profile: Some(linked.state.clone()),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(admin.state.root_scope_id()));

    let outcome = apply_remote_device_approval_receipt_event(&mut cfg, &receipt_event).unwrap();

    assert_eq!(outcome, NostrIdentityRosterOpApply::Applied);
    let linked_state = cfg.profile.as_ref().unwrap();
    assert_eq!(
        linked_state.authorization_state,
        AppKeyAuthorizationState::Authorized
    );
    assert!(linked_state.can_write_roots());
    assert!(linked_state.can_write_roots_for_app_key(&linked_state.app_key_pubkey));
    assert!(linked_state.outbound_app_key_link_request.is_some());
    assert!(!linked_state.profile_roster_ops.is_empty());
    assert!(linked_state.app_keys.is_none());
    assert!(apply_device_approval_roster_backfill_events(&mut cfg, &[]).is_err());

    let mut wrong_approver = cfg.clone();
    let state = wrong_approver.profile.as_mut().unwrap();
    state.profile_roster_ops.clear();
    let mut roster = admin.state.current_app_keys_projection().unwrap();
    roster
        .app_actors
        .iter_mut()
        .find(|actor| actor.pubkey == admin.state.app_key_pubkey)
        .unwrap()
        .pubkey = Keys::generate().public_key().to_hex();
    state.app_keys = Some(roster);
    assert!(
        apply_device_approval_roster_backfill_events(&mut wrong_approver, &[]).is_err(),
        "a second actor cannot stand in for the device that signed the approval receipt"
    );

    let backfill = admin
        .state
        .profile_roster_ops
        .iter()
        .map(profile_event)
        .collect::<Vec<_>>();
    let mut rotations = 0;
    let without_approval_epoch = backfill
        .iter()
        .filter(|event| {
            let parsed = crate::parse_nostr_identity_roster_op_event(event).unwrap();
            if matches!(
                parsed.content.op,
                crate::NostrIdentityRosterOp::RotateSecretEpoch { .. }
            ) {
                rotations += 1;
                return rotations == 1;
            }
            true
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut missing_approval_epoch = cfg.clone();
    assert!(
        apply_device_approval_roster_backfill_events(
            &mut missing_approval_epoch,
            &without_approval_epoch,
        )
        .is_err(),
        "the joining device must have a wrap in the latest applied key epoch"
    );
    assert!(apply_device_approval_roster_backfill_events(&mut cfg, &backfill).unwrap() > 0);
    let linked_state = cfg.profile.as_ref().unwrap();
    let roster = linked_state.app_keys.as_ref().unwrap();
    assert_eq!(roster.app_actors.len(), 2);
    assert!(roster.contains(&linked_state.app_key_pubkey));
    assert!(roster.contains(&admin.state.app_key_pubkey));

    let path = linked_dir.path().join("config.toml");
    cfg.save(&path).unwrap();
    let loaded = AppConfig::load_or_default(&path).unwrap();
    let loaded_state = loaded.profile.as_ref().unwrap();
    assert_eq!(
        loaded_state.authorization_state,
        AppKeyAuthorizationState::Authorized
    );
    assert!(loaded_state.can_write_roots());
    assert!(loaded_state.can_write_roots_for_app_key(&loaded_state.app_key_pubkey));
    assert!(loaded_state.outbound_app_key_link_request.is_some());
}
