use super::*;
use crate::{
    NostrIdentityCapabilities, NostrIdentityFacet, NostrIdentityRosterOp,
    build_nostr_identity_roster_op_event,
};
use tempfile::tempdir;

fn attacker_bootstrap(profile_id: crate::NostrIdentityId, attacker: &Keys) -> Event {
    build_nostr_identity_roster_op_event(
        attacker,
        profile_id,
        Vec::new(),
        None,
        NostrIdentityRosterOp::AddFacet {
            facet: NostrIdentityFacet::app_key(
                attacker.public_key().to_hex(),
                1,
                None,
                NostrIdentityCapabilities::app_admin(),
            ),
        },
        1,
    )
    .unwrap()
}

#[test]
fn remote_roster_cannot_replace_established_bootstrap_with_backdated_attacker() {
    let dir = tempdir().unwrap();
    let (mut config, profile) = tests::config_with_owner_account(dir.path());
    let attacker = Keys::generate();
    let event = attacker_bootstrap(profile.state.profile_id, &attacker);
    let before = config.profile.clone();

    assert!(apply_remote_nostr_identity_roster_op_event(&mut config, &event).is_err());
    assert_eq!(config.profile, before);
    let projection = config.profile.as_ref().unwrap().profile_projection();
    assert!(projection.can_admin_profile(&profile.state.app_key_pubkey));
    assert!(!projection.can_admin_profile(&attacker.public_key().to_hex()));
}

#[test]
fn roster_frame_cannot_replace_established_bootstrap_with_backdated_attacker() {
    let dir = tempdir().unwrap();
    let (mut config, profile) = tests::config_with_owner_account(dir.path());
    let attacker = Keys::generate();
    let bootstrap = attacker_bootstrap(profile.state.profile_id, &attacker);
    let epoch_at = profile.state.app_keys.as_ref().unwrap().created_at + 10;
    let epoch = build_nostr_identity_roster_op_event(
        &attacker,
        profile.state.profile_id,
        vec![bootstrap.id.to_hex()],
        None,
        NostrIdentityRosterOp::RotateSecretEpoch {
            epoch: 2,
            wrapped_secrets: BTreeMap::new(),
        },
        epoch_at,
    )
    .unwrap();
    let frame = AppKeyLinkRosterFrame {
        schema: 1,
        profile_id: profile.state.profile_id,
        admin_app_key_pubkey: attacker.public_key().to_hex(),
        profile_roster_ops: vec![
            parse_nostr_identity_roster_op_event(&bootstrap).unwrap(),
            parse_nostr_identity_roster_op_event(&epoch).unwrap(),
        ],
        sent_at: u64::try_from(epoch_at).unwrap(),
    };
    let before = config.profile.clone();

    assert!(
        apply_app_key_link_roster_frame(&mut config, &frame, &attacker.public_key().to_hex())
            .is_err()
    );
    assert_eq!(config.profile, before);
}
