use super::*;
use tempfile::tempdir;

fn recipient_evidence_file(dir: &Path, recipient: &Profile, display_name: &str) -> PathBuf {
    let acceptance_event = iris_drive_core::build_nostr_identity_facet_acceptance_event(
        recipient.app_key.keys(),
        recipient.state.profile_id,
        [iris_drive_core::NostrIdentityKeyPurpose::AppKey],
        recipient
            .state
            .profile_roster_ops
            .first()
            .map(|op| op.op_id.clone()),
        20,
    )
    .unwrap();
    let evidence = iris_drive_core::ShareRecipientProfileEvidence {
        profile_id: recipient.state.profile_id,
        representative_pubkey: Some(recipient.state.app_key_pubkey.clone()),
        representative_npub: None,
        display_name: Some(display_name.to_string()),
        roster_ops: recipient.state.profile_roster_ops.clone(),
        acceptances: vec![
            iris_drive_core::parse_nostr_identity_facet_acceptance_event(&acceptance_event)
                .unwrap(),
        ],
    };
    let path = dir.join("recipient-evidence.json");
    std::fs::write(&path, serde_json::to_vec(&evidence).unwrap()).unwrap();
    path
}

#[test]
fn shares_shortcut_command_adds_unique_my_drive_shortcut() {
    let config_dir = tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
    let folder = iris_drive_core::create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Mac".into()),
        Vec::new(),
        10,
    )
    .unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_shared_folder(folder.clone());
    config.upsert_share_shortcut(
        iris_drive_core::ShareShortcut::new(folder.share_id, "Projects/Alpha", "").unwrap(),
    );
    config.save(config_path_in(config_dir.path())).unwrap();

    let services = super::sharing_test_network::SharingTestServices::new();
    services.prepare(config_dir.path());

    cmd_shares_shortcut(
        config_dir.path(),
        &folder.share_id.to_string(),
        None,
        Some("Projects"),
        "",
    )
    .unwrap();

    let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    let shortcuts = saved
        .share_shortcuts
        .iter()
        .map(|shortcut| shortcut.path.as_str())
        .collect::<Vec<_>>();
    assert_eq!(shortcuts, vec!["Projects/Alpha", "Projects/Alpha (2)"]);
}

#[test]
fn shares_create_command_creates_entity_member_share() {
    let config_dir = tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
    let config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.save(config_path_in(config_dir.path())).unwrap();

    let services = super::sharing_test_network::SharingTestServices::new();
    services.prepare(config_dir.path());

    cmd_shares_create(config_dir.path(), "Projects/Alpha", None).unwrap();

    let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    assert_eq!(saved.shared_folders.len(), 1);
    let folder = &saved.shared_folders[0];
    assert_eq!(folder.source_path, "Projects/Alpha");
    assert_eq!(folder.display_name, "Alpha");
    let members = iris_drive_core::project_share_access(&folder.access).members;
    assert_eq!(
        members
            .get(&account.state.profile_id.to_string())
            .unwrap()
            .role,
        iris_drive_core::ShareRole::Admin
    );
    assert_eq!(
        folder
            .access
            .devices
            .get(&account.state.app_key_pubkey)
            .and_then(|device| device.profile_id),
        Some(account.state.profile_id)
    );
    assert_eq!(saved.share_shortcuts.len(), 1);
    let shortcut = &saved.share_shortcuts[0];
    assert_eq!(shortcut.share_id, folder.share_id);
    assert_eq!(shortcut.path, "Alpha");
    assert_eq!(shortcut.target_path, "");
}

#[test]
fn shares_delete_command_removes_share_and_shortcuts() {
    let config_dir = tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
    let folder = iris_drive_core::create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Mac".into()),
        Vec::new(),
        10,
    )
    .unwrap();
    let share_id = folder.share_id;
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_shared_folder(folder);
    config.upsert_share_shortcut(
        iris_drive_core::ShareShortcut::new(share_id, "Projects/Alpha", "").unwrap(),
    );
    config.save(config_path_in(config_dir.path())).unwrap();

    let services = super::sharing_test_network::SharingTestServices::new();
    services.prepare(config_dir.path());

    cmd_shares_delete(config_dir.path(), &share_id.to_string()).unwrap();

    let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    assert_eq!(saved.shared_folders.len(), 0);
    assert_eq!(saved.share_shortcuts.len(), 0);
}

#[test]
fn shares_invite_command_records_pending_npub_hint_without_authority() {
    let config_dir = tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    let folder = iris_drive_core::create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Mac".into()),
        Vec::new(),
        10,
    )
    .unwrap();
    let share_id = folder.share_id;
    config.upsert_shared_folder(folder);
    config.save(config_path_in(config_dir.path())).unwrap();
    let representative_pubkey = nostr_sdk::Keys::generate().public_key().to_hex();
    let representative_npub = iris_drive_core::app_key_summary::pubkey_npub(&representative_pubkey);

    let services = super::sharing_test_network::SharingTestServices::new();
    services.prepare(config_dir.path());

    cmd_shares_invite(
        config_dir.path(),
        &share_id.to_string(),
        None,
        None,
        None,
        "reader",
        Some(representative_npub.clone()),
        Some("Alice".into()),
        None,
    )
    .unwrap();

    let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    let share = saved.shared_folder(share_id).unwrap();
    assert_eq!(share.pending_invites.len(), 1);
    assert_eq!(
        iris_drive_core::project_share_access(&share.access)
            .members
            .len(),
        1
    );
    assert_eq!(share.access.devices.len(), 1);
    let pending = share.pending_invites.get(&representative_npub).unwrap();
    assert_eq!(pending.display_name.as_deref(), Some("Alice"));
    assert_eq!(pending.status, iris_drive_core::ShareMemberStatus::Pending);
}

#[test]
fn shares_recipient_evidence_command_exports_resolvable_profile_bundle() {
    let config_dir = tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("Phone".into())).unwrap();
    let config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.save(config_path_in(config_dir.path())).unwrap();

    let evidence_json = share_recipient_evidence_json(config_dir.path(), Some("Alice"), 30)
        .expect("recipient evidence export succeeds");
    let evidence: iris_drive_core::ShareRecipientProfileEvidence =
        serde_json::from_str(&evidence_json).unwrap();
    let resolved = iris_drive_core::resolve_share_recipient_from_evidence(&evidence, None)
        .expect("exported evidence resolves");

    assert_eq!(resolved.profile_id, account.state.profile_id);
    assert_eq!(resolved.representative_pubkey, account.state.app_key_pubkey);
    assert_eq!(resolved.display_name.as_deref(), Some("Alice"));
    assert_eq!(resolved.app_pubkeys, vec![account.state.app_key_pubkey]);
}

#[test]
fn share_json_includes_core_projection_keys_and_labels() {
    let config_dir = tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
    let recipient_keys = nostr_sdk::Keys::generate();
    let recipient_profile_id = iris_drive_core::NostrIdentityId::new_v4();
    let folder = iris_drive_core::create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Mac".into()),
        vec![iris_drive_core::ShareRecipient {
            profile_id: recipient_profile_id,
            app_pubkey: recipient_keys.public_key().to_hex(),
            role: iris_drive_core::ShareRole::Reader,
            label: Some("Phone".into()),
            representative_npub_hint: Some("npub1alice".into()),
            display_name: Some("Alice".into()),
        }],
        10,
    )
    .unwrap();

    let view = iris_drive_core::shared_folder_view(&folder, &[], &account.state.app_key_pubkey);
    let share = share_view_json(&view);
    assert_eq!(share["local_role"], "admin");
    assert_eq!(share["local_role_label"], "Admin");
    assert_eq!(share["key_status"], "available");
    assert_eq!(share["key_status_label"], "Available");
    assert_eq!(share["write_authorization"], "authorized");
    assert_eq!(share["write_authorization_label"], "Authorized");
    assert!(share.get("missing_key_wraps").is_none());
    assert!(
        share_view_json_with_diagnostics(&view, true)
            .get("missing_key_wraps")
            .is_some()
    );

    let member = share["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|member| member["profile_id"] == recipient_profile_id.to_string())
        .unwrap();
    assert_eq!(member["role"], "reader");
    assert_eq!(member["role_label"], "Reader");
    assert_eq!(member["status"], "active");
    assert_eq!(member["status_label"], "Active");
}

#[test]
fn shares_members_command_projects_entity_members() {
    let config_dir = tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
    let recipient_keys = nostr_sdk::Keys::generate();
    let recipient_profile_id = iris_drive_core::NostrIdentityId::new_v4();
    let folder = iris_drive_core::create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Mac".into()),
        vec![iris_drive_core::ShareRecipient {
            profile_id: recipient_profile_id,
            app_pubkey: recipient_keys.public_key().to_hex(),
            role: iris_drive_core::ShareRole::Reader,
            label: Some("Phone".into()),
            representative_npub_hint: Some("npub1alice".into()),
            display_name: Some("Alice".into()),
        }],
        10,
    )
    .unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_shared_folder(folder.clone());
    config.save(config_path_in(config_dir.path())).unwrap();

    cmd_shares_members(config_dir.path(), &folder.share_id.to_string()).unwrap();

    let view = iris_drive_core::shared_folder_view(&folder, &[], &account.state.app_key_pubkey);
    assert_eq!(view.members.len(), 2);
    assert!(view.members.iter().any(|member| {
        member.profile_id == recipient_profile_id
            && member.display_name == "Alice"
            && member.role == iris_drive_core::ShareRole::Reader
    }));
}

#[test]
fn shares_revoke_command_revokes_profile_member_and_rotates_epoch() {
    let config_dir = tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
    let recipient_keys = nostr_sdk::Keys::generate();
    let recipient_profile_id = iris_drive_core::NostrIdentityId::new_v4();
    let folder = iris_drive_core::create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Mac".into()),
        vec![iris_drive_core::ShareRecipient {
            profile_id: recipient_profile_id,
            app_pubkey: recipient_keys.public_key().to_hex(),
            role: iris_drive_core::ShareRole::Editor,
            label: Some("Phone".into()),
            representative_npub_hint: None,
            display_name: Some("Alice".into()),
        }],
        10,
    )
    .unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_shared_folder(folder.clone());
    config.save(config_path_in(config_dir.path())).unwrap();

    let services = super::sharing_test_network::SharingTestServices::new();
    services.prepare(config_dir.path());

    cmd_shares_revoke(
        config_dir.path(),
        &folder.share_id.to_string(),
        &recipient_profile_id.to_string(),
        Some("removed"),
        false,
    )
    .unwrap();

    let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    let revoked = saved.shared_folder(folder.share_id).unwrap();
    let projection = revoked.projection();
    let members = iris_drive_core::project_share_access(&revoked.access).members;
    assert_eq!(
        members
            .get(&recipient_profile_id.to_string())
            .unwrap()
            .status,
        iris_drive_core::ShareMemberStatus::Revoked
    );
    assert!(projection.secret_epochs.contains_key(&2));
    assert!(
        projection
            .secret_epochs
            .get(&2)
            .unwrap()
            .wrapped_secrets
            .contains_key(&account.state.app_key_pubkey)
    );
    assert!(
        !projection
            .secret_epochs
            .get(&2)
            .unwrap()
            .wrapped_secrets
            .contains_key(&recipient_keys.public_key().to_hex())
    );
}

#[test]
fn shares_revoke_json_hides_app_keys_without_diagnostics() {
    let config_dir = tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
    let recipient_keys = nostr_sdk::Keys::generate();
    let recipient_profile_id = iris_drive_core::NostrIdentityId::new_v4();
    let folder = iris_drive_core::create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Mac".into()),
        vec![iris_drive_core::ShareRecipient {
            profile_id: recipient_profile_id,
            app_pubkey: recipient_keys.public_key().to_hex(),
            role: iris_drive_core::ShareRole::Editor,
            label: Some("Phone".into()),
            representative_npub_hint: None,
            display_name: Some("Alice".into()),
        }],
        10,
    )
    .unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_shared_folder(folder.clone());
    config.save(config_path_in(config_dir.path())).unwrap();

    let services = super::sharing_test_network::SharingTestServices::new();
    services.prepare(config_dir.path());
    let result = dispatch_cli_share_action(
        config_dir.path(),
        iris_drive_core::ShareAction::RevokeShareMember {
            share_id: folder.share_id,
            profile_id: recipient_profile_id,
            reason: Some("removed".to_string()),
        },
    )
    .unwrap();
    let view = result
        .shares
        .iter()
        .find(|view| view.share_id == folder.share_id)
        .unwrap();

    let normal =
        share_revoke_result_json(&result, view, folder.share_id, recipient_profile_id, false);
    assert_eq!(normal["revoked_app_key_count"], 1);
    assert!(normal.get("revoked_app_keys").is_none());

    let diagnostics =
        share_revoke_result_json(&result, view, folder.share_id, recipient_profile_id, true);
    assert_eq!(diagnostics["revoked_app_keys"].as_array().unwrap().len(), 1);
}

#[test]
fn shares_role_command_updates_profile_member_role() {
    let config_dir = tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
    let recipient_keys = nostr_sdk::Keys::generate();
    let recipient_profile_id = iris_drive_core::NostrIdentityId::new_v4();
    let folder = iris_drive_core::create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Mac".into()),
        vec![iris_drive_core::ShareRecipient {
            profile_id: recipient_profile_id,
            app_pubkey: recipient_keys.public_key().to_hex(),
            role: iris_drive_core::ShareRole::Reader,
            label: Some("Phone".into()),
            representative_npub_hint: None,
            display_name: Some("Alice".into()),
        }],
        10,
    )
    .unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_shared_folder(folder.clone());
    config.save(config_path_in(config_dir.path())).unwrap();

    let services = super::sharing_test_network::SharingTestServices::new();
    services.prepare(config_dir.path());

    cmd_shares_role(
        config_dir.path(),
        &folder.share_id.to_string(),
        &recipient_profile_id.to_string(),
        "editor",
    )
    .unwrap();

    let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    let updated = saved.shared_folder(folder.share_id).unwrap();
    let members = iris_drive_core::project_share_access(&updated.access).members;
    assert_eq!(
        members.get(&recipient_profile_id.to_string()).unwrap().role,
        iris_drive_core::ShareRole::Editor
    );
    assert_eq!(
        iris_drive_core::shared_folder_app_key_write_authorization(
            updated,
            &recipient_keys.public_key().to_hex()
        ),
        iris_drive_core::ShareRootWriteAuthorization::Authorized
    );
}

#[test]
fn shares_invite_and_accept_commands_import_recipient_share() {
    let owner_dir = tempdir().unwrap();
    let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
    let mut owner_config = AppConfig {
        profile: Some(owner.state.clone()),
        ..AppConfig::default()
    };
    let folder = iris_drive_core::create_shared_folder(
        owner.app_key.keys(),
        owner.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Owner".into()),
        Vec::new(),
        10,
    )
    .unwrap();
    owner_config.upsert_shared_folder(folder.clone());
    owner_config.save(config_path_in(owner_dir.path())).unwrap();
    let recipient_dir = tempdir().unwrap();
    let recipient = Profile::create(recipient_dir.path(), Some("Recipient".into())).unwrap();
    let recipient_config = AppConfig {
        profile: Some(recipient.state.clone()),
        ..AppConfig::default()
    };
    recipient_config
        .save(config_path_in(recipient_dir.path()))
        .unwrap();

    let services = super::sharing_test_network::SharingTestServices::new();
    services.prepare(owner_dir.path());

    cmd_shares_invite(
        owner_dir.path(),
        &folder.share_id.to_string(),
        Some(&recipient.state.profile_id.to_string()),
        Some(&recipient.state.app_key_pubkey),
        None,
        "reader",
        Some("npub1alice".into()),
        Some("Alice".into()),
        Some("Recipient phone".into()),
    )
    .unwrap();

    let owner_saved = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let invited = owner_saved.shared_folder(folder.share_id).unwrap();
    let bundle = iris_drive_core::ShareInviteBundle {
        schema: iris_drive_core::SHARE_INVITE_SCHEMA,
        shared_folder: invited.clone(),
        recipient_profile_id: recipient.state.profile_id,
        role: iris_drive_core::ShareRole::Reader,
        representative_npub_hint: Some("npub1alice".into()),
        access_snapshot: None,
        created_at: 20,
    };
    let invite = iris_drive_core::encode_share_invite(&bundle).unwrap();

    let services = super::sharing_test_network::SharingTestServices::new();
    services.prepare(recipient_dir.path());

    cmd_shares_accept(recipient_dir.path(), &invite).unwrap();

    let recipient_saved = AppConfig::load_or_default(config_path_in(recipient_dir.path())).unwrap();
    let accepted = recipient_saved.shared_folder(folder.share_id).unwrap();
    let accepted_members = iris_drive_core::project_share_access(&accepted.access).members;
    assert_eq!(accepted.display_name, "Alpha");
    assert_eq!(
        accepted_members
            .get(&recipient.state.profile_id.to_string())
            .unwrap()
            .display_name
            .as_deref(),
        Some("Alice")
    );
    assert_eq!(
        iris_drive_core::current_shared_folder_key(accepted, recipient.app_key.keys()).unwrap(),
        iris_drive_core::current_shared_folder_key(accepted, owner.app_key.keys()).unwrap()
    );
}

#[test]
fn shares_invite_command_resolves_recipient_evidence_file() {
    let owner_dir = tempdir().unwrap();
    let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
    let mut owner_config = AppConfig {
        profile: Some(owner.state.clone()),
        ..AppConfig::default()
    };
    let folder = iris_drive_core::create_shared_folder(
        owner.app_key.keys(),
        owner.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Owner".into()),
        Vec::new(),
        10,
    )
    .unwrap();
    owner_config.upsert_shared_folder(folder.clone());
    owner_config.save(config_path_in(owner_dir.path())).unwrap();
    let recipient_dir = tempdir().unwrap();
    let recipient = Profile::create(recipient_dir.path(), Some("Recipient".into())).unwrap();
    let evidence_path = recipient_evidence_file(recipient_dir.path(), &recipient, "Alice");

    let services = super::sharing_test_network::SharingTestServices::new();
    services.prepare(owner_dir.path());

    cmd_shares_invite(
        owner_dir.path(),
        &folder.share_id.to_string(),
        None,
        None,
        Some(&evidence_path),
        "editor",
        None,
        None,
        None,
    )
    .unwrap();

    let owner_saved = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let invited = owner_saved.shared_folder(folder.share_id).unwrap();
    let members = iris_drive_core::project_share_access(&invited.access).members;
    let member = members
        .get(&recipient.state.profile_id.to_string())
        .unwrap();
    assert_eq!(member.display_name.as_deref(), Some("Alice"));
    assert_eq!(member.role, iris_drive_core::ShareRole::Editor);
    assert_eq!(
        invited
            .access
            .devices
            .get(&recipient.state.app_key_pubkey)
            .and_then(|device| device.profile_id),
        Some(recipient.state.profile_id)
    );
    assert_eq!(
        iris_drive_core::current_shared_folder_key(invited, recipient.app_key.keys()).unwrap(),
        iris_drive_core::current_shared_folder_key(invited, owner.app_key.keys()).unwrap()
    );
}

#[test]
fn shares_repair_wraps_command_repairs_missing_share_wraps() {
    let config_dir = tempdir().unwrap();
    let account = Profile::create(config_dir.path(), Some("Mac".into())).unwrap();
    let recipient_keys = nostr_sdk::Keys::generate();
    let recipient_pubkey = recipient_keys.public_key().to_hex();
    let recipient_profile_id = iris_drive_core::NostrIdentityId::new_v4();
    let mut folder = iris_drive_core::create_shared_folder(
        account.app_key.keys(),
        account.state.profile_id,
        "Projects/Alpha",
        "Alpha",
        Some("Mac".into()),
        Vec::new(),
        10,
    )
    .unwrap();
    iris_drive_core::invite_shared_folder_member(
        &mut folder,
        account.app_key.keys(),
        iris_drive_core::ShareRecipient {
            profile_id: recipient_profile_id,
            app_pubkey: recipient_pubkey.clone(),
            role: iris_drive_core::ShareRole::Editor,
            label: Some("Phone".into()),
            representative_npub_hint: None,
            display_name: Some("Phone".into()),
        },
        12,
    )
    .unwrap();
    let current_epoch = folder
        .projection()
        .secret_epochs
        .keys()
        .next_back()
        .copied()
        .unwrap();
    folder
        .access
        .key_epochs
        .get_mut(&current_epoch)
        .unwrap()
        .wrapped_secrets
        .remove(&recipient_pubkey);
    assert_eq!(
        iris_drive_core::shared_folder_missing_key_wrap_pubkeys(&folder, current_epoch),
        vec![recipient_pubkey.clone()]
    );
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_shared_folder(folder.clone());
    config.save(config_path_in(config_dir.path())).unwrap();

    let services = super::sharing_test_network::SharingTestServices::new();
    services.prepare(config_dir.path());

    cmd_shares_repair_wraps(config_dir.path(), &folder.share_id.to_string(), false).unwrap();

    let saved = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    let repaired = saved.shared_folder(folder.share_id).unwrap();
    assert_eq!(
        iris_drive_core::shared_folder_missing_key_wrap_pubkeys(repaired, current_epoch).len(),
        0
    );
    assert_eq!(
        iris_drive_core::current_shared_folder_key(repaired, &recipient_keys).unwrap(),
        iris_drive_core::current_shared_folder_key(repaired, account.app_key.keys()).unwrap()
    );
}

#[test]
fn shares_repair_json_hides_app_key_wrap_lists_without_diagnostics() {
    let share_id = iris_drive_core::NostrIdentityId::new_v4();
    let missing_pubkey = nostr_sdk::Keys::generate().public_key().to_hex();
    let repaired_pubkey = nostr_sdk::Keys::generate().public_key().to_hex();
    let result = iris_drive_core::ShareActionResult {
        shares: Vec::new(),
        share_id: Some(share_id),
        profile_id: None,
        role: None,
        epoch: Some(2),
        last_share_invite: None,
        shortcut: None,
        repaired_key_wrap_count: Some(1),
        remaining_missing_key_wrap_count: Some(1),
        revoked_app_pubkeys: Vec::new(),
        repaired_key_wrap_pubkeys: vec![repaired_pubkey],
        remaining_missing_key_wrap_pubkeys: vec![missing_pubkey],
    };

    let normal = share_repair_result_json(&result, share_id, false);
    assert_eq!(normal["repaired_key_wrap_count"], 1);
    assert_eq!(normal["remaining_missing_key_wrap_count"], 1);
    assert!(normal.get("repaired_key_wraps").is_none());
    assert!(normal.get("remaining_missing_key_wraps").is_none());

    let diagnostics = share_repair_result_json(&result, share_id, true);
    assert_eq!(
        diagnostics["repaired_key_wraps"].as_array().unwrap().len(),
        1
    );
    assert_eq!(
        diagnostics["remaining_missing_key_wraps"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
