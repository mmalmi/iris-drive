use super::*;
use crate::sharing::{
    create_shared_folder, invite_shared_folder_resolved_recipient,
    share_recipient_profile_evidence_for_app_key, sign_share_access_snapshot,
};
use crate::{Profile, revoke_shared_folder_member};
use nostr_sdk::JsonUtil;

struct Party {
    _dir: tempfile::TempDir,
    profile: Profile,
}

fn party(label: &str) -> Party {
    let dir = tempfile::tempdir().unwrap();
    let profile = Profile::create(dir.path(), Some(label.into())).unwrap();
    Party { _dir: dir, profile }
}

fn owner_folder(owner: &Party) -> SharedFolder {
    create_shared_folder(
        owner.profile.app_key.keys(),
        owner.profile.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Owner".into()),
        Vec::new(),
        10,
    )
    .unwrap()
}

fn evidence(guest: &Party, at: i64) -> ShareRecipientProfileEvidence {
    share_recipient_profile_evidence_for_app_key(
        guest.profile.state.profile_id,
        &guest.profile.state.profile_roster_ops,
        guest.profile.app_key.keys(),
        Some("Guest".into()),
        at,
    )
    .unwrap()
}

fn owner_pubkey(owner: &Party) -> String {
    owner.profile.app_key.keys().public_key().to_hex()
}

#[test]
fn link_roundtrips_through_web_url_and_native_scheme() {
    let owner = party("owner");
    let folder = owner_folder(&owner);
    let link = ShareJoinLink::new(&folder, &owner_pubkey(&owner), ShareRole::Editor).unwrap();
    let url = link.url();

    assert!(url.starts_with("https://drive.iris.to/#/share-join/"));
    assert!(ShareJoinLink::is_link(&url));
    assert_eq!(ShareJoinLink::parse(&url).unwrap(), link);
    let native = url.replace("https://drive.iris.to/#", "iris-drive:/");
    assert_eq!(ShareJoinLink::parse(&native).unwrap(), link);
    assert!(ShareJoinLink::new(&folder, &owner_pubkey(&owner), ShareRole::Admin).is_err());
}

#[test]
fn parses_a_link_made_by_iris_drive_web() {
    // encodeShareJoinLink() from iris-drive-web src/drive/shareJoinLink.ts.
    let web = "https://drive.iris.to/#/share-join/eyJ2IjoxLCJzaGFyZV9pZCI6IjExMTExMTExLTExMTEtNDExMS04MTExLTExMTExMTExMTExMSIsIm93bmVyX3Byb2ZpbGVfaWQiOiIyMjIyMjIyMi0yMjIyLTQyMjItODIyMi0yMjIyMjIyMjIyMjIiLCJhZG1pbiI6ImFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWFhYWEiLCJuYW1lIjoiVHJpcCBwaG90b3MiLCJyb2xlIjoicmVhZGVyIiwia2V5IjoiMDEwMTAxMDEwMTAxMDEwMTAxMDEwMTAxMDEwMTAxMDEwMTAxMDEwMTAxMDEwMTAxMDEwMTAxMDEwMTAxMDEwMSJ9";
    let link = ShareJoinLink::parse(web).unwrap();
    assert_eq!(link.name, "Trip photos");
    assert_eq!(link.role, ShareRole::Reader);
    assert_eq!(link.admin, "a".repeat(64));
    assert_eq!(
        link.share_id.to_string(),
        "11111111-1111-4111-8111-111111111111"
    );
}

#[test]
fn rejects_malformed_links() {
    assert!(ShareJoinLink::parse("https://drive.iris.to/#/share-join/notjson").is_err());
    let owner = party("owner");
    let folder = owner_folder(&owner);
    let mut link = ShareJoinLink::new(&folder, &owner_pubkey(&owner), ShareRole::Reader).unwrap();
    link.doc = Some("a/b".into());
    assert!(ShareJoinLink::parse(&link.url()).is_err());
}

#[test]
fn guest_request_is_approved_and_guest_opens_the_folder() {
    let owner = party("owner");
    let guest = party("guest");
    let mut folder = owner_folder(&owner);
    let link = ShareJoinLink::new(&folder, &owner_pubkey(&owner), ShareRole::Reader).unwrap();
    let guest_pubkey = guest.profile.app_key.keys().public_key().to_hex();

    let request = build_share_join_request(&link, &evidence(&guest, 20), 20).unwrap();
    assert!(is_share_join_request_event(&request));
    let filter = share_join_request_filter(&[link.pubkey().unwrap()]).unwrap();
    assert!(filter.match_event(&request, nostr_sdk::filter::MatchEventOptions::default()));

    let (resolved, requester) = share_recipient_from_join_request(&link, &request).unwrap();
    assert_eq!(requester, guest_pubkey);
    assert_eq!(resolved.profile_id, guest.profile.state.profile_id);
    assert!(join_request_needs_approval(
        &folder,
        resolved.profile_id,
        &requester
    ));

    // Before approval the guest sees nothing it can open.
    let before = sign_share_access_snapshot(owner.profile.app_key.keys(), &folder, 21).unwrap();
    let before = Event::from_json(&before.event_json).unwrap();
    assert!(shared_folder_from_access_event_for_join(&link, &before, &guest_pubkey).is_none());

    invite_shared_folder_resolved_recipient(
        &mut folder,
        owner.profile.app_key.keys(),
        &resolved,
        link.role,
        30,
    )
    .unwrap();
    assert!(!join_request_needs_approval(
        &folder,
        resolved.profile_id,
        &requester
    ));

    let snapshot = sign_share_access_snapshot(
        owner.profile.app_key.keys(),
        &folder,
        folder.access.updated_at,
    )
    .unwrap();
    let event = Event::from_json(&snapshot.event_json).unwrap();
    let joined = shared_folder_from_access_event_for_join(&link, &event, &guest_pubkey).unwrap();
    assert_eq!(joined.share_id, folder.share_id);
    assert_eq!(joined.local_role, ShareRole::Reader);
    assert_eq!(joined.display_name, "Alpha");
    assert!(crate::current_shared_folder_key(&joined, guest.profile.app_key.keys()).is_ok());
}

#[test]
fn requests_signed_without_the_link_or_for_another_folder_are_refused() {
    let owner = party("owner");
    let guest = party("guest");
    let folder = owner_folder(&owner);
    let link = ShareJoinLink::new(&folder, &owner_pubkey(&owner), ShareRole::Editor).unwrap();
    let other = ShareJoinLink::new(&folder, &owner_pubkey(&owner), ShareRole::Editor).unwrap();

    let forged = build_share_join_request(&other, &evidence(&guest, 20), 20).unwrap();
    assert!(share_recipient_from_join_request(&link, &forged).is_err());

    let mut elsewhere = link.clone();
    elsewhere.share_id = NostrIdentityId::new_v4();
    let misdirected = build_share_join_request(&elsewhere, &evidence(&guest, 20), 20).unwrap();
    assert!(share_recipient_from_join_request(&link, &misdirected).is_err());
}

#[test]
fn a_removed_person_is_not_let_back_in_by_the_link() {
    let owner = party("owner");
    let guest = party("guest");
    let mut folder = owner_folder(&owner);
    let link = ShareJoinLink::new(&folder, &owner_pubkey(&owner), ShareRole::Editor).unwrap();
    let request = build_share_join_request(&link, &evidence(&guest, 20), 20).unwrap();
    let (resolved, requester) = share_recipient_from_join_request(&link, &request).unwrap();
    invite_shared_folder_resolved_recipient(
        &mut folder,
        owner.profile.app_key.keys(),
        &resolved,
        link.role,
        30,
    )
    .unwrap();
    revoke_shared_folder_member(
        &mut folder,
        owner.profile.app_key.keys(),
        resolved.profile_id,
        None,
        40,
    )
    .unwrap();

    assert!(!join_request_needs_approval(
        &folder,
        resolved.profile_id,
        &requester
    ));
}

#[test]
fn access_lists_signed_by_anyone_but_the_links_admin_are_ignored() {
    let owner = party("owner");
    let guest = party("guest");
    let stranger = party("stranger");
    let folder = owner_folder(&owner);
    let link = ShareJoinLink::new(&folder, &owner_pubkey(&owner), ShareRole::Reader).unwrap();
    let mut forged = owner_folder(&stranger);
    forged.share_id = folder.share_id;
    forged.access.resource_id = folder.share_id;
    let guest_pubkey = guest.profile.app_key.keys().public_key().to_hex();
    let snapshot =
        sign_share_access_snapshot(stranger.profile.app_key.keys(), &forged, 50).unwrap();
    let event = Event::from_json(&snapshot.event_json).unwrap();

    assert!(shared_folder_from_access_event_for_join(&link, &event, &guest_pubkey).is_none());
}
