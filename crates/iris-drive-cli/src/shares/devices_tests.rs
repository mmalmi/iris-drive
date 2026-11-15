//! A profile's shared folders are the same on all of its devices, end to end
//! over a real relay and Blossom server: what one device shares, joins, moves
//! or leaves, the profile's other devices share, join, move or leave too.
use super::*;
use iris_drive_core::{ShareAction, ShareRole, SharedFolderView};
use std::time::Duration;
use tempfile::tempdir;

fn new_profile(dir: &Path, label: &str) {
    let profile = Profile::create(dir, Some(label.into())).unwrap();
    AppConfig {
        profile: Some(profile.state),
        ..AppConfig::default()
    }
    .save(config_path_in(dir))
    .unwrap();
}

/// Link a new device in `dir` to the profile in `first_dir`, approved there.
fn link_device(first_dir: &Path, dir: &Path, label: &str) {
    let mut config = AppConfig::load_or_default(config_path_in(first_dir)).unwrap();
    let mut first = Profile::load(config.profile.clone().unwrap(), first_dir).unwrap();
    let mut linked = Profile::link_to_profile(
        dir,
        first.state.profile_id,
        first.state.app_key_pubkey.clone(),
        Some(label.into()),
    )
    .unwrap();
    first
        .approve_app_key(&linked.state.app_key_pubkey, Some(label.into()))
        .unwrap();
    linked
        .state
        .profile_roster_ops
        .clone_from(&first.state.profile_roster_ops);
    let linked = Profile::load(linked.state, dir).unwrap();
    assert!(linked.state.is_authorized());
    config.profile = Some(first.state);
    config.save(config_path_in(first_dir)).unwrap();
    AppConfig {
        profile: Some(linked.state),
        ..AppConfig::default()
    }
    .save(config_path_in(dir))
    .unwrap();
}

fn block_on<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Runtime::new().unwrap().block_on(future)
}

fn sync(dir: &Path) {
    block_on(iris_drive_core::network_sync::sync_once(
        dir,
        &[],
        Duration::from_secs(5),
    ))
    .unwrap();
}

/// What a running Drive does after a local change: publish its current state.
fn publish(dir: &Path) {
    block_on(async {
        let config = AppConfig::load_or_default(config_path_in(dir)).unwrap();
        let state = config.profile.clone().unwrap();
        let client = iris_drive_core::relay_sync::connect(&config.relays)
            .await
            .unwrap();
        publish_current_state(&client, dir, &config, &state, true)
            .await
            .unwrap();
        iris_drive_core::relay_sync::shutdown_client(&client).await;
    });
}

fn read_in_drive(dir: &Path, path: &str) -> Option<String> {
    block_on(async {
        let config = AppConfig::load_or_default(config_path_in(dir)).unwrap();
        let daemon = iris_drive_core::Daemon::open(dir).unwrap();
        let root = iris_drive_core::primary_merged_root(daemon.tree(), &config)
            .await
            .unwrap();
        let cid = daemon.tree().resolve(&root.root_cid, path).await.unwrap()?;
        let bytes = daemon.tree().get(&cid, None).await.unwrap()?;
        Some(String::from_utf8(bytes).unwrap())
    })
}

/// Save a file the way a mounted Drive does: an edit of the visible folder.
fn write_in_drive(dir: &Path, path: &str, content: &str) {
    block_on(async {
        let config = AppConfig::load_or_default(config_path_in(dir)).unwrap();
        let mut daemon = iris_drive_core::Daemon::open(dir).unwrap();
        let base = iris_drive_core::primary_merged_root(daemon.tree(), &config)
            .await
            .unwrap()
            .root_cid;
        let (cid, size) = daemon.tree().put(content.as_bytes()).await.unwrap();
        let parts: Vec<&str> = path.split('/').collect();
        let (name, parent) = parts.split_last().unwrap();
        let edited = daemon
            .tree()
            .set_entry(
                &base,
                parent,
                name,
                &cid,
                size,
                hashtree_core::LinkType::Blob,
            )
            .await
            .unwrap();
        daemon
            .import_visible_root_with_tombstone_base(edited, Some(base))
            .await
            .unwrap();
    });
}

fn shares_of(dir: &Path) -> Vec<SharedFolderView> {
    iris_drive_core::share_action_state(dir).unwrap().shares
}

fn act(dir: &Path, action: ShareAction) -> iris_drive_core::ShareActionResult {
    dispatch_cli_share_action(dir, action).unwrap()
}

fn join_link(dir: &Path, share_id: iris_drive_core::NostrIdentityId, role: ShareRole) -> String {
    act(dir, ShareAction::CreateShareJoinLink { share_id, role })
        .share_join_link
        .unwrap()
}

fn only_share_id(dir: &Path) -> iris_drive_core::NostrIdentityId {
    let shares = shares_of(dir);
    assert_eq!(shares.len(), 1);
    shares[0].share_id
}

#[test]
fn a_folder_shared_on_one_device_is_shared_on_the_profiles_other_devices() {
    let services = super::sharing_test_network::SharingTestServices::new();
    let laptop = tempdir().unwrap();
    new_profile(laptop.path(), "Laptop");
    services.prepare(laptop.path());
    cmd_shares_create(laptop.path(), "Projects/Alpha", None).unwrap();
    let share_id = only_share_id(laptop.path());

    // A phone linked afterwards is let into the folder by the laptop.
    let phone = tempdir().unwrap();
    link_device(laptop.path(), phone.path(), "Phone");
    services.prepare(phone.path());
    sync(laptop.path());
    sync(phone.path());

    let shares = shares_of(phone.path());
    assert_eq!(shares.len(), 1);
    assert_eq!(shares[0].share_id, share_id);
    assert_eq!(shares[0].source_path, "Projects/Alpha");
    assert!(shares[0].can_admin && shares[0].can_write);
    assert_eq!(
        read_in_drive(phone.path(), "Projects/Alpha/Plan.txt").as_deref(),
        Some("Shared plan")
    );

    // A guest joins by the laptop's link.
    let guest = tempdir().unwrap();
    new_profile(guest.path(), "Guest");
    services.prepare(guest.path());
    let link = join_link(laptop.path(), share_id, ShareRole::Editor);
    act(
        guest.path(),
        ShareAction::AcceptShareInvite { invite: link },
    );
    sync(laptop.path());
    sync(guest.path());
    assert_eq!(
        read_in_drive(guest.path(), "Alpha/Plan.txt").as_deref(),
        Some("Shared plan")
    );

    // What the phone saves inside the folder reaches the guest.
    sync(phone.path());
    write_in_drive(phone.path(), "Projects/Alpha/Notes.txt", "From the phone");
    publish(phone.path());
    sync(guest.path());
    assert_eq!(
        read_in_drive(guest.path(), "Alpha/Notes.txt").as_deref(),
        Some("From the phone")
    );

    // The laptop stops sharing: the phone stops sharing too and keeps the folder.
    act(laptop.path(), ShareAction::DeleteShare { share_id });
    sync(laptop.path());
    sync(phone.path());
    assert_eq!(shares_of(phone.path()), Vec::new());
    assert_eq!(
        read_in_drive(phone.path(), "Projects/Alpha/Plan.txt").as_deref(),
        Some("Shared plan")
    );
}

#[test]
fn a_folder_joined_on_one_device_appears_on_the_profiles_other_devices() {
    let services = super::sharing_test_network::SharingTestServices::new();
    let owner = tempdir().unwrap();
    new_profile(owner.path(), "Owner");
    services.prepare(owner.path());
    cmd_shares_create(owner.path(), "Projects/Alpha", None).unwrap();
    let share_id = only_share_id(owner.path());
    let link = join_link(owner.path(), share_id, ShareRole::Editor);

    let laptop = tempdir().unwrap();
    new_profile(laptop.path(), "Laptop");
    let phone = tempdir().unwrap();
    link_device(laptop.path(), phone.path(), "Phone");
    services.prepare(laptop.path());
    services.prepare(phone.path());

    // The laptop joins; the phone never saw the link.
    act(
        laptop.path(),
        ShareAction::AcceptShareInvite { invite: link },
    );
    sync(owner.path());
    sync(laptop.path());
    assert_eq!(shares_of(laptop.path())[0].shortcut_paths, ["Alpha"]);
    // The owner let in the device that asked. The phone learns of the folder
    // from the laptop, asks with the same link, and is let in too.
    sync(phone.path());
    assert_eq!(shares_of(phone.path()), Vec::new());
    let waiting = iris_drive_core::share_action_state(phone.path())
        .unwrap()
        .pending_joins;
    assert_eq!(waiting.len(), 1);
    assert_eq!(waiting[0].share_id, share_id);
    sync(owner.path());
    sync(phone.path());

    let shares = shares_of(phone.path());
    assert_eq!(shares.len(), 1);
    assert_eq!(shares[0].share_id, share_id);
    assert_eq!(shares[0].local_role, ShareRole::Editor);
    assert_eq!(shares[0].shortcut_paths, ["Alpha"]);
    assert_eq!(
        read_in_drive(phone.path(), "Alpha/Plan.txt").as_deref(),
        Some("Shared plan")
    );

    assert_eq!(
        iris_drive_core::share_action_state(phone.path())
            .unwrap()
            .pending_joins,
        Vec::new()
    );

    // What the phone saves inside reaches the owner.
    write_in_drive(phone.path(), "Alpha/Notes.txt", "From the phone");
    publish(phone.path());
    sync(owner.path());
    assert_eq!(
        read_in_drive(owner.path(), "Projects/Alpha/Notes.txt").as_deref(),
        Some("From the phone")
    );

    // "Remove from My Drive" on the laptop takes it out of the phone's too.
    act(
        laptop.path(),
        ShareAction::RemoveShareShortcuts { share_id },
    );
    sync(laptop.path());
    sync(phone.path());
    let shares = shares_of(phone.path());
    assert_eq!(shares.len(), 1);
    assert_eq!(shares[0].shortcut_paths, Vec::<String>::new());
    assert_eq!(read_in_drive(phone.path(), "Alpha/Plan.txt"), None);

    // Added back somewhere else, it sits in the same place on the phone.
    act(
        laptop.path(),
        ShareAction::AddShareShortcut {
            share_id,
            path: Some("Work/Alpha".into()),
            parent: None,
            target_path: None,
        },
    );
    sync(laptop.path());
    sync(phone.path());
    assert_eq!(shares_of(phone.path())[0].shortcut_paths, ["Work/Alpha"]);
    assert_eq!(
        read_in_drive(phone.path(), "Work/Alpha/Plan.txt").as_deref(),
        Some("Shared plan")
    );

    // The laptop leaves: the phone leaves too.
    act(laptop.path(), ShareAction::DeleteShare { share_id });
    sync(laptop.path());
    sync(phone.path());
    assert_eq!(shares_of(phone.path()), Vec::new());
    assert_eq!(read_in_drive(phone.path(), "Work/Alpha/Plan.txt"), None);
}

#[test]
fn a_device_linked_later_asks_to_join_with_the_profiles_link() {
    let services = super::sharing_test_network::SharingTestServices::new();
    let owner = tempdir().unwrap();
    new_profile(owner.path(), "Owner");
    services.prepare(owner.path());
    cmd_shares_create(owner.path(), "Projects/Alpha", None).unwrap();
    let share_id = only_share_id(owner.path());
    let link = join_link(owner.path(), share_id, ShareRole::Reader);

    let laptop = tempdir().unwrap();
    new_profile(laptop.path(), "Laptop");
    services.prepare(laptop.path());
    act(
        laptop.path(),
        ShareAction::AcceptShareInvite { invite: link },
    );
    sync(owner.path());
    sync(laptop.path());
    assert_eq!(only_share_id(laptop.path()), share_id);

    // The owner let in only the laptop: the phone did not exist yet.
    let phone = tempdir().unwrap();
    link_device(laptop.path(), phone.path(), "Phone");
    services.prepare(phone.path());
    sync(laptop.path());
    sync(phone.path());
    assert_eq!(shares_of(phone.path()), Vec::new());

    // The phone asked with the profile's link; the owner's Drive lets it in.
    sync(owner.path());
    sync(phone.path());
    let shares = shares_of(phone.path());
    assert_eq!(shares.len(), 1);
    assert_eq!(shares[0].share_id, share_id);
    assert_eq!(shares[0].local_role, ShareRole::Reader);
    assert_eq!(shares[0].shortcut_paths, ["Alpha"]);
    assert_eq!(
        read_in_drive(phone.path(), "Alpha/Plan.txt").as_deref(),
        Some("Shared plan")
    );
}
