use super::*;
use crate::app_key_link_transport::AppKeyLinkRosterFrame;
use crate::config::Drive;
use crate::iris_profile::{
    IrisProfileCapabilities, IrisProfileFacet, IrisProfileId, IrisProfileRosterOp,
    build_iris_profile_facet_acceptance_event, build_iris_profile_roster_op_event,
};
use crate::nostr_events::{
    build_app_key_link_request_event, build_drive_root_event, build_private_hashtree_root_event,
};
use crate::profile::{AppKeyAuthorizationState, Profile};
use crate::sharing::{
    ShareAccessDevice, ShareAccessGrant, ShareAccessTarget, ShareMemberStatus, ShareRecipient,
    ShareRole,
};
use hashtree_core::Cid;
use nostr_sdk::filter::MatchEventOptions;
use tempfile::tempdir;

fn config_with_owner_account(dir: &std::path::Path) -> (AppConfig, Profile) {
    let acct = Profile::create(dir, None).unwrap();
    let mut cfg = AppConfig {
        profile: Some(acct.state.clone()),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(acct.state.root_scope_id()));
    (cfg, acct)
}

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

fn profile_event(op: &crate::SignedIrisProfileRosterOp) -> Event {
    Event::from_json(&op.event_json).unwrap()
}

fn filter_matches(filter: &Filter, event: &Event) -> bool {
    filter.match_event(event, MatchEventOptions::default())
}

fn encrypted_root(seed: u8, published_at: i64, dck_generation: u64) -> AppKeyRootRef {
    AppKeyRootRef::legacy(
        Cid::encrypted([seed; 32], [seed.wrapping_add(1); 32]).to_string(),
        published_at,
        dck_generation,
    )
}

fn causal_encrypted_root(
    seed: u8,
    published_at: i64,
    dck_generation: u64,
    app_key_seq: u64,
) -> AppKeyRootRef {
    AppKeyRootRef {
        root_cid: Cid::encrypted([seed; 32], [seed.wrapping_add(1); 32]).to_string(),
        published_at,
        dck_generation,
        app_key_seq,
        parents: Vec::new(),
        observed: std::collections::BTreeMap::new(),
        local_only: false,
    }
}

fn roster_frame(
    admin: &Profile,
    profile_roster_ops: Vec<crate::SignedIrisProfileRosterOp>,
    sent_at: u64,
) -> AppKeyLinkRosterFrame {
    AppKeyLinkRosterFrame {
        schema: 1,
        profile_id: admin.state.profile_id,
        admin_app_key_pubkey: admin.state.app_key_pubkey.clone(),
        profile_roster_ops,
        sent_at,
    }
}

fn linked_config_after_initial_roster() -> (Profile, AppConfig) {
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
    let linked_pubkey = linked.state.app_key_pubkey.clone();
    linked
        .state
        .queue_outbound_app_key_link_request(
            admin.state.app_key_pubkey.clone(),
            &admin.state.app_key_link_secret,
            123,
        )
        .unwrap();

    admin
        .approve_app_key(&linked_pubkey, Some("phone".into()))
        .unwrap();
    let mut cfg = AppConfig {
        profile: Some(linked.state.clone()),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(admin.state.root_scope_id()));

    let first_frame = roster_frame(&admin, admin.state.profile_roster_ops.clone(), 456);
    let initial =
        apply_app_key_link_roster_frame(&mut cfg, &first_frame, &admin.state.app_key_pubkey)
            .unwrap();
    assert!(matches!(
        initial,
        AppKeyLinkRosterApply::Applied(ApplyDecision::Adopted)
    ));
    assert_eq!(
        cfg.profile.as_ref().unwrap().authorization_state,
        AppKeyAuthorizationState::Authorized
    );
    assert_eq!(
        cfg.profile.as_ref().unwrap().profile_id,
        admin.state.profile_id
    );
    assert_eq!(
        cfg.drive(crate::PRIMARY_DRIVE_ID).unwrap().root_scope_id,
        admin.state.profile_id.to_string()
    );
    (admin, cfg)
}

#[test]
fn apply_app_key_link_roster_accepts_newer_admin_roster_after_initial_approval() {
    let (mut admin, mut cfg) = linked_config_after_initial_roster();

    let third_device = Keys::generate().public_key().to_hex();
    admin
        .approve_app_key(&third_device, Some("tablet".into()))
        .unwrap();

    let newer_frame = roster_frame(&admin, admin.state.profile_roster_ops.clone(), 789);
    let update =
        apply_app_key_link_roster_frame(&mut cfg, &newer_frame, &admin.state.app_key_pubkey)
            .unwrap();
    assert!(matches!(
        update,
        AppKeyLinkRosterApply::Applied(ApplyDecision::Replaced)
    ));
    let linked_state = cfg.profile.as_ref().unwrap();
    let linked_roster = linked_state.app_keys.as_ref().unwrap();
    assert!(linked_roster.contains(&linked_state.app_key_pubkey));
    assert!(linked_roster.contains(&third_device));
    assert!(linked_state.outbound_app_key_link_request.is_none());
}

#[test]
fn apply_app_key_link_roster_is_profile_scoped_and_ownerless() {
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
    let linked_pubkey = linked.state.app_key_pubkey.clone();
    linked
        .state
        .queue_outbound_app_key_link_request(
            admin.state.app_key_pubkey.clone(),
            &admin.state.app_key_link_secret,
            123,
        )
        .unwrap();
    admin
        .approve_app_key(&linked_pubkey, Some("phone".into()))
        .unwrap();
    let mut cfg = AppConfig {
        profile: Some(linked.state.clone()),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(admin.state.root_scope_id()));
    let frame = roster_frame(&admin, admin.state.profile_roster_ops.clone(), 456);

    let outcome =
        apply_app_key_link_roster_frame(&mut cfg, &frame, &admin.state.app_key_pubkey).unwrap();

    assert!(matches!(
        outcome,
        AppKeyLinkRosterApply::Applied(ApplyDecision::Adopted)
    ));
    let linked_state = cfg.profile.as_ref().unwrap();
    assert_eq!(linked_state.profile_id, admin.state.profile_id);
    assert_eq!(
        linked_state.app_keys.as_ref().unwrap().profile_id,
        admin.state.profile_id.to_string()
    );
}

#[test]
fn apply_app_key_link_roster_merges_older_branch_without_downgrading_epoch() {
    let (mut admin, mut cfg) = linked_config_after_initial_roster();
    let branch_base_ops = admin.state.profile_roster_ops.clone();
    let branch_at = branch_base_ops
        .iter()
        .map(|op| op.content.created_at)
        .max()
        .unwrap()
        + 1;
    let branch_app = Keys::generate().public_key().to_hex();
    let branch_op_event = build_iris_profile_roster_op_event(
        admin.app_key.keys(),
        admin.state.profile_id,
        branch_base_ops.iter().map(|op| op.op_id.clone()).collect(),
        None,
        IrisProfileRosterOp::AddFacet {
            facet: IrisProfileFacet::app_key(
                branch_app.clone(),
                branch_at,
                Some("branch app".into()),
                IrisProfileCapabilities::app_writer(),
            ),
        },
        branch_at,
    )
    .unwrap();
    let branch_op = parse_iris_profile_roster_op_event(&branch_op_event).unwrap();
    let mut branch_ops = branch_base_ops;
    branch_ops.push(branch_op.clone());

    admin.rotate_dck().unwrap();
    admin.rotate_dck().unwrap();
    let current_epoch = admin.state.app_keys.as_ref().unwrap().dck_generation;
    let current_frame = roster_frame(&admin, admin.state.profile_roster_ops.clone(), 789);
    assert!(matches!(
        apply_app_key_link_roster_frame(&mut cfg, &current_frame, &admin.state.app_key_pubkey)
            .unwrap(),
        AppKeyLinkRosterApply::Applied(ApplyDecision::Replaced)
    ));
    assert!(
        !cfg.profile
            .as_ref()
            .unwrap()
            .profile_roster_ops
            .iter()
            .any(|op| op.op_id == branch_op.op_id)
    );

    let branch_frame = roster_frame(&admin, branch_ops, 999);
    assert!(matches!(
        apply_app_key_link_roster_frame(&mut cfg, &branch_frame, &admin.state.app_key_pubkey)
            .unwrap(),
        AppKeyLinkRosterApply::Applied(ApplyDecision::Merged)
    ));

    let linked_state = cfg.profile.as_ref().unwrap();
    assert!(
        linked_state
            .profile_roster_ops
            .iter()
            .any(|op| op.op_id == branch_op.op_id)
    );
    let linked_roster = linked_state.app_keys.as_ref().unwrap();
    assert_eq!(linked_roster.dck_generation, current_epoch);
    assert!(linked_roster.contains(&branch_app));
    assert!(!linked_roster.wrapped_dck.contains_key(&branch_app));
}

#[test]
fn subscription_filters_match_app_key_link_requests_for_profile() {
    let admin = Keys::generate();
    let device = Keys::generate();
    let profile_id = IrisProfileId::new_v4();
    let frame = crate::app_key_link_transport::AppKeyLinkRequestFrame {
        schema: 1,
        profile_id,
        admin_app_key_pubkey: admin.public_key().to_hex(),
        app_key_pubkey: device.public_key().to_hex(),
        link_secret: "join-secret".to_string(),
        link_secret_hash: String::new(),
        label: Some("phone".to_string()),
        requested_at: 123,
        url: "iris-drive://app-key-link?app_key=example".to_string(),
    };
    let event = build_app_key_link_request_event(&device, &frame).unwrap();

    assert_eq!(event.kind.as_u16(), nostr_identity::FACT_OP_KIND);
    assert!(
        subscription_filters(
            &admin.public_key().to_hex(),
            &profile_id.to_string(),
            "main"
        )
        .iter()
        .any(|filter| filter_matches(filter, &event))
    );
}

#[test]
fn subscription_filters_match_iris_profile_roster_ops_for_profile() {
    let dir = tempdir().unwrap();
    let (cfg, acct) = config_with_owner_account(dir.path());
    let profile_op = profile_event(&acct.state.profile_roster_ops[0]);

    assert!(
        subscription_filters(
            &acct.state.app_key_pubkey,
            &acct.state.root_scope_id(),
            crate::PRIMARY_DRIVE_ID,
        )
        .iter()
        .any(|filter| filter_matches(filter, &profile_op))
    );
    let profile_id = cfg.profile.as_ref().unwrap().profile_id.to_string();
    assert_eq!(
        profile_op
            .tags
            .find(nostr_sdk::TagKind::from("i"))
            .and_then(|tag| tag.content()),
        Some(profile_id.as_str())
    );
}

#[test]
fn restore_candidate_filters_match_roster_mentions_and_acceptance_events() {
    let dir = tempdir().unwrap();
    let (_, acct, phrase) = config_with_recovery_owner_account(dir.path());
    let recovery_key =
        crate::identity::RecoveryKey::from_recovery_phrase(&phrase, dir.path().join("recovery"))
            .unwrap();
    let recovery_pubkey = recovery_key.pubkey_hex();
    let filters = iris_profile_restore_candidate_filters(&recovery_pubkey).unwrap();
    let recovery_add_event = profile_event(
        acct.state
            .profile_roster_ops
            .iter()
            .find(|op| op.content.op.target_pubkey() == Some(recovery_pubkey.as_str()))
            .unwrap(),
    );
    let acceptance = build_iris_profile_facet_acceptance_event(
        recovery_key.keys(),
        acct.state.profile_id,
        [crate::IrisProfileKeyPurpose::RecoveryPhrase],
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

    let candidates = iris_profile_restore_candidates_from_events(&recovery_pubkey, &events)
        .expect("candidates project");

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].profile_id, acct.state.profile_id);
    assert_eq!(candidates[0].recovery_pubkey, recovery_pubkey);
    assert!(candidates[0].can_decrypt_key_epochs);
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

    let remove_recovery = build_iris_profile_roster_op_event(
        acct.app_key.keys(),
        acct.state.profile_id,
        crate::iris_profile_roster_parent_ids(&acct.state.profile_roster_ops),
        None,
        IrisProfileRosterOp::TombstoneFacet {
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
        .push(crate::parse_iris_profile_roster_op_event(&remove_recovery).unwrap());
    let events = acct
        .state
        .profile_roster_ops
        .iter()
        .map(profile_event)
        .collect::<Vec<_>>();

    assert!(
        iris_profile_restore_candidates_from_events(&recovery_pubkey, &events)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn subscription_filters_match_share_access_snapshots_and_roots() {
    let dir = tempdir().unwrap();
    let (_, owner) = config_with_owner_account(dir.path());
    let reader = Keys::generate();
    let folder = crate::create_shared_folder(
        owner.app_key.keys(),
        owner.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Owner".into()),
        vec![ShareRecipient {
            profile_id: IrisProfileId::new_v4(),
            app_pubkey: reader.public_key().to_hex(),
            role: ShareRole::Reader,
            label: Some("Reader".into()),
            representative_npub_hint: None,
            display_name: Some("Reader".into()),
        }],
        10,
    )
    .unwrap();
    let share_snapshot =
        crate::sign_share_access_snapshot(owner.app_key.keys(), &folder, folder.access.updated_at)
            .unwrap();
    let share_event = Event::from_json(&share_snapshot.event_json).unwrap();
    let root = encrypted_root(0x55, 20, 1);
    let root_event = build_drive_root_event(
        owner.app_key.keys(),
        &folder.share_id.to_string(),
        crate::PRIMARY_DRIVE_ID,
        &root,
        &[
            owner.state.app_key_pubkey.clone(),
            reader.public_key().to_hex(),
        ],
    )
    .unwrap();

    let filters = subscription_filters_for_shared_roots(
        &owner.state.app_key_pubkey,
        &owner.state.root_scope_id(),
        crate::PRIMARY_DRIVE_ID,
        &[folder.share_id],
    );

    assert!(
        filters
            .iter()
            .any(|filter| filter_matches(filter, &share_event))
    );
    assert!(
        filters
            .iter()
            .any(|filter| filter_matches(filter, &root_event))
    );
}

#[test]
fn apply_share_access_snapshot_event_replaces_known_shared_folder() {
    let dir = tempdir().unwrap();
    let (_, owner) = config_with_owner_account(dir.path());
    let editor = Keys::generate();
    let folder = crate::create_shared_folder(
        owner.app_key.keys(),
        owner.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Owner".into()),
        Vec::new(),
        10,
    )
    .unwrap();
    let editor_id = IrisProfileId::new_v4();
    let mut remote_folder = folder.clone();
    remote_folder.access.grants.push(ShareAccessGrant {
        target: ShareAccessTarget::id(editor_id),
        role: ShareRole::Editor,
        status: ShareMemberStatus::Active,
        representative_npub_hint: None,
        display_name: Some("Editor".into()),
    });
    remote_folder.access.devices.insert(
        editor.public_key().to_hex(),
        ShareAccessDevice {
            pubkey: editor.public_key().to_hex(),
            profile_id: Some(editor_id),
            added_at: 20,
            label: Some("Editor".into()),
        },
    );
    remote_folder.access.updated_at = 20;
    let snapshot =
        crate::sign_share_access_snapshot(owner.app_key.keys(), &remote_folder, 20).unwrap();
    let snapshot_event = Event::from_json(&snapshot.event_json).unwrap();
    let mut cfg = AppConfig {
        profile: Some(owner.state.clone()),
        shared_folders: vec![folder.clone()],
        ..AppConfig::default()
    };

    let outcome = apply_remote_share_access_snapshot_event(&mut cfg, &snapshot_event).unwrap();

    assert_eq!(outcome, ShareAccessSnapshotApply::Applied);
    let folder = cfg.shared_folder(folder.share_id).unwrap();
    assert!(
        folder
            .projection()
            .can_write_roots(&editor.public_key().to_hex())
    );
}

#[test]
fn apply_iris_profile_roster_op_event_merges_profile_log_and_projection() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());
    let initial_op_ids = cfg
        .profile
        .as_ref()
        .unwrap()
        .profile_roster_ops
        .iter()
        .map(|op| op.op_id.clone())
        .collect::<std::collections::BTreeSet<_>>();

    let new_app = Keys::generate().public_key().to_hex();
    acct.approve_app_key(&new_app, Some("web app".to_string()))
        .unwrap();
    for op in acct
        .state
        .profile_roster_ops
        .iter()
        .filter(|op| !initial_op_ids.contains(&op.op_id))
    {
        let outcome = apply_remote_iris_profile_roster_op_event(&mut cfg, &profile_event(op))
            .expect("profile op applies");
        assert_ne!(outcome, IrisProfileRosterOpApply::NotOurProfile);
    }

    let state = cfg.profile.as_ref().unwrap();
    assert_eq!(
        state.profile_roster_ops.len(),
        acct.state.profile_roster_ops.len()
    );
    assert!(state.app_keys.as_ref().unwrap().contains(&new_app));
    assert_eq!(
        cfg.drive(crate::PRIMARY_DRIVE_ID).unwrap().root_scope_id,
        state.profile_id.to_string()
    );
}

#[test]
fn apply_iris_profile_roster_op_event_keeps_out_of_order_valid_ops() {
    let dir = tempdir().unwrap();
    let (mut cfg, acct) = config_with_owner_account(dir.path());
    let profile_id = acct.state.profile_id;
    let new_app = Keys::generate().public_key().to_hex();
    let latest = acct
        .state
        .profile_roster_ops
        .iter()
        .map(|op| op.content.created_at)
        .max()
        .unwrap();
    let add_event = build_iris_profile_roster_op_event(
        acct.app_key.keys(),
        profile_id,
        crate::iris_profile_roster_parent_ids(&acct.state.profile_roster_ops),
        None,
        IrisProfileRosterOp::AddFacet {
            facet: IrisProfileFacet::app_key(
                new_app.clone(),
                latest + 1,
                Some("tablet".to_string()),
                IrisProfileCapabilities::app_admin(),
            ),
        },
        latest + 1,
    )
    .unwrap();
    let add_op = crate::parse_iris_profile_roster_op_event(&add_event).unwrap();
    let mut set_parents = crate::iris_profile_roster_parent_ids(&acct.state.profile_roster_ops);
    set_parents.push(add_op.op_id.clone());
    let set_event = build_iris_profile_roster_op_event(
        acct.app_key.keys(),
        profile_id,
        set_parents,
        None,
        IrisProfileRosterOp::SetCapabilities {
            pubkey: new_app.clone(),
            capabilities: IrisProfileCapabilities::app_writer(),
        },
        latest + 2,
    )
    .unwrap();

    assert_eq!(
        apply_remote_iris_profile_roster_op_event(&mut cfg, &set_event).unwrap(),
        IrisProfileRosterOpApply::Applied
    );
    assert!(
        !cfg.profile
            .as_ref()
            .unwrap()
            .profile_projection()
            .active_facets
            .contains_key(&new_app)
    );

    assert_eq!(
        apply_remote_iris_profile_roster_op_event(&mut cfg, &add_event).unwrap(),
        IrisProfileRosterOpApply::Applied
    );
    let projection = cfg.profile.as_ref().unwrap().profile_projection();
    let facet = projection.active_facets.get(&new_app).unwrap();
    assert!(facet.capabilities.can_write_roots);
    assert!(!facet.capabilities.can_admin_profile);
    assert!(projection.rejected_op_ids.is_empty());
}

#[test]
fn apply_app_key_link_request_event_records_admin_inbound_request() {
    let admin_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let admin = Profile::create(admin_dir.path(), Some("admin".into())).unwrap();
    let linked = Profile::link_to_profile(
        linked_dir.path(),
        admin.state.profile_id,
        admin.state.app_key_pubkey.clone(),
        Some("phone".into()),
    )
    .unwrap();
    let frame = crate::app_key_link_transport::AppKeyLinkRequestFrame {
        schema: 1,
        profile_id: admin.state.profile_id,
        admin_app_key_pubkey: admin.state.app_key_pubkey.clone(),
        app_key_pubkey: linked.state.app_key_pubkey.clone(),
        link_secret: String::new(),
        link_secret_hash: crate::app_key_link_transport::app_key_link_secret_hash(
            &admin.state.app_key_link_secret,
        ),
        label: Some("phone".to_string()),
        requested_at: 123,
        url: "iris-drive://app-key-link?app_key=example".to_string(),
    };
    let event = build_app_key_link_request_event(linked.app_key.keys(), &frame).unwrap();
    let mut cfg = AppConfig {
        profile: Some(admin.state.clone()),
        ..AppConfig::default()
    };

    let outcome = apply_remote_app_key_link_request_event(&mut cfg, &event).unwrap();

    assert_eq!(outcome, AppKeyLinkRequestApply::Recorded);
    let inbound = &cfg.profile.as_ref().unwrap().inbound_app_key_link_requests;
    assert_eq!(inbound.len(), 1);
    assert_eq!(inbound[0].app_key_pubkey, linked.state.app_key_pubkey);
    assert_eq!(inbound[0].label.as_deref(), Some("phone"));
    assert_eq!(inbound[0].link_secret, admin.state.app_key_link_secret);
}

#[test]
fn apply_drive_root_event_from_authorized_app_key_applies() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());

    // Approve a second AppKey whose Keys we control end-to-end.
    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    acct.approve_app_key(&device_b_hex, None).unwrap();
    cfg.profile = Some(acct.state.clone());

    // Linked AppKey publishes a drive-root event.
    let root = encrypted_root(0xab, 0, 1);
    let event = build_drive_root_event(
        &device_b,
        &acct.state.root_scope_id(),
        "main",
        &root,
        &[acct.state.app_key_pubkey.clone(), device_b_hex.clone()],
    )
    .unwrap();
    let outcome =
        apply_remote_drive_root_event(&mut cfg, &event, Some(acct.app_key.keys())).unwrap();
    assert_eq!(outcome, DriveRootApply::Applied);

    let drive = cfg.drive("main").unwrap();
    let entry = drive.app_key_roots.get(&device_b_hex).unwrap();
    assert_eq!(entry.root_cid, root.root_cid);
    assert!(entry.published_at > 0); // came from event.created_at
}

#[test]
fn apply_drive_root_event_authorizes_from_roster_without_runtime_app_keys_cache() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());

    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    acct.approve_app_key(&device_b_hex, None).unwrap();
    let mut state = acct.state.clone();
    state.app_keys = None;
    state.profile_roster_projection = None;
    cfg.profile = Some(state);

    let root = encrypted_root(0xad, 0, 1);
    let event = build_drive_root_event(
        &device_b,
        &acct.state.root_scope_id(),
        "main",
        &root,
        &[acct.state.app_key_pubkey.clone(), device_b_hex.clone()],
    )
    .unwrap();
    let outcome =
        apply_remote_drive_root_event(&mut cfg, &event, Some(acct.app_key.keys())).unwrap();

    assert_eq!(outcome, DriveRootApply::Applied);
    assert_eq!(
        cfg.drive("main")
            .unwrap()
            .app_key_roots
            .get(&device_b_hex)
            .map(|entry| entry.root_cid.as_str()),
        Some(root.root_cid.as_str())
    );
}

#[test]
fn apply_drive_root_event_authorizes_from_existing_root_without_roster_or_cache() {
    let dir = tempdir().unwrap();
    let (mut cfg, acct) = config_with_owner_account(dir.path());

    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    {
        let drive = cfg
            .drives
            .iter_mut()
            .find(|drive| drive.drive_id == "main")
            .unwrap();
        drive.app_key_roots.insert(
            acct.state.app_key_pubkey.clone(),
            encrypted_root(0xa1, 10, 1),
        );
        drive
            .app_key_roots
            .insert(device_b_hex.clone(), encrypted_root(0xa2, 11, 1));
    }
    let mut state = acct.state.clone();
    state.profile_roster_ops = Vec::new();
    state.app_keys = None;
    state.profile_roster_projection = None;
    cfg.profile = Some(state);

    let root = encrypted_root(0xa3, 20, 1);
    let event = build_drive_root_event(
        &device_b,
        &acct.state.root_scope_id(),
        "main",
        &root,
        &[acct.state.app_key_pubkey.clone(), device_b_hex.clone()],
    )
    .unwrap();
    let outcome =
        apply_remote_drive_root_event(&mut cfg, &event, Some(acct.app_key.keys())).unwrap();

    assert_eq!(outcome, DriveRootApply::Applied);
    assert_eq!(
        cfg.drive("main")
            .unwrap()
            .app_key_roots
            .get(&device_b_hex)
            .map(|entry| entry.root_cid.as_str()),
        Some(root.root_cid.as_str())
    );
}

#[test]
fn apply_drive_root_event_without_local_wrap_is_skipped() {
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let (_, mut owner_acct) = config_with_owner_account(owner_dir.path());

    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    owner_acct
        .approve_app_key(&device_b_hex, Some("old-phone".into()))
        .unwrap();

    let linked = Profile::link_to_profile(
        linked_dir.path(),
        owner_acct.state.profile_id,
        owner_acct.state.app_key_pubkey.clone(),
        Some("new-laptop".into()),
    )
    .unwrap();
    let mut linked_state = linked.state.clone();
    linked_state.app_keys = owner_acct.state.app_keys.clone();

    let mut cfg = AppConfig {
        profile: Some(linked_state),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(owner_acct.state.root_scope_id()));

    let root = encrypted_root(0xac, 0, 1);
    let event = build_drive_root_event(
        &device_b,
        &owner_acct.state.root_scope_id(),
        "main",
        &root,
        &[owner_acct.state.app_key_pubkey.clone(), device_b_hex],
    )
    .unwrap();
    let outcome =
        apply_remote_drive_root_event(&mut cfg, &event, Some(linked.app_key.keys())).unwrap();

    assert_eq!(outcome, DriveRootApply::KeyUnavailable);
    assert!(cfg.drive("main").unwrap().app_key_roots.is_empty());
}

#[test]
fn apply_files_root_event_from_current_app_key_maps_to_current_app_key() {
    let dir = tempdir().unwrap();
    let (mut cfg, acct) = config_with_owner_account(dir.path());
    let root = encrypted_root(0x5a, 1_700_000_000, 0);
    let event = build_private_hashtree_root_event(acct.app_key.keys(), "main", &root).unwrap();

    let outcome =
        apply_remote_files_root_event(&mut cfg, &event, Some(acct.app_key.keys())).unwrap();

    assert_eq!(outcome, FilesRootApply::Applied);
    let entry = cfg
        .drive("main")
        .unwrap()
        .app_key_roots
        .get(&acct.state.app_key_pubkey)
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
        .app_key_roots
        .insert(acct.state.app_key_pubkey.clone(), native_root.clone());
    let legacy_root = encrypted_root(0x5c, 1_700_000_000, 0);
    let event =
        build_private_hashtree_root_event(acct.app_key.keys(), "main", &legacy_root).unwrap();

    let outcome =
        apply_remote_files_root_event(&mut cfg, &event, Some(acct.app_key.keys())).unwrap();

    assert_eq!(outcome, FilesRootApply::StaleTimestamp);
    let entry = cfg
        .drive("main")
        .unwrap()
        .app_key_roots
        .get(&acct.state.app_key_pubkey)
        .unwrap();
    assert_eq!(entry.root_cid, native_root.root_cid);
    assert_eq!(entry.app_key_seq, 4);
}

#[test]
fn apply_files_root_event_ignores_same_root_with_newer_timestamp() {
    let dir = tempdir().unwrap();
    let (mut cfg, acct) = config_with_owner_account(dir.path());
    let mut root = encrypted_root(0x5d, 100, 0);
    let local_keys = acct.app_key.keys();
    let first = build_private_hashtree_root_event(local_keys, "main", &root).unwrap();

    assert_eq!(
        apply_remote_files_root_event(&mut cfg, &first, Some(local_keys)).unwrap(),
        FilesRootApply::Applied
    );

    root.published_at = 200;
    let republished = build_private_hashtree_root_event(local_keys, "main", &root).unwrap();

    assert_eq!(
        apply_remote_files_root_event(&mut cfg, &republished, Some(local_keys)).unwrap(),
        FilesRootApply::StaleTimestamp
    );
    let entry = cfg
        .drive("main")
        .unwrap()
        .app_key_roots
        .get(&acct.state.app_key_pubkey)
        .unwrap();
    assert_eq!(entry.root_cid, root.root_cid);
    assert_eq!(entry.published_at, 100);
}

#[test]
fn apply_files_root_event_from_foreign_app_key_ignored() {
    let dir = tempdir().unwrap();
    let (mut cfg, acct) = config_with_owner_account(dir.path());
    let other_app_key = Keys::generate();
    let root = encrypted_root(0x61, 1_700_000_000, 0);
    let event = build_private_hashtree_root_event(&other_app_key, "main", &root).unwrap();

    let outcome =
        apply_remote_files_root_event(&mut cfg, &event, Some(acct.app_key.keys())).unwrap();

    assert_eq!(outcome, FilesRootApply::NotOurAppKey);
    assert!(cfg.drive("main").unwrap().app_key_roots.is_empty());
}

#[test]
fn apply_drive_root_event_from_unauthorized_app_key_ignored() {
    let dir = tempdir().unwrap();
    let (mut cfg, _) = config_with_owner_account(dir.path());
    let stranger = Keys::generate(); // not in roster

    let root = encrypted_root(0xee, 0, 99);
    let root_scope_id = cfg.profile.as_ref().unwrap().root_scope_id();
    let recipient = cfg.profile.as_ref().unwrap().app_key_pubkey.clone();
    let outcome = {
        let event =
            build_drive_root_event(&stranger, &root_scope_id, "main", &root, &[recipient]).unwrap();
        apply_remote_drive_root_event(&mut cfg, &event, None).unwrap()
    };
    assert_eq!(outcome, DriveRootApply::UnauthorizedAppKey);
    assert!(cfg.drive("main").unwrap().app_key_roots.is_empty());
}

#[test]
fn apply_drive_root_event_for_foreign_scope_ignored() {
    let dir = tempdir().unwrap();
    let (mut cfg, _) = config_with_owner_account(dir.path());
    let other_scope = IrisProfileId::new_v4().to_string();
    let device_key = Keys::generate();
    let root = encrypted_root(0xf0, 0, 1);
    let event = build_drive_root_event(
        &device_key,
        &other_scope,
        "main",
        &root,
        &[cfg.profile.as_ref().unwrap().app_key_pubkey.clone()],
    )
    .unwrap();
    let outcome = apply_remote_drive_root_event(&mut cfg, &event, None).unwrap();
    assert_eq!(outcome, DriveRootApply::NotOurScope);
}

#[test]
fn apply_share_root_event_from_authorized_publisher_applies_to_shared_folder() {
    let owner_dir = tempdir().unwrap();
    let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
    let reader_dir = tempdir().unwrap();
    let reader = Profile::create(reader_dir.path(), Some("Reader".into())).unwrap();
    let folder = crate::create_shared_folder(
        owner.app_key.keys(),
        owner.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Owner".into()),
        vec![ShareRecipient {
            profile_id: reader.state.profile_id,
            app_pubkey: reader.state.app_key_pubkey.clone(),
            role: ShareRole::Reader,
            label: Some("Reader".into()),
            representative_npub_hint: None,
            display_name: Some("Reader".into()),
        }],
        10,
    )
    .unwrap();
    let root = causal_encrypted_root(0x44, 20, 1, 7);
    let authorized_recipients = folder
        .projection()
        .active_facets
        .values()
        .filter(|facet| facet.capabilities.can_receive_key_wraps)
        .map(|facet| facet.pubkey.clone())
        .collect::<Vec<_>>();
    let event = build_drive_root_event(
        owner.app_key.keys(),
        &folder.share_id.to_string(),
        crate::PRIMARY_DRIVE_ID,
        &root,
        &authorized_recipients,
    )
    .unwrap();
    let mut cfg = AppConfig {
        profile: Some(reader.state.clone()),
        shared_folders: vec![folder.clone()],
        ..AppConfig::default()
    };

    let outcome = apply_remote_drive_root_event(&mut cfg, &event, Some(reader.app_key.keys()))
        .expect("share root applies");

    assert_eq!(outcome, DriveRootApply::Applied);
    let stored = cfg
        .shared_folder(folder.share_id)
        .unwrap()
        .app_key_roots
        .get(&owner.state.app_key_pubkey)
        .expect("owner share root stored");
    assert_eq!(stored.root_cid, root.root_cid);
    assert_eq!(stored.app_key_seq, 7);
}

#[test]
fn apply_share_root_event_rejects_reader_publisher() {
    let owner_dir = tempdir().unwrap();
    let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
    let reader_dir = tempdir().unwrap();
    let reader = Profile::create(reader_dir.path(), Some("Reader".into())).unwrap();
    let folder = crate::create_shared_folder(
        owner.app_key.keys(),
        owner.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Owner".into()),
        vec![ShareRecipient {
            profile_id: reader.state.profile_id,
            app_pubkey: reader.state.app_key_pubkey.clone(),
            role: ShareRole::Reader,
            label: Some("Reader".into()),
            representative_npub_hint: None,
            display_name: Some("Reader".into()),
        }],
        10,
    )
    .unwrap();
    let root = causal_encrypted_root(0x45, 20, 1, 1);
    let authorized_recipients = folder
        .projection()
        .active_facets
        .values()
        .filter(|facet| facet.capabilities.can_receive_key_wraps)
        .map(|facet| facet.pubkey.clone())
        .collect::<Vec<_>>();
    let event = build_drive_root_event(
        reader.app_key.keys(),
        &folder.share_id.to_string(),
        crate::PRIMARY_DRIVE_ID,
        &root,
        &authorized_recipients,
    )
    .unwrap();
    let mut cfg = AppConfig {
        profile: Some(owner.state.clone()),
        shared_folders: vec![folder.clone()],
        ..AppConfig::default()
    };

    let outcome = apply_remote_drive_root_event(&mut cfg, &event, Some(owner.app_key.keys()))
        .expect("reader share root is inspected");

    assert_eq!(outcome, DriveRootApply::UnauthorizedAppKey);
    assert!(
        cfg.shared_folder(folder.share_id)
            .unwrap()
            .app_key_roots
            .is_empty()
    );
}

#[test]
fn apply_share_root_event_rejects_revoked_profile_member() {
    let owner_dir = tempdir().unwrap();
    let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
    let writer_dir = tempdir().unwrap();
    let writer = Profile::create(writer_dir.path(), Some("Writer".into())).unwrap();
    let mut folder = crate::create_shared_folder(
        owner.app_key.keys(),
        owner.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Owner".into()),
        vec![ShareRecipient {
            profile_id: writer.state.profile_id,
            app_pubkey: writer.state.app_key_pubkey.clone(),
            role: ShareRole::Editor,
            label: Some("Writer".into()),
            representative_npub_hint: None,
            display_name: Some("Writer".into()),
        }],
        10,
    )
    .unwrap();
    crate::revoke_shared_folder_member(
        &mut folder,
        owner.app_key.keys(),
        writer.state.profile_id,
        None,
        20,
    )
    .unwrap();
    assert!(
        !folder
            .projection()
            .can_write_roots(&writer.state.app_key_pubkey)
    );
    assert!(!crate::shared_folder_app_key_can_write_roots(
        &folder,
        &writer.state.app_key_pubkey
    ));
    let root = causal_encrypted_root(0x46, 20, 1, 1);
    let authorized_recipients = folder
        .projection()
        .active_facets
        .values()
        .filter(|facet| facet.capabilities.can_receive_key_wraps)
        .map(|facet| facet.pubkey.clone())
        .collect::<Vec<_>>();
    let event = build_drive_root_event(
        writer.app_key.keys(),
        &folder.share_id.to_string(),
        crate::PRIMARY_DRIVE_ID,
        &root,
        &authorized_recipients,
    )
    .unwrap();
    let mut cfg = AppConfig {
        profile: Some(owner.state.clone()),
        shared_folders: vec![folder.clone()],
        ..AppConfig::default()
    };

    let outcome = apply_remote_drive_root_event(&mut cfg, &event, Some(owner.app_key.keys()))
        .expect("writer share root is inspected");

    assert_eq!(outcome, DriveRootApply::UnauthorizedAppKey);
    assert!(
        cfg.shared_folder(folder.share_id)
            .unwrap()
            .app_key_roots
            .is_empty()
    );
}

#[test]
fn apply_drive_root_event_for_unknown_drive_ignored() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());
    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    acct.approve_app_key(&device_b_hex, None).unwrap();
    cfg.profile = Some(acct.state.clone());
    let root = encrypted_root(0x44, 0, 1);
    let event = build_drive_root_event(
        &device_b,
        &acct.state.root_scope_id(),
        "nonexistent",
        &root,
        &[acct.state.app_key_pubkey.clone(), device_b_hex],
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
    acct.approve_app_key(&device_b_hex, None).unwrap();
    cfg.profile = Some(acct.state.clone());

    // First publish — applied.
    let root_1 = encrypted_root(0x11, 0, 1);
    let event_1 = build_drive_root_event(
        &device_b,
        &acct.state.root_scope_id(),
        "main",
        &root_1,
        &[acct.state.app_key_pubkey.clone(), device_b_hex.clone()],
    )
    .unwrap();
    assert_eq!(
        apply_remote_drive_root_event(&mut cfg, &event_1, Some(acct.app_key.keys())).unwrap(),
        DriveRootApply::Applied
    );
    let first_published_at = cfg
        .drive("main")
        .unwrap()
        .app_key_roots
        .get(&device_b_hex)
        .unwrap()
        .published_at;

    // Replay the same event — same created_at, should be StaleTimestamp.
    assert_eq!(
        apply_remote_drive_root_event(&mut cfg, &event_1, None).unwrap(),
        DriveRootApply::StaleTimestamp
    );
    // app_key_roots entry unchanged.
    assert_eq!(
        cfg.drive("main")
            .unwrap()
            .app_key_roots
            .get(&device_b_hex)
            .unwrap()
            .root_cid,
        root_1.root_cid
    );
    assert_eq!(
        cfg.drive("main")
            .unwrap()
            .app_key_roots
            .get(&device_b_hex)
            .unwrap()
            .published_at,
        first_published_at
    );
}

#[test]
fn apply_drive_root_event_ignores_republished_root_without_causal_fields() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());
    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    acct.approve_app_key(&device_b_hex, None).unwrap();
    cfg.profile = Some(acct.state.clone());

    let mut root = encrypted_root(0x13, 100, 1);
    let event_1 = build_drive_root_event(
        &device_b,
        &acct.state.root_scope_id(),
        "main",
        &root,
        &[acct.state.app_key_pubkey.clone(), device_b_hex.clone()],
    )
    .unwrap();
    assert_eq!(
        apply_remote_drive_root_event(&mut cfg, &event_1, Some(acct.app_key.keys())).unwrap(),
        DriveRootApply::Applied
    );

    root.published_at = 200;
    let republished = build_drive_root_event(
        &device_b,
        &acct.state.root_scope_id(),
        "main",
        &root,
        &[acct.state.app_key_pubkey.clone(), device_b_hex.clone()],
    )
    .unwrap();

    assert_eq!(
        apply_remote_drive_root_event(&mut cfg, &republished, Some(acct.app_key.keys())).unwrap(),
        DriveRootApply::StaleTimestamp
    );
    let entry = cfg
        .drive("main")
        .unwrap()
        .app_key_roots
        .get(&device_b_hex)
        .unwrap();
    assert_eq!(entry.root_cid, root.root_cid);
    assert_eq!(entry.published_at, 100);
}

#[test]
fn apply_drive_root_event_prefers_higher_app_key_seq_over_newer_timestamp() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());
    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    acct.approve_app_key(&device_b_hex, None).unwrap();
    cfg.profile = Some(acct.state.clone());

    let root_1 = causal_encrypted_root(0x21, 300, 1, 1);
    let event_1 = build_drive_root_event(
        &device_b,
        &acct.state.root_scope_id(),
        "main",
        &root_1,
        &[acct.state.app_key_pubkey.clone(), device_b_hex.clone()],
    )
    .unwrap();
    assert_eq!(
        apply_remote_drive_root_event(&mut cfg, &event_1, Some(acct.app_key.keys())).unwrap(),
        DriveRootApply::Applied
    );

    let root_2 = causal_encrypted_root(0x22, 100, 1, 2);
    let event_2 = build_drive_root_event(
        &device_b,
        &acct.state.root_scope_id(),
        "main",
        &root_2,
        &[acct.state.app_key_pubkey.clone(), device_b_hex.clone()],
    )
    .unwrap();
    assert_eq!(
        apply_remote_drive_root_event(&mut cfg, &event_2, Some(acct.app_key.keys())).unwrap(),
        DriveRootApply::Applied
    );

    let entry = cfg
        .drive("main")
        .unwrap()
        .app_key_roots
        .get(&device_b_hex)
        .unwrap();
    assert_eq!(entry.root_cid, root_2.root_cid);
    assert_eq!(entry.app_key_seq, 2);
    assert_eq!(entry.published_at, 100);
}

#[test]
fn same_second_drive_root_selection_prefers_higher_app_key_seq() {
    let device = Keys::generate();
    let root_scope_id = IrisProfileId::new_v4().to_string();
    let older = causal_encrypted_root(0x31, 1_700_000_000, 1, 1);
    let newer = causal_encrypted_root(0x32, 1_700_000_000, 1, 2);
    let authorized = vec![device.public_key().to_hex()];
    let older_event =
        build_drive_root_event(&device, &root_scope_id, "main", &older, &authorized).unwrap();
    let newer_event =
        build_drive_root_event(&device, &root_scope_id, "main", &newer, &authorized).unwrap();

    assert!(drive_root_event_is_newer(&newer_event, &older_event));
    assert!(!drive_root_event_is_newer(&older_event, &newer_event));
}
