use super::*;
use tempfile::tempdir;

#[test]
fn create_yields_admin_authorized_account() {
    let dir = tempdir().unwrap();
    let acct = Account::create(dir.path(), Some("my-laptop".into())).unwrap();
    assert!(acct.state.has_owner_signing_authority);
    assert!(acct.state.can_manage_devices());
    assert!(acct.state.is_authorized());
    assert!(acct.owner_key.is_none());
    assert_eq!(acct.state.owner_pubkey, acct.state.device_pubkey);
    // Only the device key exists; roster admin authority is not a second key.
    assert!(dir.path().join("key").exists());
    assert!(!dir.path().join("owner_key").exists());
    // AppKeys lists one device — this one.
    let snap = acct.state.app_keys.as_ref().unwrap();
    assert_eq!(snap.devices.len(), 1);
    assert_eq!(snap.devices[0].pubkey, acct.state.device_pubkey);
    assert!(snap.devices[0].is_admin());
    assert_eq!(snap.signer_pubkey(), acct.state.device_pubkey);
    let record = acct.state.app_keys_event.as_ref().unwrap();
    assert_eq!(record.signer_pubkey, acct.state.device_pubkey);
    assert!(!record.event_json.is_empty());
}

#[test]
fn default_device_label_prefers_hostname() {
    assert_eq!(
        normalize_hostname_label("Example Mac mini.local").as_deref(),
        Some("Example Mac mini")
    );
    assert_eq!(
        normalize_hostname_label("localhost.localdomain").as_deref(),
        None
    );
    assert_eq!(normalize_hostname_label("2ce2e39b4cf9.local"), None);
}

#[test]
fn empty_device_label_falls_back_to_pubkey_label() {
    let pubkey = "abcdef12".to_string() + &"34".repeat(28);
    assert_eq!(
        resolve_device_label_with_hostname(Some("   ".into()), None, &pubkey).as_deref(),
        Some("device abcdef12")
    );
}

#[test]
fn restore_uses_provided_admin_device_nsec() {
    let dir_a = tempdir().unwrap();
    let original = Account::create(dir_a.path(), None).unwrap();
    let nsec = original.device.keys().secret_key().to_secret_hex();

    let dir_b = tempdir().unwrap();
    let restored = Account::restore(dir_b.path(), &nsec, None).unwrap();
    assert_eq!(restored.state.owner_pubkey, original.state.owner_pubkey);
    assert_eq!(restored.state.device_pubkey, original.state.device_pubkey);
    assert!(restored.state.has_owner_signing_authority);
    assert!(
        restored
            .state
            .app_keys
            .as_ref()
            .unwrap()
            .is_admin(&restored.state.device_pubkey)
    );
    assert!(!dir_b.path().join("owner_key").exists());
}

#[test]
fn link_starts_awaiting_approval_no_owner_key() {
    let dir = tempdir().unwrap();
    // Fake owner npub (64 hex chars).
    let owner = "ab".repeat(32);
    let acct = Account::link(dir.path(), owner.clone(), Some("phone".into())).unwrap();
    assert_eq!(acct.state.owner_pubkey, owner);
    assert!(!acct.state.has_owner_signing_authority);
    assert_eq!(
        acct.state.authorization_state,
        DeviceAuthorizationState::AwaitingApproval
    );
    assert!(acct.owner_key.is_none());
    // owner_key file does NOT exist on a linked install.
    assert!(!dir.path().join("owner_key").exists());
}

#[test]
fn inbound_device_link_requests_are_deduped_and_bounded() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    let owner = acct.state.owner_pubkey.clone();
    let link_secret = acct.state.device_link_secret.clone();
    let device = fresh_device_pubkey();

    assert!(
        acct.state
            .record_inbound_device_link_request(
                &owner,
                &device,
                Some(" phone ".to_string()),
                &link_secret,
                10,
            )
            .unwrap()
    );
    assert_eq!(acct.state.inbound_device_link_requests.len(), 1);
    assert_eq!(
        acct.state.inbound_device_link_requests[0].label.as_deref(),
        Some("phone")
    );

    assert!(
        !acct
            .state
            .record_inbound_device_link_request(
                &owner,
                &device,
                Some("phone".to_string()),
                &link_secret,
                9,
            )
            .unwrap()
    );
    assert!(
        acct.state
            .record_inbound_device_link_request(
                &owner,
                &device,
                Some("tablet".to_string()),
                &link_secret,
                11,
            )
            .unwrap()
    );
    assert_eq!(acct.state.inbound_device_link_requests.len(), 1);
    assert_eq!(
        acct.state.inbound_device_link_requests[0].label.as_deref(),
        Some("tablet")
    );
}

#[test]
fn inbound_device_link_request_requires_link_secret() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    let owner = acct.state.owner_pubkey.clone();
    let device = fresh_device_pubkey();

    assert!(
        !acct
            .state
            .record_inbound_device_link_request(
                &owner,
                &device,
                Some("phone".to_string()),
                "wrong-secret",
                10,
            )
            .unwrap()
    );
    assert!(acct.state.inbound_device_link_requests.is_empty());
}

#[test]
fn link_with_invalid_pubkey_errors() {
    let dir = tempdir().unwrap();
    let result = Account::link(dir.path(), "not-a-real-pubkey".into(), None);
    match result {
        Err(AccountError::InvalidOwnerPubkey(_)) => {}
        other => panic!("expected InvalidOwnerPubkey, got {:?}", other.is_ok()),
    }
}

/// Helper: produce a valid secp256k1 x-only pubkey hex for tests.
/// Random fake hex strings often fail NIP-44 because only ~half of
/// 32-byte values lie on the curve.
fn fresh_device_pubkey() -> String {
    Keys::generate().public_key().to_hex()
}

#[test]
fn approve_adds_device_to_roster() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    let new_device = fresh_device_pubkey();
    let snap = acct
        .approve_device(&new_device, Some("phone".into()))
        .unwrap();
    assert_eq!(snap.devices.len(), 2);
    assert!(snap.contains(&new_device));
}

#[test]
fn approve_without_owner_authority_errors() {
    let dir = tempdir().unwrap();
    // Use a real x-only pubkey hex; the test only ever fails on the authority
    // check before reaching crypto, so this is fine.
    let owner = fresh_device_pubkey();
    let mut acct = Account::link(dir.path(), owner, None).unwrap();
    match acct.approve_device(&fresh_device_pubkey(), None) {
        Err(AccountError::NoOwnerAuthority) => {}
        _ => panic!("expected NoOwnerAuthority"),
    }
}

#[test]
fn approving_already_authorized_device_errors() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    let current = acct.state.device_pubkey.clone();
    match acct.approve_device(&current, None) {
        Err(AccountError::AlreadyAuthorized) => {}
        _ => panic!("expected AlreadyAuthorized"),
    }
}

#[test]
fn revoke_removes_device_from_roster() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    let target = fresh_device_pubkey();
    acct.approve_device(&target, None).unwrap();
    let snap = acct.revoke_device(&target).unwrap();
    assert!(!snap.contains(&target));
}

#[test]
fn revoke_missing_device_errors() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    // Pubkey is well-formed but not in the roster.
    let stranger = fresh_device_pubkey();
    match acct.revoke_device(&stranger) {
        Err(AccountError::DeviceNotInRoster) => {}
        _ => panic!("expected DeviceNotInRoster"),
    }
}

#[test]
fn appoint_and_demote_admin_updates_roster_roles() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    let current = acct.state.device_pubkey.clone();
    let target = fresh_device_pubkey();
    acct.approve_device(&target, None).unwrap();

    let snap = acct.appoint_admin(&target).unwrap();
    assert!(snap.is_admin(&target));
    assert!(snap.dck_generation >= 3);

    let snap = acct.demote_admin(&target).unwrap();
    assert!(!snap.is_admin(&target));
    assert!(snap.is_admin(&current));
}

#[test]
fn cannot_demote_last_admin() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    let current = acct.state.device_pubkey.clone();
    match acct.demote_admin(&current) {
        Err(AccountError::CannotRemoveLastAdmin) => {}
        other => panic!("expected CannotRemoveLastAdmin, got {:?}", other.is_ok()),
    }
}

// ------------ DCK rotation / forward secrecy tests ------------

#[test]
fn create_seeds_dck_generation_one_with_self_wrap() {
    let dir = tempdir().unwrap();
    let acct = Account::create(dir.path(), None).unwrap();
    let snap = acct.state.app_keys.as_ref().unwrap();
    assert_eq!(snap.dck_generation, 1);
    // One wrap, for the current device.
    assert_eq!(snap.wrapped_dck.len(), 1);
    assert!(snap.wrapped_dck.contains_key(&acct.state.device_pubkey));
}

#[test]
fn current_dck_is_decryptable_by_owner_device() {
    let dir = tempdir().unwrap();
    let acct = Account::create(dir.path(), None).unwrap();
    let dck = acct.current_dck().unwrap();
    assert_eq!(dck.len(), 32);
    // Two reads return same key (state is deterministic).
    let dck2 = acct.current_dck().unwrap();
    assert_eq!(dck, dck2);
}

#[test]
fn approve_rotates_dck_generation_and_wraps_to_all_devices() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    let gen_before = acct.state.app_keys.as_ref().unwrap().dck_generation;
    let new_device = fresh_device_pubkey();
    let snap = acct
        .approve_device(&new_device, Some("phone".into()))
        .unwrap();
    assert!(snap.dck_generation > gen_before);
    // Every authorized device has a wrap.
    assert_eq!(snap.wrapped_dck.len(), snap.devices.len());
    for d in &snap.devices {
        assert!(snap.wrapped_dck.contains_key(&d.pubkey));
    }
}

#[test]
fn revoke_rotates_dck_and_drops_revoked_device_wrap() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    let target = fresh_device_pubkey();
    let own_device_pubkey = acct.state.device_pubkey.clone();
    acct.approve_device(&target, None).unwrap();
    let gen_before = acct.state.app_keys.as_ref().unwrap().dck_generation;
    let snap = acct.revoke_device(&target).unwrap();
    assert!(snap.dck_generation > gen_before);
    // Revoked device no longer has a wrap.
    assert!(!snap.wrapped_dck.contains_key(&target));
    // Remaining device(s) still have wraps.
    assert!(snap.wrapped_dck.contains_key(&own_device_pubkey));
}

#[test]
fn dck_changes_after_rotation() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    let dck_before = acct.current_dck().unwrap();
    acct.rotate_dck().unwrap();
    let dck_after = acct.current_dck().unwrap();
    assert_ne!(dck_before, dck_after);
}

#[test]
fn rotate_dck_preserves_roster() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    let new_device = fresh_device_pubkey();
    acct.approve_device(&new_device, None).unwrap();
    let devices_before: Vec<_> = acct
        .state
        .app_keys
        .as_ref()
        .unwrap()
        .devices
        .iter()
        .map(|d| d.pubkey.clone())
        .collect();
    acct.rotate_dck().unwrap();
    let devices_after: Vec<_> = acct
        .state
        .app_keys
        .as_ref()
        .unwrap()
        .devices
        .iter()
        .map(|d| d.pubkey.clone())
        .collect();
    assert_eq!(devices_before, devices_after);
    // Both devices still have a wrap for the new DCK.
    for d in &devices_after {
        assert!(
            acct.state
                .app_keys
                .as_ref()
                .unwrap()
                .wrapped_dck
                .contains_key(d)
        );
    }
}

#[test]
fn rotate_dck_without_owner_authority_errors() {
    let dir = tempdir().unwrap();
    let owner = fresh_device_pubkey();
    let mut acct = Account::link(dir.path(), owner, None).unwrap();
    match acct.rotate_dck() {
        Err(AccountError::NoOwnerAuthority) => {}
        other => panic!("expected NoOwnerAuthority, got {:?}", other.is_ok()),
    }
}

#[test]
fn current_dck_without_snapshot_errors() {
    let dir = tempdir().unwrap();
    let owner = fresh_device_pubkey();
    let acct = Account::link(dir.path(), owner, None).unwrap();
    match acct.current_dck() {
        Err(AccountError::NoCurrentSnapshot) => {}
        other => panic!("expected NoCurrentSnapshot, got {:?}", other.is_ok()),
    }
}

#[test]
fn linked_device_with_approved_wrap_decrypts_same_dck_as_owner() {
    // This is the end-to-end crypto test: owner creates,
    // owner approves a *real* device keypair, the device then
    // independently decrypts its wrap and recovers the same DCK
    // the owner has.
    let owner_dir = tempdir().unwrap();
    let mut owner_acct = Account::create(owner_dir.path(), None).unwrap();

    // Manually create a "linked device" keypair we control end-to-end.
    let linked_dir = tempdir().unwrap();
    let linked_device = DeviceIdentity::generate(linked_dir.path().join("key"));
    linked_device.save().unwrap();
    let linked_pubkey = linked_device.pubkey_hex();

    // Owner approves the device's pubkey.
    owner_acct
        .approve_device(&linked_pubkey, Some("phone".into()))
        .unwrap();
    let owner_dck = owner_acct.current_dck().unwrap();

    // Reconstruct an Account from the linked device's perspective:
    // device key is the one we generated; AccountState mirrors what
    // the device would see after pulling the latest snapshot.
    let snapshot_for_linked = owner_acct.state.app_keys.clone();
    let linked_state = AccountState {
        owner_pubkey: owner_acct.state.owner_pubkey.clone(),
        device_pubkey: linked_pubkey.clone(),
        device_link_secret: "linked-secret".into(),
        has_owner_signing_authority: false,
        authorization_state: DeviceAuthorizationState::Authorized,
        device_label: Some("phone".into()),
        app_keys: snapshot_for_linked,
        app_keys_event: None,
        outbound_device_link_request: None,
        inbound_device_link_requests: Vec::new(),
    };
    let linked_acct = Account {
        state: linked_state,
        device: linked_device,
        owner_key: None,
    };

    let linked_dck = linked_acct.current_dck().unwrap();
    assert_eq!(
        owner_dck, linked_dck,
        "linked device must derive the same DCK the owner does"
    );
}

#[test]
fn revoked_device_cannot_decrypt_new_dck() {
    // Owner approves linked device, sees a DCK, then revokes it.
    // After revoke, the linked device should fail current_dck()
    // because its wrap is no longer present.
    let owner_dir = tempdir().unwrap();
    let mut owner_acct = Account::create(owner_dir.path(), None).unwrap();
    let linked_dir = tempdir().unwrap();
    let linked_device = DeviceIdentity::generate(linked_dir.path().join("key"));
    linked_device.save().unwrap();
    let linked_pubkey = linked_device.pubkey_hex();

    owner_acct.approve_device(&linked_pubkey, None).unwrap();
    owner_acct.revoke_device(&linked_pubkey).unwrap();

    let linked_state = AccountState {
        owner_pubkey: owner_acct.state.owner_pubkey.clone(),
        device_pubkey: linked_pubkey,
        device_link_secret: "linked-secret".into(),
        has_owner_signing_authority: false,
        authorization_state: DeviceAuthorizationState::Revoked,
        device_label: None,
        app_keys: owner_acct.state.app_keys.clone(),
        app_keys_event: None,
        outbound_device_link_request: None,
        inbound_device_link_requests: Vec::new(),
    };
    let linked_acct = Account {
        state: linked_state,
        device: linked_device,
        owner_key: None,
    };
    match linked_acct.current_dck() {
        Err(AccountError::NoWrapForThisDevice) => {}
        other => panic!("expected NoWrapForThisDevice, got {:?}", other.is_ok()),
    }
}

#[test]
fn external_revocation_marks_state_revoked() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    assert!(acct.state.is_authorized());
    // Pretend a new snapshot from owner removes this device.
    let new_snap = AppKeysSnapshot {
        owner_pubkey: acct.state.owner_pubkey.clone(),
        signed_by_pubkey: Some(acct.state.device_pubkey.clone()),
        created_at: acct.state.app_keys.as_ref().unwrap().created_at + 1,
        devices: vec![DeviceEntry::member("ff".repeat(32), 0, None)],
        dck_generation: acct.state.app_keys.as_ref().unwrap().dck_generation + 1,
        wrapped_dck: BTreeMap::new(),
    };
    acct.state.apply_app_keys(new_snap);
    assert_eq!(
        acct.state.authorization_state,
        DeviceAuthorizationState::Revoked
    );
}

#[test]
fn load_round_trips_account_state() {
    let dir = tempdir().unwrap();
    let created = Account::create(dir.path(), Some("desktop".into())).unwrap();
    let state = created.state.clone();
    let loaded = Account::load(state.clone(), dir.path()).unwrap();
    assert_eq!(loaded.state, state);
    assert_eq!(loaded.device.pubkey_hex(), created.device.pubkey_hex());
    assert!(loaded.owner_key.is_none());
}

#[test]
fn load_for_linked_device_skips_owner_key() {
    let dir = tempdir().unwrap();
    let owner = "ab".repeat(32);
    let linked = Account::link(dir.path(), owner, None).unwrap();
    let state = linked.state.clone();
    let loaded = Account::load(state, dir.path()).unwrap();
    assert!(loaded.owner_key.is_none());
}
