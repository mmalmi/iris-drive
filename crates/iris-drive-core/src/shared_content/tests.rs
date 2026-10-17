use super::*;
use crate::{
    profile::Profile,
    sharing::{ShareRecipient, ShareRole, create_shared_folder},
};

#[tokio::test]
async fn editor_changes_and_deletions_merge_while_reader_writes_are_rejected() {
    let alice_dir = tempfile::tempdir().unwrap();
    let bob_dir = tempfile::tempdir().unwrap();
    let cara_dir = tempfile::tempdir().unwrap();
    let alice = Profile::create(alice_dir.path(), None).unwrap();
    let bob = Profile::create(bob_dir.path(), None).unwrap();
    let cara = Profile::create(cara_dir.path(), None).unwrap();
    let recipient = |profile: &Profile, role| ShareRecipient {
        profile_id: profile.state.profile_id,
        app_pubkey: profile.state.app_key_pubkey.clone(),
        role,
        label: None,
        representative_npub_hint: None,
        display_name: None,
    };
    let mut folder = create_shared_folder(
        alice.app_key.keys(),
        alice.state.profile_id,
        "Picnic",
        "Picnic",
        None,
        vec![
            recipient(&bob, ShareRole::Editor),
            recipient(&cara, ShareRole::Reader),
        ],
        1,
    )
    .unwrap();
    let mut config = AppConfig {
        profile: Some(alice.state.clone()),
        drives: vec![crate::Drive::primary(alice.state.root_scope_id())],
        ..AppConfig::default()
    };
    config.save(config_path_in(alice_dir.path())).unwrap();
    let daemon = Daemon::open(alice_dir.path()).unwrap();
    let tree = daemon.tree_handle();
    let empty = tree.put_directory(vec![]).await.unwrap();
    let (old, size) = tree.put(b"Original").await.unwrap();
    let initial = tree
        .set_entry(&empty, &[], "Plan.txt", &old, size, LinkType::Blob)
        .await
        .unwrap();
    let initial = tree
        .set_entry(&initial, &[], "Remove.txt", &old, size, LinkType::Blob)
        .await
        .unwrap();
    write_shared_folder(
        &tree,
        &mut folder,
        &alice.state.app_key_pubkey,
        initial,
        &empty,
    )
    .await
    .unwrap();
    let base = crate::projection::shared_folder_merged_root(&tree, &folder)
        .await
        .unwrap()
        .root_cid;
    let (new, size) = tree.put(b"Updated by editor").await.unwrap();
    let edited = tree
        .set_entry(&base, &[], "Plan.txt", &new, size, LinkType::Blob)
        .await
        .unwrap();
    let edited = tree.remove_entry(&edited, &[], "Remove.txt").await.unwrap();
    let before = folder.clone();
    assert!(
        write_shared_folder(
            &tree,
            &mut folder,
            &cara.state.app_key_pubkey,
            edited.clone(),
            &base
        )
        .await
        .unwrap_err()
        .to_string()
        .contains("read-only")
    );
    assert_eq!(folder, before);
    write_shared_folder(&tree, &mut folder, &bob.state.app_key_pubkey, edited, &base)
        .await
        .unwrap();
    let result = crate::projection::shared_folder_merged_root(&tree, &folder)
        .await
        .unwrap()
        .root_cid;
    let file = tree.resolve(&result, "Plan.txt").await.unwrap().unwrap();
    assert_eq!(
        tree.get(&file, None).await.unwrap().unwrap(),
        b"Updated by editor"
    );
    assert!(tree.resolve(&result, "Remove.txt").await.unwrap().is_none());
    let event = crate::nostr_events::build_drive_root_event(
        bob.app_key.keys(),
        &folder.share_id.to_string(),
        PRIMARY_DRIVE_ID,
        &folder.app_key_roots[&bob.state.app_key_pubkey],
        &crate::shared_folder_key_recipient_pubkeys(&folder),
    )
    .unwrap();
    let mut receiver = AppConfig {
        profile: Some(cara.state.clone()),
        shared_folders: vec![before],
        ..AppConfig::default()
    };
    assert_eq!(
        crate::relay_sync::apply_remote_drive_root_event(
            &mut receiver,
            &event,
            Some(cara.app_key.keys())
        )
        .unwrap(),
        crate::relay_sync::DriveRootApply::Applied
    );
    let received = crate::projection::shared_folder_merged_root(&tree, &receiver.shared_folders[0])
        .await
        .unwrap()
        .root_cid;
    let file = tree.resolve(&received, "Plan.txt").await.unwrap().unwrap();
    assert_eq!(
        tree.get(&file, None).await.unwrap().unwrap(),
        b"Updated by editor"
    );
    config.shared_folders = vec![folder.clone()];
    config.save(config_path_in(alice_dir.path())).unwrap();
    crate::share_actions::dispatch_share_action_with_content(
        alice_dir.path(),
        crate::ShareAction::SetShareMemberRole {
            share_id: folder.share_id,
            profile_id: bob.state.profile_id,
            role: ShareRole::Reader,
        },
        30,
    )
    .await
    .unwrap();
    let saved = AppConfig::load_or_default(config_path_in(alice_dir.path())).unwrap();
    let downgraded = &saved.shared_folders[0];
    assert!(!crate::shared_folder_app_key_can_write_roots(
        downgraded,
        &bob.state.app_key_pubkey
    ));
    let result = crate::projection::shared_folder_merged_root(&tree, downgraded)
        .await
        .unwrap()
        .root_cid;
    let file = tree.resolve(&result, "Plan.txt").await.unwrap().unwrap();
    assert_eq!(
        tree.get(&file, None).await.unwrap().unwrap(),
        b"Updated by editor"
    );
    assert!(tree.resolve(&result, "Remove.txt").await.unwrap().is_none());
    crate::share_actions::dispatch_share_action_with_content(
        alice_dir.path(),
        crate::ShareAction::DeleteShare {
            share_id: folder.share_id,
        },
        40,
    )
    .await
    .unwrap();
    let saved = AppConfig::load_or_default(config_path_in(alice_dir.path())).unwrap();
    assert!(saved.shared_folders.is_empty());
    let personal = crate::primary_merged_root(&tree, &saved).await.unwrap();
    let file = tree
        .resolve(&personal.root_cid, "Picnic/Plan.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        tree.get(&file, None).await.unwrap().unwrap(),
        b"Updated by editor"
    );
    assert!(
        tree.resolve(&personal.root_cid, "Picnic/Remove.txt")
            .await
            .unwrap()
            .is_none()
    );
}

#[test]
fn membership_updates_advance_root_announcement_even_without_file_changes() {
    let keys = nostr_sdk::Keys::generate();
    let mut folder = create_shared_folder(
        &keys,
        crate::NostrIdentityId::new_v4(),
        "Photos",
        "Photos",
        None,
        vec![],
        1,
    )
    .unwrap();
    let root = AppKeyRootRef::legacy(
        hashtree_core::Cid::encrypted([1; 32], [2; 32]).to_string(),
        1,
        1,
    );
    let first = crate::nostr_events::build_drive_root_publish_event(
        &keys,
        &folder.share_id.to_string(),
        PRIMARY_DRIVE_ID,
        &share_root_for_publish(&folder, &root),
        &[keys.public_key().to_hex()],
    )
    .unwrap();
    folder.access.updated_at = first.created_at.as_secs() as i64 + 1;
    let second = crate::nostr_events::build_drive_root_publish_event(
        &keys,
        &folder.share_id.to_string(),
        PRIMARY_DRIVE_ID,
        &share_root_for_publish(&folder, &root),
        &[keys.public_key().to_hex()],
    )
    .unwrap();
    assert!(second.created_at > first.created_at);
}

#[tokio::test]
async fn reimport_updates_owned_share_without_exposing_private_siblings() {
    let dir = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let owner = Profile::create(dir.path(), None).unwrap();
    let config = AppConfig {
        profile: Some(owner.state.clone()),
        drives: vec![crate::Drive::primary(owner.state.root_scope_id())],
        ..AppConfig::default()
    };
    config.save(config_path_in(dir.path())).unwrap();
    std::fs::create_dir(source.path().join("Picnic")).unwrap();
    std::fs::write(source.path().join("Picnic/Plan.txt"), "First plan").unwrap();
    std::fs::write(source.path().join("Private.txt"), "Private").unwrap();
    Daemon::open(dir.path())
        .unwrap()
        .import_source_dir(source.path())
        .await
        .unwrap();
    crate::share_actions::dispatch_share_action_with_content(
        dir.path(),
        crate::ShareAction::CreateShare {
            source_path: "Picnic".into(),
            display_name: None,
        },
        10,
    )
    .await
    .unwrap();
    prepare_shared_folder_roots(dir.path()).await.unwrap();
    std::fs::write(source.path().join("Picnic/Plan.txt"), "Updated plan").unwrap();
    let mut daemon = Daemon::open(dir.path()).unwrap();
    daemon.import_source_dir(source.path()).await.unwrap();
    let saved = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    let shared =
        crate::projection::shared_folder_merged_root(daemon.tree(), &saved.shared_folders[0])
            .await
            .unwrap();
    let file = daemon
        .tree()
        .resolve(&shared.root_cid, "Plan.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        daemon.tree().get(&file, None).await.unwrap().unwrap(),
        b"Updated plan"
    );
    assert!(
        daemon
            .tree()
            .resolve(&shared.root_cid, "Private.txt")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn accepting_a_share_keeps_a_same_named_personal_folder() {
    let dir = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let reader = Profile::create(dir.path(), None).unwrap();
    AppConfig {
        profile: Some(reader.state.clone()),
        drives: vec![crate::Drive::primary(reader.state.root_scope_id())],
        ..AppConfig::default()
    }
    .save(config_path_in(dir.path()))
    .unwrap();
    std::fs::create_dir(source.path().join("Picnic")).unwrap();
    std::fs::write(source.path().join("Picnic/Private.txt"), "Keep private").unwrap();
    Daemon::open(dir.path())
        .unwrap()
        .import_source_dir(source.path())
        .await
        .unwrap();
    let owner = nostr_sdk::Keys::generate();
    let mut folder = create_shared_folder(
        &owner,
        crate::NostrIdentityId::new_v4(),
        "Team",
        "Picnic",
        None,
        vec![],
        1,
    )
    .unwrap();
    let invite = crate::invite_shared_folder_member(
        &mut folder,
        &owner,
        ShareRecipient {
            profile_id: reader.state.profile_id,
            app_pubkey: reader.state.app_key_pubkey.clone(),
            role: ShareRole::Reader,
            label: None,
            representative_npub_hint: None,
            display_name: None,
        },
        2,
    )
    .unwrap();
    let result = crate::share_actions::dispatch_share_action_with_content(
        dir.path(),
        crate::ShareAction::AcceptShareInvite {
            invite: invite.invite_url,
        },
        3,
    )
    .await
    .unwrap();
    assert_eq!(result.shares[0].shortcut_paths, vec!["Picnic (shared)"]);
    let daemon = Daemon::open(dir.path()).unwrap();
    let root = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap();
    let private = daemon
        .tree()
        .resolve(&root.root_cid, "Picnic/Private.txt")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        daemon.tree().get(&private, None).await.unwrap().unwrap(),
        b"Keep private"
    );
}
