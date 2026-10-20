use super::*;
use crate::paths::config_path_in;
use crate::profile::Profile;
use crate::sharing::{current_shared_folder_key, shared_folder_missing_key_wrap_pubkeys};
use crate::{AppConfig, SHARE_INVITE_PREFIX};
use tempfile::tempdir;

fn init_config(dir: &Path, profile: &Profile) {
    let config = AppConfig {
        profile: Some(profile.state.clone()),
        ..AppConfig::default()
    };
    config.save(config_path_in(dir)).unwrap();
}

#[test]
fn create_share_defaults_display_name_to_source_dirname() {
    let owner_dir = tempdir().unwrap();
    let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
    init_config(owner_dir.path(), &owner);

    let created = dispatch_share_action(
        owner_dir.path(),
        ShareAction::CreateShare {
            source_path: "Shared/tst".to_owned(),
            display_name: None,
        },
        10,
    )
    .unwrap();

    assert_eq!(created.shares.len(), 1);
    assert_eq!(created.shares[0].display_name, "tst");
    assert_eq!(created.shares[0].shortcut_paths, vec!["tst"]);

    let saved = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    assert_eq!(saved.shared_folders.len(), 1);
    assert_eq!(saved.shared_folders[0].display_name, "tst");
    assert_eq!(saved.share_shortcuts.len(), 1);
    assert_eq!(saved.share_shortcuts[0].path, "tst");
}

#[test]
fn accept_share_invite_adds_share_to_my_drive() {
    let owner_dir = tempdir().unwrap();
    let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
    init_config(owner_dir.path(), &owner);
    let recipient_dir = tempdir().unwrap();
    let recipient = Profile::create(recipient_dir.path(), Some("Recipient".into())).unwrap();
    init_config(recipient_dir.path(), &recipient);

    let created = dispatch_share_action(
        owner_dir.path(),
        ShareAction::CreateShare {
            source_path: "Projects/Alpha".to_owned(),
            display_name: Some("Alpha".to_owned()),
        },
        10,
    )
    .unwrap();
    let share_id = created.share_id.unwrap();
    let invited = dispatch_share_action(
        owner_dir.path(),
        ShareAction::InviteShareMember {
            share_id,
            profile_id: recipient.state.profile_id,
            app_key: recipient.app_key.pubkey_hex(),
            role: ShareRole::Reader,
            representative_npub_hint: None,
            display_name: Some("Recipient".to_owned()),
            label: Some("Laptop".to_owned()),
        },
        20,
    )
    .unwrap();

    let invite = invited.last_share_invite.unwrap();
    let mut bundle = crate::parse_share_invite(&invite).unwrap();
    bundle.shared_folder.app_key_roots.insert(
        owner.state.app_key_pubkey.clone(),
        crate::AppKeyRootRef::legacy(
            hashtree_core::Cid::encrypted([1; 32], [2; 32]).to_string(),
            21,
            1,
        ),
    );
    let invite = crate::encode_share_invite(&bundle).unwrap();
    let accepted = dispatch_share_action(
        recipient_dir.path(),
        ShareAction::AcceptShareInvite {
            invite: invite.clone(),
        },
        30,
    )
    .unwrap();

    assert_eq!(accepted.shares.len(), 1);
    assert_eq!(accepted.shares[0].display_name, "Alpha");
    assert_eq!(accepted.shares[0].shortcut_paths, vec!["Alpha"]);
    let mut saved = AppConfig::load_or_default(config_path_in(recipient_dir.path())).unwrap();
    assert!(
        saved.shared_folders[0].app_key_roots.is_empty(),
        "unsigned invite roots are not trusted"
    );
    crate::revoke_shared_folder_member(
        &mut saved.shared_folders[0],
        owner.app_key.keys(),
        recipient.state.profile_id,
        None,
        100,
    )
    .unwrap();
    let latest_access = saved.shared_folders[0].access.clone();
    saved.save(config_path_in(recipient_dir.path())).unwrap();
    dispatch_share_action(
        recipient_dir.path(),
        ShareAction::AcceptShareInvite { invite },
        101,
    )
    .unwrap();
    let saved = AppConfig::load_or_default(config_path_in(recipient_dir.path())).unwrap();
    assert_eq!(
        saved.shared_folders[0].access, latest_access,
        "old invite cannot restore access"
    );
    assert_eq!(saved.share_shortcuts.len(), 1);
    assert_eq!(saved.share_shortcuts[0].path, "Alpha");
}

#[test]
fn share_state_repairs_legacy_share_without_my_drive_shortcut() {
    let owner_dir = tempdir().unwrap();
    let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
    init_config(owner_dir.path(), &owner);

    dispatch_share_action(
        owner_dir.path(),
        ShareAction::CreateShare {
            source_path: "Shared/Legacy".to_owned(),
            display_name: Some("Legacy".to_owned()),
        },
        10,
    )
    .unwrap();
    let mut saved = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    saved.share_shortcuts.clear();
    saved.save(config_path_in(owner_dir.path())).unwrap();

    let repaired = share_action_state(owner_dir.path()).unwrap();

    assert_eq!(repaired.shares.len(), 1);
    assert_eq!(repaired.shares[0].shortcut_paths, vec!["Legacy"]);
    let saved = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    assert_eq!(saved.share_shortcuts.len(), 1);
    assert_eq!(saved.share_shortcuts[0].path, "Legacy");
}

#[test]
fn delete_share_removes_share_and_shortcuts_from_config_and_result() {
    let owner_dir = tempdir().unwrap();
    let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
    init_config(owner_dir.path(), &owner);

    let created = dispatch_share_action(
        owner_dir.path(),
        ShareAction::CreateShare {
            source_path: "Shared/tst".to_owned(),
            display_name: None,
        },
        10,
    )
    .unwrap();
    let share_id = created.share_id.unwrap();

    let mut config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    assert!(
        config.upsert_share_shortcut(ShareShortcut::new(share_id, "best", "").expect("shortcut"))
    );
    config.save(config_path_in(owner_dir.path())).unwrap();

    let deleted =
        dispatch_share_action(owner_dir.path(), ShareAction::DeleteShare { share_id }, 20).unwrap();

    assert_eq!(deleted.shares.len(), 0);
    let saved = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    assert_eq!(saved.shared_folders.len(), 0);
    assert_eq!(saved.share_shortcuts.len(), 0);
}

#[test]
fn share_action_result_carries_cli_parity_metadata() {
    let owner_dir = tempdir().unwrap();
    let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
    init_config(owner_dir.path(), &owner);
    let recipient = nostr_sdk::Keys::generate();
    let recipient_profile_id = NostrIdentityId::new_v4();
    let recipient_pubkey = recipient.public_key().to_hex();

    let created = dispatch_share_action(
        owner_dir.path(),
        ShareAction::CreateShare {
            source_path: "Projects/Alpha".to_owned(),
            display_name: Some("Alpha".to_owned()),
        },
        10,
    )
    .unwrap();
    let share_id = created.share_id.unwrap();

    let invited = dispatch_share_action(
        owner_dir.path(),
        ShareAction::InviteShareMember {
            share_id,
            profile_id: recipient_profile_id,
            app_key: recipient_pubkey.clone(),
            role: ShareRole::Editor,
            representative_npub_hint: None,
            display_name: Some("Alice".to_owned()),
            label: Some("Phone".to_owned()),
        },
        20,
    )
    .unwrap();

    assert_eq!(invited.share_id, Some(share_id));
    assert_eq!(invited.profile_id, Some(recipient_profile_id));
    assert!(
        invited
            .last_share_invite
            .unwrap()
            .starts_with(SHARE_INVITE_PREFIX)
    );
    assert_eq!(invited.epoch, Some(2));
    assert_eq!(invited.revoked_app_pubkeys.len(), 0);

    let role_changed = dispatch_share_action(
        owner_dir.path(),
        ShareAction::SetShareMemberRole {
            share_id,
            profile_id: recipient_profile_id,
            role: ShareRole::Reader,
        },
        25,
    )
    .unwrap();

    assert_eq!(role_changed.share_id, Some(share_id));
    assert_eq!(role_changed.profile_id, Some(recipient_profile_id));
    assert_eq!(role_changed.role, Some(ShareRole::Reader));
    assert_eq!(role_changed.epoch, None);
    assert!(
        role_changed
            .shares
            .iter()
            .find(|share| share.share_id == share_id)
            .unwrap()
            .members
            .iter()
            .any(|member| {
                member.profile_id == recipient_profile_id && member.role == ShareRole::Reader
            })
    );

    let revoked = dispatch_share_action(
        owner_dir.path(),
        ShareAction::RevokeShareMember {
            share_id,
            profile_id: recipient_profile_id,
            reason: Some("removed".to_owned()),
        },
        30,
    )
    .unwrap();

    assert_eq!(revoked.share_id, Some(share_id));
    assert_eq!(revoked.profile_id, Some(recipient_profile_id));
    assert_eq!(revoked.epoch, Some(3));
    assert_eq!(revoked.revoked_app_pubkeys, vec![recipient_pubkey]);
}

#[test]
fn pending_share_invite_action_records_contact_hint_without_authority() {
    let owner_dir = tempdir().unwrap();
    let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
    init_config(owner_dir.path(), &owner);
    let representative_pubkey = nostr_sdk::Keys::generate().public_key().to_hex();
    let representative_npub = crate::app_key_summary::pubkey_npub(&representative_pubkey);

    let created = dispatch_share_action(
        owner_dir.path(),
        ShareAction::CreateShare {
            source_path: "Projects/Alpha".to_owned(),
            display_name: Some("Alpha".to_owned()),
        },
        10,
    )
    .unwrap();
    let share_id = created.share_id.unwrap();
    let owner_member_count = created.shares[0].members.len();
    let owner_participant_count = created.shares[0].participant_count;
    let owner_epoch = created.shares[0].current_key_epoch;

    let pending = dispatch_share_action(
        owner_dir.path(),
        ShareAction::RecordPendingShareInvite {
            share_id,
            representative_npub_hint: representative_npub.clone(),
            role: ShareRole::Reader,
            display_name: Some("Alice".to_owned()),
        },
        20,
    )
    .unwrap();

    let view = pending
        .shares
        .iter()
        .find(|share| share.share_id == share_id)
        .unwrap();
    assert_eq!(view.members.len(), owner_member_count);
    assert_eq!(view.participant_count, owner_participant_count);
    assert_eq!(view.current_key_epoch, owner_epoch);
    assert_eq!(view.pending_invites.len(), 1);
    assert_eq!(
        view.pending_invites[0].representative_npub_hint,
        representative_npub
    );
    assert_eq!(view.pending_invites[0].display_name, "Alice");
    assert_eq!(
        view.pending_invites[0].status,
        crate::sharing::ShareMemberStatus::Pending
    );

    let recipient_keys = nostr_sdk::Keys::generate();
    let recipient_profile_id = NostrIdentityId::new_v4();
    let invited = dispatch_share_action(
        owner_dir.path(),
        ShareAction::InviteShareMember {
            share_id,
            profile_id: recipient_profile_id,
            app_key: recipient_keys.public_key().to_hex(),
            role: ShareRole::Reader,
            representative_npub_hint: Some(representative_npub),
            display_name: Some("Alice".to_owned()),
            label: Some("Phone".to_owned()),
        },
        30,
    )
    .unwrap();
    let view = invited
        .shares
        .iter()
        .find(|share| share.share_id == share_id)
        .unwrap();
    assert_eq!(view.pending_invites.len(), 0);
    assert!(
        view.members
            .iter()
            .any(|member| member.profile_id == recipient_profile_id
                && member.status == crate::sharing::ShareMemberStatus::Active)
    );
}

#[test]
fn repair_share_action_reports_repaired_and_remaining_wrap_pubkeys() {
    let owner_dir = tempdir().unwrap();
    let owner = Profile::create(owner_dir.path(), Some("Owner".into())).unwrap();
    init_config(owner_dir.path(), &owner);
    let recipient = nostr_sdk::Keys::generate();
    let recipient_profile_id = NostrIdentityId::new_v4();
    let recipient_pubkey = recipient.public_key().to_hex();

    let share_id = dispatch_share_action(
        owner_dir.path(),
        ShareAction::CreateShare {
            source_path: "Projects/Alpha".to_owned(),
            display_name: Some("Alpha".to_owned()),
        },
        10,
    )
    .unwrap()
    .share_id
    .unwrap();
    dispatch_share_action(
        owner_dir.path(),
        ShareAction::InviteShareMember {
            share_id,
            profile_id: recipient_profile_id,
            app_key: recipient_pubkey.clone(),
            role: ShareRole::Editor,
            representative_npub_hint: None,
            display_name: Some("Alice".to_owned()),
            label: Some("Phone".to_owned()),
        },
        20,
    )
    .unwrap();

    let mut config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let folder = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == share_id)
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
        shared_folder_missing_key_wrap_pubkeys(folder, current_epoch),
        vec![recipient_pubkey.clone()]
    );
    config.save(config_path_in(owner_dir.path())).unwrap();

    let repaired = dispatch_share_action(
        owner_dir.path(),
        ShareAction::RepairShareWraps { share_id },
        30,
    )
    .unwrap();

    assert_eq!(repaired.share_id, Some(share_id));
    assert_eq!(repaired.epoch, Some(current_epoch));
    assert_eq!(repaired.repaired_key_wrap_count, Some(1));
    assert_eq!(repaired.repaired_key_wrap_pubkeys, vec![recipient_pubkey]);
    assert_eq!(repaired.remaining_missing_key_wrap_pubkeys.len(), 0);
    let saved = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let folder = saved.shared_folder(share_id).unwrap();
    assert_eq!(
        current_shared_folder_key(folder, &recipient).unwrap(),
        current_shared_folder_key(folder, owner.app_key.keys()).unwrap()
    );
}
