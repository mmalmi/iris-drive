use super::*;
use crate::nostr_events::{build_drive_root_event, parse_drive_root_event_for_device};
use crate::recovery_phrase::{generate_recovery_phrase, recovery_phrase_to_keys};
use crate::{AppKeyRootRef, Profile, network_sync::drive_root_recipient_app_key_pubkeys};
use hashtree_core::Cid;
use tempfile::tempdir;

struct Owner {
    _dir: tempfile::TempDir,
    profile: Profile,
    recovery_keys: Keys,
    drive: Drive,
}

fn owner_with_recovery_phrase() -> Owner {
    let dir = tempdir().unwrap();
    let mut profile = Profile::create(dir.path(), Some("Laptop".into())).unwrap();
    let phrase = generate_recovery_phrase().unwrap();
    let recovery_keys = recovery_phrase_to_keys(&phrase).unwrap();
    profile.add_recovery_phrase(&phrase).unwrap();
    let drive = Drive::primary(profile.state.root_scope_id());
    Owner {
        _dir: dir,
        profile,
        recovery_keys,
        drive,
    }
}

fn root(byte: u8) -> AppKeyRootRef {
    AppKeyRootRef::legacy(
        Cid::encrypted([byte; 32], [byte ^ 0x5a; 32]).to_string(),
        1_700_000_000,
        1,
    )
}

#[test]
fn my_drive_root_is_wrapped_for_the_recovery_phrase() {
    let owner = owner_with_recovery_phrase();
    let state = &owner.profile.state;
    let recovery_pubkey = owner.recovery_keys.public_key().to_hex();
    let recipients = drive_root_key_recipient_pubkeys(state, &owner.drive);
    assert!(recipients.contains(&recovery_pubkey));
    assert!(recipients.contains(&state.app_key_pubkey));
    // Checks that count devices (missing roots, writers) still see devices only.
    assert!(!drive_root_recipient_app_key_pubkeys(state, &owner.drive).contains(&recovery_pubkey));

    let root = root(0x11);
    let event = build_drive_root_event(
        owner.profile.app_key.keys(),
        &state.root_scope_id(),
        PRIMARY,
        &root,
        &recipients,
    )
    .unwrap();
    let (_, _, _, opened) =
        parse_drive_root_event_for_device(&event, &owner.recovery_keys).unwrap();
    assert_eq!(opened.root_cid, root.root_cid);
    assert!(parse_drive_root_event_for_device(&event, &Keys::generate()).is_err());
}

#[test]
fn adding_a_recovery_phrase_is_seen_in_the_ops_to_publish() {
    let owner = owner_with_recovery_phrase();
    let state = &owner.profile.state;
    assert!(ops_add_active_recovery_phrase(
        state,
        &state.profile_roster_ops
    ));
    let without_phrase = state
        .profile_roster_ops
        .iter()
        .filter(|op| {
            !matches!(&op.content.op, NostrIdentityRosterOp::AddFacet { facet }
                if facet.pubkey == owner.recovery_keys.public_key().to_hex())
        })
        .cloned()
        .collect::<Vec<_>>();
    assert!(!ops_add_active_recovery_phrase(state, &without_phrase));
}

#[test]
fn the_recovery_phrase_alone_opens_every_device_root() {
    let owner = owner_with_recovery_phrase();
    let state = &owner.profile.state;
    let wrapped = root(0x21);
    let wrapped_event = build_drive_root_event(
        owner.profile.app_key.keys(),
        &state.root_scope_id(),
        PRIMARY,
        &wrapped,
        &drive_root_key_recipient_pubkeys(state, &owner.drive),
    )
    .unwrap();

    let fresh = tempdir().unwrap();
    let restored = Profile::restore_with_recovery_phrase_keys(
        fresh.path(),
        &owner.recovery_keys,
        state.profile_id,
        state.profile_roster_ops.clone(),
        Some("Phone".into()),
    )
    .unwrap();
    // Nothing is written until the caller saves the whole restore.
    assert_eq!(std::fs::read_dir(fresh.path()).unwrap().count(), 0);
    assert!(restored.state.can_write_roots());
    assert_eq!(
        restored.current_dck().unwrap(),
        owner.profile.current_dck().unwrap()
    );

    let mut config = AppConfig {
        profile: Some(restored.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(restored.state.root_scope_id()));
    let report = apply_drive_root_events_opening_with_recovery_key(
        &mut config,
        &[wrapped_event],
        restored.app_key.keys(),
        &owner.recovery_keys,
    )
    .unwrap();
    assert_eq!(report.applied, 1);
    assert_eq!(report.root_cids_to_download, vec![wrapped.root_cid.clone()]);
    let drive = config.drive(PRIMARY).unwrap();
    assert_eq!(
        drive.app_key_roots[&state.app_key_pubkey].root_cid,
        wrapped.root_cid
    );
}

#[test]
fn a_root_published_before_the_phrase_stays_closed_to_it() {
    let owner = owner_with_recovery_phrase();
    let state = &owner.profile.state;
    let old_event = build_drive_root_event(
        owner.profile.app_key.keys(),
        &state.root_scope_id(),
        PRIMARY,
        &root(0x31),
        &drive_root_recipient_app_key_pubkeys(state, &owner.drive),
    )
    .unwrap();
    let fresh = tempdir().unwrap();
    let restored = Profile::restore_with_recovery_phrase_keys(
        fresh.path(),
        &owner.recovery_keys,
        state.profile_id,
        state.profile_roster_ops.clone(),
        None,
    )
    .unwrap();
    let mut config = AppConfig {
        profile: Some(restored.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(restored.state.root_scope_id()));
    let report = apply_drive_root_events_opening_with_recovery_key(
        &mut config,
        &[old_event],
        restored.app_key.keys(),
        &owner.recovery_keys,
    )
    .unwrap();
    assert_eq!((report.applied, report.skipped), (0, 1));
    assert_eq!(report.root_cids_to_download, Vec::<String>::new());
}

const PRIMARY: &str = crate::PRIMARY_DRIVE_ID;
