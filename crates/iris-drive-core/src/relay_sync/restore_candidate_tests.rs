use super::*;
use crate::config::Drive;
use crate::nostr_identity::{
    NostrIdentityRosterOp, build_nostr_identity_facet_acceptance_event,
    build_nostr_identity_roster_op_event,
};
use crate::profile::Profile;
use nostr_sdk::Event;
use nostr_sdk::filter::MatchEventOptions;
use tempfile::tempdir;

fn config_with_recovery_owner_account(dir: &std::path::Path) -> (AppConfig, Profile, String) {
    let phrase = crate::recovery_phrase::generate_recovery_phrase().unwrap();
    let acct = Profile::restore(dir, &phrase, None).unwrap();
    let mut cfg = AppConfig {
        profile: Some(acct.state.clone()),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(acct.state.root_scope_id()));
    (cfg, acct, phrase)
}

fn profile_event(op: &crate::SignedNostrIdentityRosterOp) -> Event {
    Event::from_json(&op.event_json).unwrap()
}

fn filter_matches(filter: &Filter, event: &Event) -> bool {
    filter.match_event(event, MatchEventOptions::default())
}

#[test]
fn restore_candidate_filters_match_roster_mentions_and_acceptance_events() {
    let dir = tempdir().unwrap();
    let (_, acct, phrase) = config_with_recovery_owner_account(dir.path());
    let recovery_key =
        crate::identity::RecoveryKey::from_recovery_phrase(&phrase, dir.path().join("recovery"))
            .unwrap();
    let recovery_pubkey = recovery_key.pubkey_hex();
    let filters = nostr_identity_restore_candidate_filters(&recovery_pubkey).unwrap();
    let recovery_add_event = profile_event(
        acct.state
            .profile_roster_ops
            .iter()
            .find(|op| op.content.op.target_pubkey() == Some(recovery_pubkey.as_str()))
            .unwrap(),
    );
    let acceptance = build_nostr_identity_facet_acceptance_event(
        recovery_key.keys(),
        acct.state.profile_id,
        [crate::NostrIdentityKeyPurpose::RecoveryPhrase],
        None,
        20,
    )
    .unwrap();

    assert!(
        filters
            .iter()
            .any(|filter| filter_matches(filter, &recovery_add_event))
    );
    assert!(
        filters
            .iter()
            .any(|filter| filter_matches(filter, &acceptance))
    );
}

#[test]
fn restore_candidates_require_active_recovery_facet_projection() {
    let dir = tempdir().unwrap();
    let (_, mut acct, phrase) = config_with_recovery_owner_account(dir.path());
    let recovery_key =
        crate::identity::RecoveryKey::from_recovery_phrase(&phrase, dir.path().join("recovery"))
            .unwrap();
    let recovery_pubkey = recovery_key.pubkey_hex();
    let events = acct
        .state
        .profile_roster_ops
        .iter()
        .map(profile_event)
        .collect::<Vec<_>>();

    let candidates = nostr_identity_restore_candidates_from_events(&recovery_pubkey, &events)
        .expect("candidates project");

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].profile_id, acct.state.profile_id);
    assert_eq!(candidates[0].recovery_pubkey, recovery_pubkey);
    assert!(candidates[0].can_decrypt_secret_epochs);
    assert_eq!(candidates[0].accepted_roster_op_count, 3);
    assert_eq!(candidates[0].active_app_key_count, 1);
    assert!(candidates[0].latest_roster_op_created_at.is_some());
    assert_eq!(
        candidates[0].profile_roster_ops.len(),
        acct.state.profile_roster_ops.len()
    );

    let restored_dir = tempdir().unwrap();
    let restored = Profile::restore_with_profile_roster_ops(
        restored_dir.path(),
        &phrase,
        candidates[0].profile_id,
        candidates[0].profile_roster_ops.clone(),
        Some("restored".into()),
    )
    .unwrap();
    assert_eq!(restored.state.profile_id, acct.state.profile_id);

    let remove_recovery = build_nostr_identity_roster_op_event(
        acct.app_key.keys(),
        acct.state.profile_id,
        crate::nostr_identity_roster_parent_ids(&acct.state.profile_roster_ops),
        None,
        NostrIdentityRosterOp::TombstoneFacet {
            pubkey: recovery_pubkey.clone(),
            reason: Some("lost phrase".into()),
        },
        acct.state
            .profile_roster_ops
            .iter()
            .map(|op| op.content.created_at)
            .max()
            .unwrap()
            + 1,
    )
    .unwrap();
    acct.state
        .profile_roster_ops
        .push(crate::parse_nostr_identity_roster_op_event(&remove_recovery).unwrap());
    let events = acct
        .state
        .profile_roster_ops
        .iter()
        .map(profile_event)
        .collect::<Vec<_>>();

    assert!(
        nostr_identity_restore_candidates_from_events(&recovery_pubkey, &events)
            .unwrap()
            .is_empty()
    );
}
