use super::*;
use crate::profile::Profile;
use nostr_sdk::Event;
use nostr_sdk::filter::MatchEventOptions;
use tempfile::tempdir;

fn profile_event(op: &crate::SignedNostrIdentityRosterOp) -> Event {
    Event::from_json(&op.event_json).unwrap()
}

fn filter_matches(filter: &Filter, event: &Event) -> bool {
    filter.match_event(event, MatchEventOptions::default())
}

#[test]
fn app_key_approval_candidates_project_rosters_that_mention_joining_key() {
    let admin_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let mut admin = Profile::create(admin_dir.path(), Some("admin".into())).unwrap();
    let linked = Profile::start_join_request(linked_dir.path(), Some("phone".into())).unwrap();
    let linked_pubkey = linked.state.app_key_pubkey.clone();
    admin
        .approve_app_key(&linked_pubkey, Some("phone".into()))
        .unwrap();
    let filters = nostr_identity_app_key_approval_candidate_filters(&linked_pubkey).unwrap();
    let add_event = profile_event(
        admin
            .state
            .profile_roster_ops
            .iter()
            .find(|op| op.content.op.target_pubkey() == Some(linked_pubkey.as_str()))
            .expect("approval roster op mentions joining AppKey"),
    );

    assert!(
        filters
            .iter()
            .any(|filter| filter_matches(filter, &add_event))
    );

    let events = admin
        .state
        .profile_roster_ops
        .iter()
        .map(profile_event)
        .collect::<Vec<_>>();
    let candidates =
        nostr_identity_app_key_approval_candidates_from_events(&linked_pubkey, &events)
            .expect("approval candidates project");

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].profile_id, admin.state.profile_id);
    assert_eq!(candidates[0].app_key_pubkey, linked_pubkey);
    assert_eq!(
        candidates[0].admin_app_key_pubkey,
        admin.state.app_key_pubkey
    );
    assert_eq!(
        candidates[0].profile_roster_ops.len(),
        admin.state.profile_roster_ops.len()
    );
    assert!(candidates[0].accepted_roster_op_count >= 2);
    assert_eq!(candidates[0].active_app_key_count, 2);
}
