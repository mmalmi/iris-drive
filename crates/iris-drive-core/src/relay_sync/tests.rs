use super::*;
use crate::account::{Account, DeviceAuthorizationState};
use crate::config::Drive;
use crate::nostr_events::{
    build_app_keys_event, build_drive_root_event, build_private_hashtree_root_event,
};
use hashtree_core::Cid;
use tempfile::tempdir;

fn config_with_owner_account(dir: &std::path::Path) -> (AppConfig, Account) {
    let acct = Account::create(dir, None).unwrap();
    let mut cfg = AppConfig {
        account: Some(acct.state.clone()),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(acct.state.owner_pubkey.clone()));
    (cfg, acct)
}

fn encrypted_root(seed: u8, published_at: i64, dck_generation: u64) -> DeviceRootRef {
    DeviceRootRef::legacy(
        Cid::encrypted([seed; 32], [seed.wrapping_add(1); 32]).to_string(),
        published_at,
        dck_generation,
    )
}

fn causal_encrypted_root(
    seed: u8,
    published_at: i64,
    dck_generation: u64,
    device_seq: u64,
) -> DeviceRootRef {
    DeviceRootRef {
        root_cid: Cid::encrypted([seed; 32], [seed.wrapping_add(1); 32]).to_string(),
        published_at,
        dck_generation,
        device_seq,
        parents: Vec::new(),
        observed: std::collections::BTreeMap::new(),
        materialized_only: false,
    }
}

#[test]
fn apply_app_keys_event_from_our_owner_replaces() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());

    // Owner approves a fake device — produces a newer snapshot.
    let new_device = Keys::generate().public_key().to_hex();
    acct.approve_device(new_device, None).unwrap();
    let newer_snap = acct.state.app_keys.clone().unwrap();
    let event = build_app_keys_event(acct.owner_key.as_ref().unwrap().keys(), &newer_snap).unwrap();

    // Older state in config.
    let outcome = apply_remote_app_keys_event(&mut cfg, &event).unwrap();
    let applied = matches!(
        outcome,
        AppKeysApply::Applied(ApplyDecision::Replaced | ApplyDecision::Adopted)
    );
    assert!(applied, "unexpected outcome {outcome:?}");
    assert_eq!(
        cfg.account
            .as_ref()
            .unwrap()
            .app_keys
            .as_ref()
            .unwrap()
            .devices
            .len(),
        2,
    );
}

#[test]
fn apply_app_keys_event_from_attacker_ignored() {
    let dir = tempdir().unwrap();
    let (mut cfg, _) = config_with_owner_account(dir.path());
    let attacker = Keys::generate();
    // Attacker publishes their own AppKeys claiming to be the owner —
    // we ignore because their pubkey isn't our owner.
    let mut snap = cfg.account.as_ref().unwrap().app_keys.clone().unwrap();
    snap.owner_pubkey = attacker.public_key().to_hex();
    snap.created_at = i64::MAX;
    let event = build_app_keys_event(&attacker, &snap).unwrap();
    let outcome = apply_remote_app_keys_event(&mut cfg, &event).unwrap();
    assert_eq!(outcome, AppKeysApply::NotOurOwner);
}

#[test]
fn apply_drive_root_event_from_authorized_device_applies() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());

    // Approve a second device whose Keys we control end-to-end.
    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    acct.approve_device(device_b_hex.clone(), None).unwrap();
    cfg.account = Some(acct.state.clone());

    // Device B publishes a drive-root event.
    let root = encrypted_root(0xab, 0, 1);
    let event = build_drive_root_event(
        &device_b,
        &acct.state.owner_pubkey,
        "main",
        &root,
        &[acct.state.device_pubkey.clone(), device_b_hex.clone()],
    )
    .unwrap();
    let outcome =
        apply_remote_drive_root_event(&mut cfg, &event, Some(acct.device.keys())).unwrap();
    assert_eq!(outcome, DriveRootApply::Applied);

    let drive = cfg.drive("main").unwrap();
    let entry = drive.device_roots.get(&device_b_hex).unwrap();
    assert_eq!(entry.root_cid, root.root_cid);
    assert!(entry.published_at > 0); // came from event.created_at
}

#[test]
fn apply_drive_root_event_without_local_wrap_is_skipped() {
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let (_, mut owner_acct) = config_with_owner_account(owner_dir.path());

    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    owner_acct
        .approve_device(device_b_hex.clone(), Some("old-phone".into()))
        .unwrap();

    let linked = Account::link(
        linked_dir.path(),
        owner_acct.state.owner_pubkey.clone(),
        Some("new-laptop".into()),
    )
    .unwrap();
    let mut linked_state = linked.state.clone();
    linked_state.app_keys = owner_acct.state.app_keys.clone();

    let mut cfg = AppConfig {
        account: Some(linked_state),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(owner_acct.state.owner_pubkey.clone()));

    let root = encrypted_root(0xac, 0, 1);
    let event = build_drive_root_event(
        &device_b,
        &owner_acct.state.owner_pubkey,
        "main",
        &root,
        &[owner_acct.state.device_pubkey.clone(), device_b_hex],
    )
    .unwrap();
    let outcome =
        apply_remote_drive_root_event(&mut cfg, &event, Some(linked.device.keys())).unwrap();

    assert_eq!(outcome, DriveRootApply::KeyUnavailable);
    assert!(cfg.drive("main").unwrap().device_roots.is_empty());
}

#[test]
fn apply_files_root_event_from_owner_maps_to_current_device() {
    let dir = tempdir().unwrap();
    let (mut cfg, acct) = config_with_owner_account(dir.path());
    let root = encrypted_root(0x5a, 1_700_000_000, 0);
    let event =
        build_private_hashtree_root_event(acct.owner_key.as_ref().unwrap().keys(), "main", &root)
            .unwrap();

    let outcome = apply_remote_files_root_event(
        &mut cfg,
        &event,
        Some(acct.owner_key.as_ref().unwrap().keys()),
    )
    .unwrap();

    assert_eq!(outcome, FilesRootApply::Applied);
    let entry = cfg
        .drive("main")
        .unwrap()
        .device_roots
        .get(&acct.state.device_pubkey)
        .unwrap();
    assert_eq!(entry.root_cid, root.root_cid);
    assert_eq!(entry.dck_generation, 0);
    assert_eq!(
        cfg.drive("main").unwrap().last_root_cid.as_deref(),
        Some(root.root_cid.as_str())
    );
}

#[test]
fn apply_files_root_event_does_not_replace_causal_native_root() {
    let dir = tempdir().unwrap();
    let (mut cfg, acct) = config_with_owner_account(dir.path());
    let native_root = causal_encrypted_root(0x5b, 100, 1, 4);
    cfg.drives[0]
        .device_roots
        .insert(acct.state.device_pubkey.clone(), native_root.clone());
    let legacy_root = encrypted_root(0x5c, 1_700_000_000, 0);
    let event = build_private_hashtree_root_event(
        acct.owner_key.as_ref().unwrap().keys(),
        "main",
        &legacy_root,
    )
    .unwrap();

    let outcome = apply_remote_files_root_event(
        &mut cfg,
        &event,
        Some(acct.owner_key.as_ref().unwrap().keys()),
    )
    .unwrap();

    assert_eq!(outcome, FilesRootApply::StaleTimestamp);
    let entry = cfg
        .drive("main")
        .unwrap()
        .device_roots
        .get(&acct.state.device_pubkey)
        .unwrap();
    assert_eq!(entry.root_cid, native_root.root_cid);
    assert_eq!(entry.device_seq, 4);
}

#[test]
fn apply_files_root_event_ignores_same_root_with_newer_timestamp() {
    let dir = tempdir().unwrap();
    let (mut cfg, acct) = config_with_owner_account(dir.path());
    let mut root = encrypted_root(0x5d, 100, 0);
    let owner_keys = acct.owner_key.as_ref().unwrap().keys();
    let first = build_private_hashtree_root_event(owner_keys, "main", &root).unwrap();

    assert_eq!(
        apply_remote_files_root_event(&mut cfg, &first, Some(owner_keys)).unwrap(),
        FilesRootApply::Applied
    );

    root.published_at = 200;
    let republished = build_private_hashtree_root_event(owner_keys, "main", &root).unwrap();

    assert_eq!(
        apply_remote_files_root_event(&mut cfg, &republished, Some(owner_keys)).unwrap(),
        FilesRootApply::StaleTimestamp
    );
    let entry = cfg
        .drive("main")
        .unwrap()
        .device_roots
        .get(&acct.state.device_pubkey)
        .unwrap();
    assert_eq!(entry.root_cid, root.root_cid);
    assert_eq!(entry.published_at, 100);
}

#[test]
fn apply_files_root_event_from_foreign_owner_ignored() {
    let dir = tempdir().unwrap();
    let (mut cfg, _) = config_with_owner_account(dir.path());
    let other_owner = Keys::generate();
    let root = encrypted_root(0x61, 1_700_000_000, 0);
    let event = build_private_hashtree_root_event(&other_owner, "main", &root).unwrap();

    let outcome = apply_remote_files_root_event(&mut cfg, &event, Some(&other_owner)).unwrap();

    assert_eq!(outcome, FilesRootApply::NotOurOwner);
    assert!(cfg.drive("main").unwrap().device_roots.is_empty());
}

#[test]
fn apply_drive_root_event_from_unauthorized_device_ignored() {
    let dir = tempdir().unwrap();
    let (mut cfg, _) = config_with_owner_account(dir.path());
    let stranger = Keys::generate(); // not in roster

    let root = encrypted_root(0xee, 0, 99);
    let owner_hex = cfg.account.as_ref().unwrap().owner_pubkey.clone();
    let recipient = cfg.account.as_ref().unwrap().device_pubkey.clone();
    let outcome = {
        let event =
            build_drive_root_event(&stranger, &owner_hex, "main", &root, &[recipient]).unwrap();
        apply_remote_drive_root_event(&mut cfg, &event, None).unwrap()
    };
    assert_eq!(outcome, DriveRootApply::UnauthorizedDevice);
    assert!(cfg.drive("main").unwrap().device_roots.is_empty());
}

#[test]
fn apply_drive_root_event_for_foreign_owner_ignored() {
    let dir = tempdir().unwrap();
    let (mut cfg, _) = config_with_owner_account(dir.path());
    let other_owner = Keys::generate().public_key().to_hex();
    let device_key = Keys::generate();
    let root = encrypted_root(0xf0, 0, 1);
    let event = build_drive_root_event(
        &device_key,
        &other_owner,
        "main",
        &root,
        &[cfg.account.as_ref().unwrap().device_pubkey.clone()],
    )
    .unwrap();
    let outcome = apply_remote_drive_root_event(&mut cfg, &event, None).unwrap();
    assert_eq!(outcome, DriveRootApply::NotOurOwner);
}

#[test]
fn apply_drive_root_event_for_unknown_drive_ignored() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());
    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    acct.approve_device(device_b_hex.clone(), None).unwrap();
    cfg.account = Some(acct.state.clone());
    let root = encrypted_root(0x44, 0, 1);
    let event = build_drive_root_event(
        &device_b,
        &acct.state.owner_pubkey,
        "nonexistent",
        &root,
        &[acct.state.device_pubkey.clone(), device_b_hex],
    )
    .unwrap();
    let outcome = apply_remote_drive_root_event(&mut cfg, &event, None).unwrap();
    assert_eq!(outcome, DriveRootApply::UnknownDrive);
}

#[test]
fn apply_drive_root_event_stale_timestamp_ignored() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());
    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    acct.approve_device(device_b_hex.clone(), None).unwrap();
    cfg.account = Some(acct.state.clone());

    // First publish — applied.
    let root_1 = encrypted_root(0x11, 0, 1);
    let event_1 = build_drive_root_event(
        &device_b,
        &acct.state.owner_pubkey,
        "main",
        &root_1,
        &[acct.state.device_pubkey.clone(), device_b_hex.clone()],
    )
    .unwrap();
    assert_eq!(
        apply_remote_drive_root_event(&mut cfg, &event_1, Some(acct.device.keys())).unwrap(),
        DriveRootApply::Applied
    );
    let first_published_at = cfg
        .drive("main")
        .unwrap()
        .device_roots
        .get(&device_b_hex)
        .unwrap()
        .published_at;

    // Replay the same event — same created_at, should be StaleTimestamp.
    assert_eq!(
        apply_remote_drive_root_event(&mut cfg, &event_1, None).unwrap(),
        DriveRootApply::StaleTimestamp
    );
    // device_roots entry unchanged.
    assert_eq!(
        cfg.drive("main")
            .unwrap()
            .device_roots
            .get(&device_b_hex)
            .unwrap()
            .root_cid,
        root_1.root_cid
    );
    assert_eq!(
        cfg.drive("main")
            .unwrap()
            .device_roots
            .get(&device_b_hex)
            .unwrap()
            .published_at,
        first_published_at
    );
}

#[test]
fn apply_drive_root_event_ignores_same_legacy_root_with_newer_timestamp() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());
    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    acct.approve_device(device_b_hex.clone(), None).unwrap();
    cfg.account = Some(acct.state.clone());

    let mut root = encrypted_root(0x13, 100, 1);
    let event_1 = build_drive_root_event(
        &device_b,
        &acct.state.owner_pubkey,
        "main",
        &root,
        &[acct.state.device_pubkey.clone(), device_b_hex.clone()],
    )
    .unwrap();
    assert_eq!(
        apply_remote_drive_root_event(&mut cfg, &event_1, Some(acct.device.keys())).unwrap(),
        DriveRootApply::Applied
    );

    root.published_at = 200;
    let republished = build_drive_root_event(
        &device_b,
        &acct.state.owner_pubkey,
        "main",
        &root,
        &[acct.state.device_pubkey.clone(), device_b_hex.clone()],
    )
    .unwrap();

    assert_eq!(
        apply_remote_drive_root_event(&mut cfg, &republished, Some(acct.device.keys())).unwrap(),
        DriveRootApply::StaleTimestamp
    );
    let entry = cfg
        .drive("main")
        .unwrap()
        .device_roots
        .get(&device_b_hex)
        .unwrap();
    assert_eq!(entry.root_cid, root.root_cid);
    assert_eq!(entry.published_at, 100);
}

#[test]
fn apply_drive_root_event_prefers_higher_device_seq_over_newer_timestamp() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());
    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    acct.approve_device(device_b_hex.clone(), None).unwrap();
    cfg.account = Some(acct.state.clone());

    let root_1 = causal_encrypted_root(0x21, 300, 1, 1);
    let event_1 = build_drive_root_event(
        &device_b,
        &acct.state.owner_pubkey,
        "main",
        &root_1,
        &[acct.state.device_pubkey.clone(), device_b_hex.clone()],
    )
    .unwrap();
    assert_eq!(
        apply_remote_drive_root_event(&mut cfg, &event_1, Some(acct.device.keys())).unwrap(),
        DriveRootApply::Applied
    );

    let root_2 = causal_encrypted_root(0x22, 100, 1, 2);
    let event_2 = build_drive_root_event(
        &device_b,
        &acct.state.owner_pubkey,
        "main",
        &root_2,
        &[acct.state.device_pubkey.clone(), device_b_hex.clone()],
    )
    .unwrap();
    assert_eq!(
        apply_remote_drive_root_event(&mut cfg, &event_2, Some(acct.device.keys())).unwrap(),
        DriveRootApply::Applied
    );

    let entry = cfg
        .drive("main")
        .unwrap()
        .device_roots
        .get(&device_b_hex)
        .unwrap();
    assert_eq!(entry.root_cid, root_2.root_cid);
    assert_eq!(entry.device_seq, 2);
    assert_eq!(entry.published_at, 100);
}

#[test]
fn same_second_drive_root_selection_prefers_higher_device_seq() {
    let device = Keys::generate();
    let owner = Keys::generate().public_key().to_hex();
    let older = causal_encrypted_root(0x31, 1_700_000_000, 1, 1);
    let newer = causal_encrypted_root(0x32, 1_700_000_000, 1, 2);
    let authorized = vec![device.public_key().to_hex()];
    let older_event = build_drive_root_event(&device, &owner, "main", &older, &authorized).unwrap();
    let newer_event = build_drive_root_event(&device, &owner, "main", &newer, &authorized).unwrap();

    assert!(drive_root_event_is_newer(&newer_event, &older_event));
    assert!(!drive_root_event_is_newer(&older_event, &newer_event));
}

#[test]
fn apply_app_keys_event_revokes_authorized_state_when_we_get_removed() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());
    assert_eq!(
        cfg.account.as_ref().unwrap().authorization_state,
        DeviceAuthorizationState::Authorized
    );
    // Owner publishes a new snapshot removing this device.
    let other_device = Keys::generate().public_key().to_hex();
    acct.approve_device(other_device, None).unwrap();
    acct.revoke_device(&cfg.account.as_ref().unwrap().device_pubkey)
        .unwrap();
    let event = build_app_keys_event(
        acct.owner_key.as_ref().unwrap().keys(),
        acct.state.app_keys.as_ref().unwrap(),
    )
    .unwrap();
    let outcome = apply_remote_app_keys_event(&mut cfg, &event).unwrap();
    assert!(
        matches!(outcome, AppKeysApply::Applied(_)),
        "expected Applied, got {outcome:?}"
    );
    assert_eq!(
        cfg.account.as_ref().unwrap().authorization_state,
        DeviceAuthorizationState::Revoked
    );
}
