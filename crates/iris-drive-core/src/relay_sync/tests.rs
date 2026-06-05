use super::*;
use crate::account::{Account, DeviceAuthorizationState};
use crate::config::Drive;
use crate::device_link_transport::DeviceLinkRosterFrame;
use crate::iris_profile::{
    IrisProfileCapabilities, IrisProfileFacet, IrisProfileId, IrisProfileRosterOp,
    build_iris_profile_roster_op_event,
};
use crate::nostr_events::{
    build_device_link_request_event, build_drive_root_event, build_private_hashtree_root_event,
    device_link_request_d_tag,
};
use crate::sharing::{ShareRecipient, ShareRole};
use hashtree_core::Cid;
use tempfile::tempdir;

fn config_with_owner_account(dir: &std::path::Path) -> (AppConfig, Account) {
    let acct = Account::create(dir, None).unwrap();
    let mut cfg = AppConfig {
        account: Some(acct.state.clone()),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(acct.state.root_scope_id()));
    (cfg, acct)
}

fn profile_event(op: &crate::SignedIrisProfileRosterOp) -> Event {
    Event::from_json(&op.event_json).unwrap()
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
        local_only: false,
    }
}

fn roster_frame(
    admin: &Account,
    profile_roster_ops: Vec<crate::SignedIrisProfileRosterOp>,
    sent_at: u64,
) -> DeviceLinkRosterFrame {
    DeviceLinkRosterFrame {
        schema: 1,
        profile_id: admin.state.profile_id,
        admin_device_pubkey: admin.state.device_pubkey.clone(),
        profile_roster_ops,
        sent_at,
    }
}

fn linked_config_after_initial_roster() -> (Account, AppConfig) {
    let admin_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let mut admin = Account::create(admin_dir.path(), Some("admin".into())).unwrap();
    let mut linked = Account::link_to_profile(
        linked_dir.path(),
        admin.state.profile_id,
        admin.state.device_pubkey.clone(),
        Some("phone".into()),
    )
    .unwrap();
    let linked_pubkey = linked.state.device_pubkey.clone();
    linked
        .state
        .queue_outbound_device_link_request(
            admin.state.device_pubkey.clone(),
            &admin.state.device_link_secret,
            123,
        )
        .unwrap();

    admin
        .approve_device(&linked_pubkey, Some("phone".into()))
        .unwrap();
    let mut cfg = AppConfig {
        account: Some(linked.state.clone()),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(admin.state.root_scope_id()));

    let first_frame = roster_frame(&admin, admin.state.profile_roster_ops.clone(), 456);
    let initial =
        apply_device_link_roster_frame(&mut cfg, &first_frame, &admin.state.device_pubkey).unwrap();
    assert!(matches!(
        initial,
        DeviceLinkRosterApply::Applied(ApplyDecision::Adopted)
    ));
    assert_eq!(
        cfg.account.as_ref().unwrap().authorization_state,
        DeviceAuthorizationState::Authorized
    );
    assert_eq!(
        cfg.account.as_ref().unwrap().profile_id,
        admin.state.profile_id
    );
    assert_eq!(
        cfg.drive(crate::PRIMARY_DRIVE_ID).unwrap().root_scope_id,
        admin.state.profile_id.to_string()
    );
    (admin, cfg)
}

#[test]
fn apply_device_link_roster_accepts_newer_admin_roster_after_initial_approval() {
    let (mut admin, mut cfg) = linked_config_after_initial_roster();

    let third_device = Keys::generate().public_key().to_hex();
    admin
        .approve_device(&third_device, Some("tablet".into()))
        .unwrap();

    let newer_frame = roster_frame(&admin, admin.state.profile_roster_ops.clone(), 789);
    let update =
        apply_device_link_roster_frame(&mut cfg, &newer_frame, &admin.state.device_pubkey).unwrap();
    assert!(matches!(
        update,
        DeviceLinkRosterApply::Applied(ApplyDecision::Replaced)
    ));
    let linked_state = cfg.account.as_ref().unwrap();
    let linked_roster = linked_state.app_keys.as_ref().unwrap();
    assert!(linked_roster.contains(&linked_state.device_pubkey));
    assert!(linked_roster.contains(&third_device));
    assert!(linked_state.outbound_device_link_request.is_none());
}

#[test]
fn apply_device_link_roster_is_profile_scoped_and_ownerless() {
    let admin_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let mut admin = Account::create(admin_dir.path(), Some("admin".into())).unwrap();
    let mut linked = Account::link_to_profile(
        linked_dir.path(),
        admin.state.profile_id,
        admin.state.device_pubkey.clone(),
        Some("phone".into()),
    )
    .unwrap();
    let linked_pubkey = linked.state.device_pubkey.clone();
    linked
        .state
        .queue_outbound_device_link_request(
            admin.state.device_pubkey.clone(),
            &admin.state.device_link_secret,
            123,
        )
        .unwrap();
    admin
        .approve_device(&linked_pubkey, Some("phone".into()))
        .unwrap();
    let mut cfg = AppConfig {
        account: Some(linked.state.clone()),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(admin.state.root_scope_id()));
    let frame = roster_frame(&admin, admin.state.profile_roster_ops.clone(), 456);

    let outcome =
        apply_device_link_roster_frame(&mut cfg, &frame, &admin.state.device_pubkey).unwrap();

    assert!(matches!(
        outcome,
        DeviceLinkRosterApply::Applied(ApplyDecision::Adopted)
    ));
    let linked_state = cfg.account.as_ref().unwrap();
    assert_eq!(linked_state.profile_id, admin.state.profile_id);
    assert_eq!(
        linked_state.app_keys.as_ref().unwrap().profile_id,
        admin.state.profile_id.to_string()
    );
}

#[test]
fn apply_device_link_roster_merges_older_branch_without_downgrading_epoch() {
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
        admin.device.keys(),
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
        apply_device_link_roster_frame(&mut cfg, &current_frame, &admin.state.device_pubkey)
            .unwrap(),
        DeviceLinkRosterApply::Applied(ApplyDecision::Replaced)
    ));
    assert!(
        !cfg.account
            .as_ref()
            .unwrap()
            .profile_roster_ops
            .iter()
            .any(|op| op.op_id == branch_op.op_id)
    );

    let branch_frame = roster_frame(&admin, branch_ops, 999);
    assert!(matches!(
        apply_device_link_roster_frame(&mut cfg, &branch_frame, &admin.state.device_pubkey)
            .unwrap(),
        DeviceLinkRosterApply::Applied(ApplyDecision::Merged)
    ));

    let linked_state = cfg.account.as_ref().unwrap();
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
fn subscription_filters_match_device_link_requests_for_profile() {
    let device = Keys::generate();
    let profile_id = IrisProfileId::new_v4();
    let frame = crate::device_link_transport::DeviceLinkRequestFrame {
        schema: 1,
        profile_id,
        device_pubkey: device.public_key().to_hex(),
        link_secret: "join-secret".to_string(),
        label: Some("phone".to_string()),
        requested_at: 123,
        url: "iris-drive://device-link?device=example".to_string(),
    };
    let event = build_device_link_request_event(&device, &frame).unwrap();

    assert_eq!(
        event.identifier(),
        Some(device_link_request_d_tag(profile_id).as_str())
    );
    assert!(
        subscription_filters(
            &device.public_key().to_hex(),
            &profile_id.to_string(),
            "main"
        )
        .iter()
        .any(|filter| filter.match_event(&event))
    );
}

#[test]
fn subscription_filters_match_iris_profile_roster_ops_for_profile() {
    let dir = tempdir().unwrap();
    let (cfg, acct) = config_with_owner_account(dir.path());
    let profile_op = profile_event(&acct.state.profile_roster_ops[0]);

    assert!(
        subscription_filters(
            &acct.state.device_pubkey,
            &acct.state.root_scope_id(),
            crate::PRIMARY_DRIVE_ID,
        )
        .iter()
        .any(|filter| filter.match_event(&profile_op))
    );
    let profile_id = cfg.account.as_ref().unwrap().profile_id.to_string();
    assert_eq!(
        profile_op.get_tag_content(nostr_sdk::TagKind::from("i")),
        Some(profile_id.as_str())
    );
}

#[test]
fn subscription_filters_match_share_roster_ops_and_roots() {
    let dir = tempdir().unwrap();
    let (_, owner) = config_with_owner_account(dir.path());
    let reader = Keys::generate();
    let folder = crate::create_shared_folder(
        owner.device.keys(),
        owner.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Owner".into()),
        vec![ShareRecipient {
            profile_id: IrisProfileId::new_v4(),
            app_pubkey: reader.public_key().to_hex(),
            role: ShareRole::Reader,
            label: Some("Reader".into()),
        }],
        10,
    )
    .unwrap();
    let share_op = profile_event(&folder.roster_ops[0]);
    let root = encrypted_root(0x55, 20, 1);
    let root_event = build_drive_root_event(
        owner.device.keys(),
        &folder.share_id.to_string(),
        crate::PRIMARY_DRIVE_ID,
        &root,
        &[
            owner.state.device_pubkey.clone(),
            reader.public_key().to_hex(),
        ],
    )
    .unwrap();

    let filters = subscription_filters_for_shared_roots(
        &owner.state.device_pubkey,
        &owner.state.root_scope_id(),
        crate::PRIMARY_DRIVE_ID,
        &[folder.share_id],
    );

    assert!(filters.iter().any(|filter| filter.match_event(&share_op)));
    assert!(filters.iter().any(|filter| filter.match_event(&root_event)));
}

#[test]
fn apply_share_roster_op_event_merges_known_shared_folder() {
    let dir = tempdir().unwrap();
    let (_, owner) = config_with_owner_account(dir.path());
    let editor = Keys::generate();
    let mut folder = crate::create_shared_folder(
        owner.device.keys(),
        owner.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Owner".into()),
        Vec::new(),
        10,
    )
    .unwrap();
    let op_event = build_iris_profile_roster_op_event(
        owner.device.keys(),
        folder.share_id,
        crate::iris_profile_roster_parent_ids(&folder.roster_ops),
        None,
        IrisProfileRosterOp::AddFacet {
            facet: IrisProfileFacet::app_key(
                editor.public_key().to_hex(),
                20,
                Some("Editor".into()),
                IrisProfileCapabilities::app_writer(),
            ),
        },
        20,
    )
    .unwrap();
    let mut cfg = AppConfig {
        account: Some(owner.state.clone()),
        shared_folders: vec![folder.clone()],
        ..AppConfig::default()
    };

    let outcome = apply_remote_iris_profile_roster_op_event(&mut cfg, &op_event).unwrap();

    assert_eq!(outcome, IrisProfileRosterOpApply::Applied);
    folder = cfg.shared_folder(folder.share_id).unwrap().clone();
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
        .account
        .as_ref()
        .unwrap()
        .profile_roster_ops
        .iter()
        .map(|op| op.op_id.clone())
        .collect::<std::collections::BTreeSet<_>>();

    let new_app = Keys::generate().public_key().to_hex();
    acct.approve_device(&new_app, Some("web app".to_string()))
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

    let state = cfg.account.as_ref().unwrap();
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
        acct.device.keys(),
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
        acct.device.keys(),
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
        !cfg.account
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
    let projection = cfg.account.as_ref().unwrap().profile_projection();
    let facet = projection.active_facets.get(&new_app).unwrap();
    assert!(facet.capabilities.can_write_roots);
    assert!(!facet.capabilities.can_admin_profile);
    assert!(projection.rejected_op_ids.is_empty());
}

#[test]
fn apply_device_link_request_event_records_admin_inbound_request() {
    let admin_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let admin = Account::create(admin_dir.path(), Some("admin".into())).unwrap();
    let linked = Account::link_to_profile(
        linked_dir.path(),
        admin.state.profile_id,
        admin.state.device_pubkey.clone(),
        Some("phone".into()),
    )
    .unwrap();
    let frame = crate::device_link_transport::DeviceLinkRequestFrame {
        schema: 1,
        profile_id: admin.state.profile_id,
        device_pubkey: linked.state.device_pubkey.clone(),
        link_secret: admin.state.device_link_secret.clone(),
        label: Some("phone".to_string()),
        requested_at: 123,
        url: "iris-drive://device-link?device=example".to_string(),
    };
    let event = build_device_link_request_event(linked.device.keys(), &frame).unwrap();
    let mut cfg = AppConfig {
        account: Some(admin.state.clone()),
        ..AppConfig::default()
    };

    let outcome = apply_remote_device_link_request_event(&mut cfg, &event).unwrap();

    assert_eq!(outcome, DeviceLinkRequestApply::Recorded);
    let inbound = &cfg.account.as_ref().unwrap().inbound_device_link_requests;
    assert_eq!(inbound.len(), 1);
    assert_eq!(inbound[0].device_pubkey, linked.state.device_pubkey);
    assert_eq!(inbound[0].label.as_deref(), Some("phone"));
}

#[test]
fn apply_drive_root_event_from_authorized_device_applies() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());

    // Approve a second device whose Keys we control end-to-end.
    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    acct.approve_device(&device_b_hex, None).unwrap();
    cfg.account = Some(acct.state.clone());

    // Device B publishes a drive-root event.
    let root = encrypted_root(0xab, 0, 1);
    let event = build_drive_root_event(
        &device_b,
        &acct.state.root_scope_id(),
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
        .approve_device(&device_b_hex, Some("old-phone".into()))
        .unwrap();

    let linked = Account::link_to_profile(
        linked_dir.path(),
        owner_acct.state.profile_id,
        owner_acct.state.device_pubkey.clone(),
        Some("new-laptop".into()),
    )
    .unwrap();
    let mut linked_state = linked.state.clone();
    linked_state.app_keys = owner_acct.state.app_keys.clone();

    let mut cfg = AppConfig {
        account: Some(linked_state),
        ..AppConfig::default()
    };
    cfg.upsert_drive(Drive::primary(owner_acct.state.root_scope_id()));

    let root = encrypted_root(0xac, 0, 1);
    let event = build_drive_root_event(
        &device_b,
        &owner_acct.state.root_scope_id(),
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
fn apply_files_root_event_from_current_app_key_maps_to_current_device() {
    let dir = tempdir().unwrap();
    let (mut cfg, acct) = config_with_owner_account(dir.path());
    let root = encrypted_root(0x5a, 1_700_000_000, 0);
    let event = build_private_hashtree_root_event(acct.device.keys(), "main", &root).unwrap();

    let outcome =
        apply_remote_files_root_event(&mut cfg, &event, Some(acct.device.keys())).unwrap();

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
    let event =
        build_private_hashtree_root_event(acct.device.keys(), "main", &legacy_root).unwrap();

    let outcome =
        apply_remote_files_root_event(&mut cfg, &event, Some(acct.device.keys())).unwrap();

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
    let owner_keys = acct.device.keys();
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
fn apply_files_root_event_from_foreign_app_key_ignored() {
    let dir = tempdir().unwrap();
    let (mut cfg, _) = config_with_owner_account(dir.path());
    let other_owner = Keys::generate();
    let root = encrypted_root(0x61, 1_700_000_000, 0);
    let event = build_private_hashtree_root_event(&other_owner, "main", &root).unwrap();

    let outcome = apply_remote_files_root_event(&mut cfg, &event, Some(&other_owner)).unwrap();

    assert_eq!(outcome, FilesRootApply::NotOurAppKey);
    assert!(cfg.drive("main").unwrap().device_roots.is_empty());
}

#[test]
fn apply_drive_root_event_from_unauthorized_device_ignored() {
    let dir = tempdir().unwrap();
    let (mut cfg, _) = config_with_owner_account(dir.path());
    let stranger = Keys::generate(); // not in roster

    let root = encrypted_root(0xee, 0, 99);
    let owner_hex = cfg.account.as_ref().unwrap().root_scope_id();
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
    let other_owner = IrisProfileId::new_v4().to_string();
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
fn apply_share_root_event_from_authorized_publisher_applies_to_shared_folder() {
    let owner_dir = tempdir().unwrap();
    let owner = Account::create(owner_dir.path(), Some("Owner".into())).unwrap();
    let reader_dir = tempdir().unwrap();
    let reader = Account::create(reader_dir.path(), Some("Reader".into())).unwrap();
    let folder = crate::create_shared_folder(
        owner.device.keys(),
        owner.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Owner".into()),
        vec![ShareRecipient {
            profile_id: reader.state.profile_id,
            app_pubkey: reader.state.device_pubkey.clone(),
            role: ShareRole::Reader,
            label: Some("Reader".into()),
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
        owner.device.keys(),
        &folder.share_id.to_string(),
        crate::PRIMARY_DRIVE_ID,
        &root,
        &authorized_recipients,
    )
    .unwrap();
    let mut cfg = AppConfig {
        account: Some(reader.state.clone()),
        shared_folders: vec![folder.clone()],
        ..AppConfig::default()
    };

    let outcome = apply_remote_drive_root_event(&mut cfg, &event, Some(reader.device.keys()))
        .expect("share root applies");

    assert_eq!(outcome, DriveRootApply::Applied);
    let stored = cfg
        .shared_folder(folder.share_id)
        .unwrap()
        .device_roots
        .get(&owner.state.device_pubkey)
        .expect("owner share root stored");
    assert_eq!(stored.root_cid, root.root_cid);
    assert_eq!(stored.device_seq, 7);
}

#[test]
fn apply_share_root_event_rejects_reader_publisher() {
    let owner_dir = tempdir().unwrap();
    let owner = Account::create(owner_dir.path(), Some("Owner".into())).unwrap();
    let reader_dir = tempdir().unwrap();
    let reader = Account::create(reader_dir.path(), Some("Reader".into())).unwrap();
    let folder = crate::create_shared_folder(
        owner.device.keys(),
        owner.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Owner".into()),
        vec![ShareRecipient {
            profile_id: reader.state.profile_id,
            app_pubkey: reader.state.device_pubkey.clone(),
            role: ShareRole::Reader,
            label: Some("Reader".into()),
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
        reader.device.keys(),
        &folder.share_id.to_string(),
        crate::PRIMARY_DRIVE_ID,
        &root,
        &authorized_recipients,
    )
    .unwrap();
    let mut cfg = AppConfig {
        account: Some(owner.state.clone()),
        shared_folders: vec![folder.clone()],
        ..AppConfig::default()
    };

    let outcome = apply_remote_drive_root_event(&mut cfg, &event, Some(owner.device.keys()))
        .expect("reader share root is inspected");

    assert_eq!(outcome, DriveRootApply::UnauthorizedDevice);
    assert!(
        cfg.shared_folder(folder.share_id)
            .unwrap()
            .device_roots
            .is_empty()
    );
}

#[test]
fn apply_drive_root_event_for_unknown_drive_ignored() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());
    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    acct.approve_device(&device_b_hex, None).unwrap();
    cfg.account = Some(acct.state.clone());
    let root = encrypted_root(0x44, 0, 1);
    let event = build_drive_root_event(
        &device_b,
        &acct.state.root_scope_id(),
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
    acct.approve_device(&device_b_hex, None).unwrap();
    cfg.account = Some(acct.state.clone());

    // First publish — applied.
    let root_1 = encrypted_root(0x11, 0, 1);
    let event_1 = build_drive_root_event(
        &device_b,
        &acct.state.root_scope_id(),
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
fn apply_drive_root_event_ignores_republished_root_without_causal_fields() {
    let dir = tempdir().unwrap();
    let (mut cfg, mut acct) = config_with_owner_account(dir.path());
    let device_b = Keys::generate();
    let device_b_hex = device_b.public_key().to_hex();
    acct.approve_device(&device_b_hex, None).unwrap();
    cfg.account = Some(acct.state.clone());

    let mut root = encrypted_root(0x13, 100, 1);
    let event_1 = build_drive_root_event(
        &device_b,
        &acct.state.root_scope_id(),
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
        &acct.state.root_scope_id(),
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
    acct.approve_device(&device_b_hex, None).unwrap();
    cfg.account = Some(acct.state.clone());

    let root_1 = causal_encrypted_root(0x21, 300, 1, 1);
    let event_1 = build_drive_root_event(
        &device_b,
        &acct.state.root_scope_id(),
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
        &acct.state.root_scope_id(),
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
