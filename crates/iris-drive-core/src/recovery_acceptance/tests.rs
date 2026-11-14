use super::*;
use crate::recovery_phrase::{generate_recovery_phrase, recovery_phrase_to_keys};
use crate::{Profile, nostr_identity_candidate_ids_for_pubkey_from_events};
use std::str::FromStr;
use tempfile::tempdir;

/// The web's `buildNostrIdentityFacetAcceptanceEventDraft` (nostr-social-graph
/// 4.1.0) for these inputs hashes to this id, so the tags and content match.
#[test]
fn signs_the_same_event_as_the_web() {
    let keys =
        Keys::parse("7f7ff03d123792d6ac594bfa67bf6d0c0ab55b6b1fdb6249303fe861f1ccba9a").unwrap();
    let event = sign_acceptance_with_nonce(
        &keys,
        NostrIdentityId::from_str("4ec8a9da-89cd-452f-9b76-74170a5e6caf").unwrap(),
        "2268b20912aeb07fd4055b9a4ea43d5d17dde6a1fe67e671ba1332b7716c5dfb",
        1_794_682_779,
        "fixture-nonce:accept",
    )
    .unwrap();
    assert_eq!(
        event.id.to_hex(),
        "9df75f864bd7f31ca9c2257ba7026abf485a1db5de31792599b1effc0eb2cf81"
    );
}

#[test]
fn a_native_phrase_signs_an_acceptance_that_finds_its_account() {
    let dir = tempdir().unwrap();
    let mut profile = Profile::create(dir.path(), Some("Laptop".into())).unwrap();
    let phrase = generate_recovery_phrase().unwrap();
    let recovery_keys = recovery_phrase_to_keys(&phrase).unwrap();
    let recovery_pubkey = recovery_keys.public_key().to_hex();
    profile.add_recovery_pubkey(&recovery_pubkey).unwrap();
    let add_op = recovery_phrase_add_op(&profile.state, &recovery_pubkey).unwrap();

    let event =
        sign_recovery_phrase_acceptance(&recovery_keys, profile.state.profile_id, add_op).unwrap();
    assert_eq!(event.pubkey, recovery_keys.public_key());
    let acceptance = parse_nostr_identity_facet_acceptance_event(&event).unwrap();
    assert_eq!(
        acceptance.content.roster_op_id.as_deref(),
        Some(add_op.op_id.as_str())
    );
    assert_eq!(
        acceptance.content.accepted_at,
        add_op.content.created_at + 1
    );
    assert!(acceptance.content.client_nonce.ends_with(":accept"));
    assert_eq!(
        acceptance
            .content
            .purposes
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        vec![NostrIdentityKeyPurpose::RecoveryPhrase]
    );
    assert!(acceptance.is_active_in_roster(&profile.state.profile_projection()));
    // The web looks for events the phrase key signed that tag that key.
    assert!(
        event
            .tags
            .iter()
            .any(|tag| { tag.as_slice() == ["p".to_string(), recovery_pubkey.clone()].as_slice() })
    );
    assert_eq!(
        nostr_identity_candidate_ids_for_pubkey_from_events(&recovery_pubkey, &[event]).unwrap(),
        vec![profile.state.profile_id]
    );
}

#[test]
fn queued_acceptances_are_kept_once() {
    let dir = tempdir().unwrap();
    let mut profile = Profile::create(dir.path(), None).unwrap();
    let recovery_keys = Keys::generate();
    let recovery_pubkey = recovery_keys.public_key().to_hex();
    profile.add_recovery_pubkey(&recovery_pubkey).unwrap();
    let add_op = recovery_phrase_add_op(&profile.state, &recovery_pubkey).unwrap();
    let event =
        sign_recovery_phrase_acceptance(&recovery_keys, profile.state.profile_id, add_op).unwrap();

    assert_eq!(queued_facet_acceptances(dir.path()), Vec::<Event>::new());
    queue_facet_acceptance(dir.path(), &event).unwrap();
    queue_facet_acceptance(dir.path(), &event).unwrap();
    let queued = queued_facet_acceptances(dir.path());
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].as_json(), event.as_json());
    assert!(
        recovery_phrase_add_op(&profile.state, &Keys::generate().public_key().to_hex()).is_none()
    );
}
