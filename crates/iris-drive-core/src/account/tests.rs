use super::*;
use crate::iris_profile::{IrisProfileKeyPurpose, KeyWrapStatus};
use tempfile::tempdir;

#[test]
fn create_yields_admin_authorized_account() {
    let dir = tempdir().unwrap();
    let acct = Account::create(dir.path(), Some("my-laptop".into())).unwrap();
    let phrase = crate::recovery_phrase::load_recovery_phrase(
        crate::paths::recovery_phrase_path_in(dir.path()),
    )
    .unwrap();
    let recovery_keys =
        OwnerKey::from_recovery_phrase(&phrase, dir.path().join("recovery")).unwrap();
    assert!(acct.state.can_manage_devices());
    assert!(acct.state.is_authorized());
    assert!(acct.state.can_write_roots());
    assert!(acct.owner_key.is_none());
    assert_ne!(
        acct.state.device_pubkey,
        recovery_keys.pubkey_hex(),
        "the 12-word recovery phrase must not be the app key"
    );
    assert_eq!(
        acct.state.profile_id,
        crate::recovery_phrase::recovery_phrase_to_profile_id(&phrase).unwrap()
    );
    // Roster admin authority is not a second key.
    assert!(dir.path().join("key").exists());
    assert!(dir.path().join("recovery_phrase").exists());
    assert!(!dir.path().join("owner_key").exists());
    // AppKeys lists one device — this one.
    let snap = acct.state.app_keys.as_ref().unwrap();
    assert_eq!(snap.profile_id, acct.state.profile_id.to_string());
    assert_eq!(snap.app_actors.len(), 1);
    assert_eq!(snap.app_actors[0].pubkey, acct.state.device_pubkey);
    assert!(snap.app_actors[0].is_admin());
    assert_eq!(
        snap.signer_pubkey(),
        Some(acct.state.device_pubkey.as_str())
    );
    assert!(!acct.state.profile_roster_ops.is_empty());
    assert!(acct.state.profile_roster_ops.iter().all(|op| {
        op.signer_pubkey == acct.state.device_pubkey
            || op.signer_pubkey == recovery_keys.pubkey_hex()
    }));

    let projection = acct.state.profile_projection();
    assert!(projection.can_write_roots(&acct.state.device_pubkey));
    assert!(projection.can_admin_profile(&acct.state.device_pubkey));
    assert!(!projection.can_write_roots(&recovery_keys.pubkey_hex()));
    assert!(projection.can_admin_profile(&recovery_keys.pubkey_hex()));
    assert_eq!(projection.key_epochs.len(), 1);
    assert_eq!(
        acct.current_dck_from_recovery_phrase(&phrase).unwrap(),
        acct.current_dck().unwrap()
    );
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
fn empty_device_label_uses_pubkey_label() {
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
    assert_eq!(restored.state.device_pubkey, original.state.device_pubkey);
    assert_ne!(restored.state.profile_id, original.state.profile_id);
    assert!(restored.state.can_manage_devices());
    assert!(
        restored
            .state
            .app_keys
            .as_ref()
            .unwrap()
            .is_admin(&restored.state.device_pubkey)
    );
    assert!(!dir_b.path().join("owner_key").exists());
    assert!(!dir_b.path().join("recovery_phrase").exists());
}

#[test]
fn restore_from_recovery_phrase_preserves_profile_and_export_phrase() {
    let dir_a = tempdir().unwrap();
    let original = Account::create(dir_a.path(), None).unwrap();
    let phrase = crate::recovery_phrase::load_recovery_phrase(
        crate::paths::recovery_phrase_path_in(dir_a.path()),
    )
    .unwrap();
    assert_eq!(phrase.split_whitespace().count(), 12);

    let dir_b = tempdir().unwrap();
    let restored = Account::restore(dir_b.path(), &phrase, None).unwrap();
    assert_eq!(restored.state.profile_id, original.state.profile_id);
    assert_ne!(
        restored.state.device_pubkey, original.state.device_pubkey,
        "recovery creates a fresh app key instead of cloning the old one"
    );
    assert_eq!(
        crate::recovery_phrase::load_recovery_phrase(crate::paths::recovery_phrase_path_in(
            dir_b.path()
        ))
        .unwrap(),
        phrase
    );
}

#[test]
fn recovery_phrase_admits_fresh_app_key_into_existing_profile_log() {
    let owner_dir = tempdir().unwrap();
    let mut owner = Account::create(owner_dir.path(), Some("native".into())).unwrap();
    let phrase = crate::recovery_phrase::load_recovery_phrase(
        crate::paths::recovery_phrase_path_in(owner_dir.path()),
    )
    .unwrap();
    let recovery_key =
        OwnerKey::from_recovery_phrase(&phrase, owner_dir.path().join("recovery")).unwrap();
    let old_owner_dck = owner.current_dck().unwrap();

    let recovered_dir = tempdir().unwrap();
    let recovered_device = DeviceIdentity::generate(recovered_dir.path().join("key"));
    recovered_device.save().unwrap();
    let recovered_pubkey = recovered_device.pubkey_hex();
    let mut recovered = Account {
        state: AccountState {
            profile_id: owner.state.profile_id,
            device_pubkey: recovered_pubkey.clone(),
            profile_roster_ops: owner.state.profile_roster_ops.clone(),
            device_link_secret: "recover-secret".into(),
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label: Some("web app".into()),
            app_keys: None,
            outbound_device_link_request: None,
            inbound_device_link_requests: Vec::new(),
        },
        device: recovered_device,
        owner_key: None,
    };

    let snap = recovered
        .admit_current_app_key_with_recovery_phrase(&phrase, None)
        .unwrap();

    assert!(snap.is_admin(&recovered_pubkey));
    assert_eq!(
        snap.signer_pubkey(),
        Some(recovery_key.pubkey_hex().as_str())
    );
    assert!(recovered.state.is_authorized());
    assert!(recovered.state.can_manage_devices());
    let projection = recovered.state.profile_projection();
    assert!(projection.can_write_roots(&recovered_pubkey));
    assert!(projection.can_admin_profile(&recovered_pubkey));
    assert!(!projection.can_write_roots(&recovery_key.pubkey_hex()));
    assert!(projection.can_admin_profile(&recovery_key.pubkey_hex()));
    assert_eq!(projection.key_epochs.keys().next_back().copied(), Some(2));

    let recovered_dck = recovered.current_dck().unwrap();
    assert_ne!(recovered_dck, old_owner_dck);

    owner.state.profile_roster_ops = recovered.state.profile_roster_ops.clone();
    owner.state.sync_app_keys_from_profile();
    assert_eq!(owner.current_dck().unwrap(), recovered_dck);
}

#[test]
fn admin_can_configure_nip46_recovery_with_epoch_decrypt_wrap() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), Some("native".into())).unwrap();
    let nip46 = Keys::generate();
    let nip46_pubkey = nip46.public_key().to_hex();

    acct.add_nip46_recovery(&nip46_pubkey, Some("bunker".into()), true)
        .unwrap();

    let projection = acct.state.profile_projection();
    let facet = projection.active_facets.get(&nip46_pubkey).unwrap();
    assert!(facet.has_purpose(IrisProfileKeyPurpose::Nip46Signer));
    assert!(!projection.can_write_roots(&nip46_pubkey));
    assert!(projection.can_admin_profile(&nip46_pubkey));
    let latest_epoch = projection.key_epochs.keys().next_back().copied().unwrap();
    assert_eq!(
        projection.key_wrap_status(&nip46_pubkey, latest_epoch),
        KeyWrapStatus::Available
    );
    assert_eq!(
        acct.current_dck_from_nip46_keys(&nip46).unwrap(),
        acct.current_dck().unwrap()
    );
}

#[test]
fn nip46_authority_admits_fresh_app_key_with_decrypt_wrap() {
    let owner_dir = tempdir().unwrap();
    let mut owner = Account::create(owner_dir.path(), Some("native".into())).unwrap();
    let nip46 = Keys::generate();
    let nip46_pubkey = nip46.public_key().to_hex();
    owner
        .add_nip46_recovery(&nip46_pubkey, Some("bunker".into()), true)
        .unwrap();
    let old_owner_dck = owner.current_dck().unwrap();

    let recovered_dir = tempdir().unwrap();
    let recovered_device = DeviceIdentity::generate(recovered_dir.path().join("key"));
    recovered_device.save().unwrap();
    let recovered_pubkey = recovered_device.pubkey_hex();
    let mut recovered = Account {
        state: AccountState {
            profile_id: owner.state.profile_id,
            device_pubkey: recovered_pubkey.clone(),
            profile_roster_ops: owner.state.profile_roster_ops.clone(),
            device_link_secret: "recover-secret".into(),
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label: Some("web app".into()),
            app_keys: None,
            outbound_device_link_request: None,
            inbound_device_link_requests: Vec::new(),
        },
        device: recovered_device,
        owner_key: None,
    };

    let snap = recovered
        .admit_current_app_key_with_nip46_keys(&nip46, None)
        .unwrap();

    assert!(snap.is_admin(&recovered_pubkey));
    assert_eq!(snap.signer_pubkey(), Some(nip46_pubkey.as_str()));
    assert!(recovered.state.is_authorized());
    assert!(recovered.state.can_manage_devices());
    let projection = recovered.state.profile_projection();
    assert!(projection.can_write_roots(&recovered_pubkey));
    assert!(projection.can_admin_profile(&recovered_pubkey));
    assert!(!projection.can_write_roots(&nip46_pubkey));
    assert_eq!(projection.key_epochs.keys().next_back().copied(), Some(3));

    let recovered_dck = recovered.current_dck().unwrap();
    assert_ne!(recovered_dck, old_owner_dck);
    owner.state.profile_roster_ops = recovered.state.profile_roster_ops.clone();
    owner.state.sync_app_keys_from_profile();
    assert_eq!(owner.current_dck().unwrap(), recovered_dck);
}

#[test]
fn nip46_without_decrypt_admits_app_key_but_leaves_wrap_repair_needed() {
    let owner_dir = tempdir().unwrap();
    let mut owner = Account::create(owner_dir.path(), Some("native".into())).unwrap();
    let nip46 = Keys::generate();
    let nip46_pubkey = nip46.public_key().to_hex();
    owner
        .add_nip46_recovery(&nip46_pubkey, Some("signer only".into()), false)
        .unwrap();

    let recovered_dir = tempdir().unwrap();
    let recovered_device = DeviceIdentity::generate(recovered_dir.path().join("key"));
    recovered_device.save().unwrap();
    let recovered_pubkey = recovered_device.pubkey_hex();
    let mut recovered = Account {
        state: AccountState {
            profile_id: owner.state.profile_id,
            device_pubkey: recovered_pubkey.clone(),
            profile_roster_ops: owner.state.profile_roster_ops.clone(),
            device_link_secret: "recover-secret".into(),
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label: Some("web app".into()),
            app_keys: None,
            outbound_device_link_request: None,
            inbound_device_link_requests: Vec::new(),
        },
        device: recovered_device,
        owner_key: None,
    };

    let snap = recovered
        .admit_current_app_key_with_nip46_keys(&nip46, None)
        .unwrap();

    assert!(snap.contains(&recovered_pubkey));
    assert_eq!(
        snap.signer_pubkey(),
        Some(owner.state.device_pubkey.as_str())
    );
    let projection = recovered.state.profile_projection();
    assert!(projection.can_write_roots(&recovered_pubkey));
    assert_eq!(projection.key_epochs.keys().next_back().copied(), Some(1));
    assert_eq!(
        projection.key_wrap_status(&recovered_pubkey, 1),
        KeyWrapStatus::RepairNeeded
    );
    assert!(matches!(
        recovered.current_dck(),
        Err(AccountError::NoWrapForThisDevice)
    ));
}

#[test]
fn epoch_signing_admin_can_repair_missing_app_key_wraps() {
    let owner_dir = tempdir().unwrap();
    let mut owner = Account::create(owner_dir.path(), Some("native".into())).unwrap();
    let nip46 = Keys::generate();
    owner
        .add_nip46_recovery(
            &nip46.public_key().to_hex(),
            Some("signer only".into()),
            false,
        )
        .unwrap();
    let owner_dck = owner.current_dck().unwrap();

    let recovered_dir = tempdir().unwrap();
    let recovered_device = DeviceIdentity::generate(recovered_dir.path().join("key"));
    recovered_device.save().unwrap();
    let recovered_pubkey = recovered_device.pubkey_hex();
    let mut recovered = Account {
        state: AccountState {
            profile_id: owner.state.profile_id,
            device_pubkey: recovered_pubkey.clone(),
            profile_roster_ops: owner.state.profile_roster_ops.clone(),
            device_link_secret: "recover-secret".into(),
            authorization_state: DeviceAuthorizationState::AwaitingApproval,
            device_label: Some("web app".into()),
            app_keys: None,
            outbound_device_link_request: None,
            inbound_device_link_requests: Vec::new(),
        },
        device: recovered_device,
        owner_key: None,
    };
    recovered
        .admit_current_app_key_with_nip46_keys(&nip46, None)
        .unwrap();

    owner.state.profile_roster_ops = recovered.state.profile_roster_ops.clone();
    owner.state.sync_app_keys_from_profile();
    assert_eq!(
        owner
            .state
            .profile_projection()
            .active_key_recipients_missing_wraps(1),
        vec![recovered_pubkey.clone()]
    );

    let repair = owner.repair_current_key_epoch_wraps().unwrap();

    assert_eq!(repair.epoch, 1);
    assert_eq!(repair.repaired_pubkeys, vec![recovered_pubkey.clone()]);
    assert_eq!(owner.current_dck().unwrap(), owner_dck);
    assert!(
        owner
            .state
            .profile_projection()
            .active_key_recipients_missing_wraps(1)
            .is_empty()
    );

    recovered.state.profile_roster_ops = owner.state.profile_roster_ops.clone();
    recovered.state.sync_app_keys_from_profile();
    assert_eq!(recovered.current_dck().unwrap(), owner_dck);
}

#[test]
fn link_starts_awaiting_approval_no_owner_key() {
    let dir = tempdir().unwrap();
    let profile_id = IrisProfileId::new_v4();
    let admin_app_key = fresh_device_pubkey();
    let acct = Account::link_to_profile(
        dir.path(),
        profile_id,
        admin_app_key.clone(),
        Some("phone".into()),
    )
    .unwrap();
    assert_eq!(acct.state.profile_id, profile_id);
    assert_ne!(acct.state.device_pubkey, admin_app_key);
    assert!(!acct.state.can_manage_devices());
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
    let profile_id = acct.state.profile_id;
    let link_secret = acct.state.device_link_secret.clone();
    let device = fresh_device_pubkey();

    assert!(
        acct.state
            .record_inbound_device_link_request(
                profile_id,
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
                profile_id,
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
                profile_id,
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
    let profile_id = acct.state.profile_id;
    let device = fresh_device_pubkey();

    assert!(
        !acct
            .state
            .record_inbound_device_link_request(
                profile_id,
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
fn reset_device_link_secret_rotates_invite_and_clears_pending_requests() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    let profile_id = acct.state.profile_id;
    let old_secret = acct.state.device_link_secret.clone();
    let device = fresh_device_pubkey();

    acct.state
        .record_inbound_device_link_request(
            profile_id,
            &device,
            Some("phone".to_string()),
            &old_secret,
            10,
        )
        .unwrap();

    assert!(acct.state.reset_device_link_secret());
    assert_ne!(acct.state.device_link_secret, old_secret);
    assert!(acct.state.inbound_device_link_requests.is_empty());

    assert!(
        !acct
            .state
            .record_inbound_device_link_request(
                profile_id,
                &fresh_device_pubkey(),
                Some("old".to_string()),
                &old_secret,
                11,
            )
            .unwrap()
    );
}

#[test]
fn link_with_invalid_pubkey_errors() {
    let dir = tempdir().unwrap();
    let result = Account::link_to_profile(
        dir.path(),
        IrisProfileId::new_v4(),
        "not-a-real-pubkey".into(),
        None,
    );
    match result {
        Err(AccountError::InvalidAppKeyPubkey(_)) => {}
        other => panic!("expected InvalidAppKeyPubkey, got {:?}", other.is_ok()),
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
    assert_eq!(snap.app_actors.len(), 2);
    assert!(snap.contains(&new_device));

    let projection = acct.state.profile_projection();
    assert!(projection.can_write_roots(&new_device));
    assert!(!projection.can_admin_profile(&new_device));
    assert!(projection.active_facets.contains_key(&new_device));
    let latest_epoch = projection.key_epochs.values().next_back().unwrap();
    assert!(latest_epoch.wrapped_dck.contains_key(&new_device));
}

#[test]
fn approve_without_admin_authority_errors() {
    let dir = tempdir().unwrap();
    // Use a real x-only pubkey hex; the test only ever fails on the authority
    // check before reaching crypto, so this is fine.
    let admin_app_key = fresh_device_pubkey();
    let mut acct =
        Account::link_to_profile(dir.path(), IrisProfileId::new_v4(), admin_app_key, None).unwrap();
    match acct.approve_device(&fresh_device_pubkey(), None) {
        Err(AccountError::NoAdminAuthority) => {}
        _ => panic!("expected NoAdminAuthority"),
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

    let projection = acct.state.profile_projection();
    assert!(!projection.active_facets.contains_key(&target));
    assert!(projection.tombstones.contains_key(&target));
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
    assert!(acct.state.profile_projection().can_admin_profile(&target));

    let snap = acct.demote_admin(&target).unwrap();
    assert!(!snap.is_admin(&target));
    assert!(snap.is_admin(&current));
    let projection = acct.state.profile_projection();
    assert!(!projection.can_admin_profile(&target));
    assert!(projection.can_admin_profile(&current));
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
fn current_dck_comes_from_profile_epoch_without_snapshot_adapter() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    let expected = acct.current_dck().unwrap();
    acct.state.app_keys = None;
    assert_eq!(acct.current_dck().unwrap(), expected);
}

#[test]
fn authorization_recomputes_from_profile_without_snapshot_adapter() {
    let dir = tempdir().unwrap();
    let mut acct = Account::create(dir.path(), None).unwrap();
    acct.state.app_keys = None;
    acct.state.authorization_state = DeviceAuthorizationState::AwaitingApproval;

    acct.state.recompute_authorization();

    assert_eq!(
        acct.state.authorization_state,
        DeviceAuthorizationState::Authorized
    );
    assert!(acct.state.can_manage_devices());
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
    assert_eq!(snap.wrapped_dck.len(), snap.app_actors.len());
    for d in &snap.app_actors {
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
        .app_actors
        .iter()
        .map(|d| d.pubkey.clone())
        .collect();
    acct.rotate_dck().unwrap();
    let latest_epoch = acct
        .state
        .profile_projection()
        .key_epochs
        .keys()
        .next_back()
        .copied()
        .unwrap();
    assert_eq!(latest_epoch, 3);
    let devices_after: Vec<_> = acct
        .state
        .app_keys
        .as_ref()
        .unwrap()
        .app_actors
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
fn rotate_dck_without_admin_authority_errors() {
    let dir = tempdir().unwrap();
    let admin_app_key = fresh_device_pubkey();
    let mut acct =
        Account::link_to_profile(dir.path(), IrisProfileId::new_v4(), admin_app_key, None).unwrap();
    match acct.rotate_dck() {
        Err(AccountError::NoAdminAuthority) => {}
        other => panic!("expected NoAdminAuthority, got {:?}", other.is_ok()),
    }
}

#[test]
fn current_dck_without_snapshot_errors() {
    let dir = tempdir().unwrap();
    let admin_app_key = fresh_device_pubkey();
    let acct =
        Account::link_to_profile(dir.path(), IrisProfileId::new_v4(), admin_app_key, None).unwrap();
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
        profile_id: owner_acct.state.profile_id,
        device_pubkey: linked_pubkey.clone(),
        profile_roster_ops: owner_acct.state.profile_roster_ops.clone(),
        device_link_secret: "linked-secret".into(),
        authorization_state: DeviceAuthorizationState::Authorized,
        device_label: Some("phone".into()),
        app_keys: snapshot_for_linked,
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
        profile_id: owner_acct.state.profile_id,
        device_pubkey: linked_pubkey,
        profile_roster_ops: owner_acct.state.profile_roster_ops.clone(),
        device_link_secret: "linked-secret".into(),
        authorization_state: DeviceAuthorizationState::Revoked,
        device_label: None,
        app_keys: owner_acct.state.app_keys.clone(),
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
    let tombstone = signed_profile_roster_op_with_parents(
        acct.device.keys(),
        acct.state.profile_id,
        iris_profile_roster_parent_ids(&acct.state.profile_roster_ops),
        IrisProfileRosterOp::TombstoneFacet {
            pubkey: acct.state.device_pubkey.clone(),
            reason: Some("external revocation".to_owned()),
        },
        next_profile_timestamp(&acct.state),
    )
    .unwrap();

    acct.state.profile_roster_ops.push(tombstone);
    acct.state.recompute_authorization();

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
    let admin_app_key = fresh_device_pubkey();
    let linked =
        Account::link_to_profile(dir.path(), IrisProfileId::new_v4(), admin_app_key, None).unwrap();
    let state = linked.state.clone();
    let loaded = Account::load(state, dir.path()).unwrap();
    assert!(loaded.owner_key.is_none());
}
