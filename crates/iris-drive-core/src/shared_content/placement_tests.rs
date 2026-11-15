//! Where a shared folder sits on disk. Like Google Drive, it is in My Drive
//! only as a shortcut the person keeps: deleting or moving that shortcut
//! changes their own Drive, never the shared files or their membership.
use super::*;
use crate::{
    profile::Profile,
    sharing::{ShareRecipient, ShareRole, ShareShortcut, create_shared_folder},
};

struct Joined {
    dir: tempfile::TempDir,
    share_id: crate::NostrIdentityId,
}

/// A profile that keeps a folder someone else shares, with one file in it.
async fn joined(role: ShareRole, shortcut: &str) -> Joined {
    let dir = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let member = Profile::create(dir.path(), None).unwrap();
    AppConfig {
        profile: Some(member.state.clone()),
        drives: vec![crate::Drive::primary(member.state.root_scope_id())],
        ..AppConfig::default()
    }
    .save(config_path_in(dir.path()))
    .unwrap();
    std::fs::create_dir(source.path().join("Work")).unwrap();
    std::fs::write(source.path().join("Work/Notes.txt"), "Mine").unwrap();
    let mut daemon = Daemon::open(dir.path()).unwrap();
    daemon.import_source_dir(source.path()).await.unwrap();

    let owner = nostr_sdk::Keys::generate();
    let mut folder = create_shared_folder(
        &owner,
        crate::NostrIdentityId::new_v4(),
        "Team/Trip",
        "Trip",
        None,
        vec![ShareRecipient {
            profile_id: member.state.profile_id,
            app_pubkey: member.state.app_key_pubkey.clone(),
            role,
            label: None,
            representative_npub_hint: None,
            display_name: None,
        }],
        1,
    )
    .unwrap();
    let tree = daemon.tree_handle();
    let (file, size) = tree.put(b"Meet at noon").await.unwrap();
    let empty = tree.put_directory(vec![]).await.unwrap();
    let shared = tree
        .set_entry(&empty, &[], "Plan.txt", &file, size, LinkType::Blob)
        .await
        .unwrap();
    folder.app_key_roots.insert(
        owner.public_key().to_hex(),
        AppKeyRootRef::legacy(shared.to_string(), 1, 1),
    );
    let share_id = folder.share_id;
    let mut config = AppConfig::load_or_default(config_path_in(dir.path())).unwrap();
    config.upsert_shared_folder(folder);
    config.upsert_share_shortcut(ShareShortcut::new(share_id, shortcut, "").unwrap());
    config.save(config_path_in(dir.path())).unwrap();
    Joined { dir, share_id }
}

async fn visible(dir: &Path) -> (Daemon, Cid) {
    let daemon = Daemon::open(dir).unwrap();
    let root = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap()
        .root_cid;
    (daemon, root)
}

async fn has(daemon: &Daemon, root: &Cid, path: &str) -> bool {
    daemon.tree().resolve(root, path).await.unwrap().is_some()
}

fn saved(dir: &Path) -> AppConfig {
    AppConfig::load_or_default(config_path_in(dir)).unwrap()
}

#[tokio::test]
async fn a_joined_folder_is_on_disk_only_where_it_was_added_to_my_drive() {
    let joined = joined(ShareRole::Reader, "Trip").await;
    let (daemon, root) = visible(joined.dir.path()).await;

    assert!(has(&daemon, &root, "Trip/Plan.txt").await);
    assert!(!has(&daemon, &root, "Shared with me").await);

    let mut config = saved(joined.dir.path());
    config.remove_share_shortcuts_for_share(joined.share_id);
    config.save(config_path_in(joined.dir.path())).unwrap();
    let (daemon, root) = visible(joined.dir.path()).await;
    assert!(!has(&daemon, &root, "Trip").await);
    assert!(has(&daemon, &root, "Work/Notes.txt").await);
    assert!(daemon.config().shared_folder(joined.share_id).is_some());
}

#[tokio::test]
async fn deleting_the_shortcut_on_disk_removes_it_from_my_drive_and_nothing_else() {
    // A reader may tidy their own Drive: the shared files are not theirs to delete.
    let joined = joined(ShareRole::Reader, "Trip").await;
    let (mut daemon, root) = visible(joined.dir.path()).await;
    let edited = daemon
        .tree()
        .remove_entry(&root, &[], "Trip")
        .await
        .unwrap();
    daemon
        .import_visible_root_with_tombstone_base(edited, Some(root))
        .await
        .unwrap();

    let config = saved(joined.dir.path());
    assert_eq!(config.share_shortcuts, Vec::new());
    let folder = config.shared_folder(joined.share_id).unwrap();
    let shared = crate::projection::shared_folder_merged_root(daemon.tree(), folder)
        .await
        .unwrap()
        .root_cid;
    assert!(has(&daemon, &shared, "Plan.txt").await);
    let (daemon, root) = visible(joined.dir.path()).await;
    assert!(!has(&daemon, &root, "Trip").await);
    assert!(has(&daemon, &root, "Work/Notes.txt").await);
}

#[tokio::test]
async fn moving_the_shortcut_on_disk_moves_it_in_my_drive() {
    let joined = joined(ShareRole::Reader, "Trip").await;
    let (mut daemon, root) = visible(joined.dir.path()).await;
    let tree = daemon.tree_handle();
    let trip = tree.resolve(&root, "Trip").await.unwrap().unwrap();
    let edited = tree.remove_entry(&root, &[], "Trip").await.unwrap();
    let edited = tree
        .set_entry(&edited, &["Work"], "Holiday", &trip, 0, LinkType::Dir)
        .await
        .unwrap();
    daemon
        .import_visible_root_with_tombstone_base(edited, Some(root))
        .await
        .unwrap();

    let config = saved(joined.dir.path());
    assert_eq!(
        config
            .share_shortcuts
            .iter()
            .map(|shortcut| shortcut.path.as_str())
            .collect::<Vec<_>>(),
        vec!["Work/Holiday"]
    );
    let (daemon, root) = visible(joined.dir.path()).await;
    assert!(has(&daemon, &root, "Work/Holiday/Plan.txt").await);
    assert!(!has(&daemon, &root, "Trip").await);
    // The moved folder is still the shared one, not a private copy.
    let personal = daemon.config().drives[0]
        .app_key_roots
        .values()
        .next()
        .unwrap();
    let personal = Cid::parse(&personal.root_cid).unwrap();
    assert!(!has(&daemon, &personal, "Work/Holiday").await);
}

#[tokio::test]
async fn an_owner_cannot_delete_a_folder_on_disk_while_it_is_shared() {
    let dir = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let owner = Profile::create(dir.path(), None).unwrap();
    AppConfig {
        profile: Some(owner.state.clone()),
        drives: vec![crate::Drive::primary(owner.state.root_scope_id())],
        ..AppConfig::default()
    }
    .save(config_path_in(dir.path()))
    .unwrap();
    std::fs::create_dir_all(source.path().join("Projects/Alpha")).unwrap();
    std::fs::write(source.path().join("Projects/Alpha/Plan.txt"), "Plan").unwrap();
    Daemon::open(dir.path())
        .unwrap()
        .import_source_dir(source.path())
        .await
        .unwrap();
    let created = crate::share_actions::dispatch_share_action_with_content(
        dir.path(),
        crate::ShareAction::CreateShare {
            source_path: "Projects/Alpha".into(),
            display_name: None,
        },
        1,
    )
    .await
    .unwrap();
    // Sharing a folder leaves it where it is: no second copy at the top of My Drive.
    assert_eq!(created.shares[0].shortcut_paths, Vec::<String>::new());
    let (mut daemon, root) = visible(dir.path()).await;
    assert!(has(&daemon, &root, "Projects/Alpha/Plan.txt").await);
    assert!(!has(&daemon, &root, "Alpha").await);

    let edited = daemon
        .tree()
        .remove_entry(&root, &["Projects"], "Alpha")
        .await
        .unwrap();
    let error = daemon
        .import_visible_root_with_tombstone_base(edited, Some(root))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("Stop sharing"), "{error}");
}
