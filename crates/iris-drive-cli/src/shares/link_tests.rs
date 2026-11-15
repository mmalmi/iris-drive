//! Sharing by link, end to end over a real relay and Blossom server: the
//! owner hands out a drive.iris.to link, a guest who is not a member opens
//! it, the owner's Drive lets them in, and the guest reads the folder.
use super::*;
use std::time::Duration;
use tempfile::tempdir;

fn new_profile(dir: &Path, label: &str) -> Profile {
    let profile = Profile::create(dir, Some(label.into())).unwrap();
    AppConfig {
        profile: Some(profile.state.clone()),
        ..AppConfig::default()
    }
    .save(config_path_in(dir))
    .unwrap();
    profile
}

fn sync(dir: &Path) {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(iris_drive_core::network_sync::sync_once(
            dir,
            &[],
            Duration::from_secs(5),
        ))
        .unwrap();
}

fn read_in_drive(dir: &Path, path: &str) -> Option<String> {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
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

#[test]
fn a_guest_joins_by_link_and_reads_the_shared_file() {
    let services = super::sharing_test_network::SharingTestServices::new();
    let owner_dir = tempdir().unwrap();
    new_profile(owner_dir.path(), "Owner");
    services.prepare(owner_dir.path());
    cmd_shares_create(owner_dir.path(), "Projects/Alpha", None).unwrap();
    let share_id = AppConfig::load_or_default(config_path_in(owner_dir.path()))
        .unwrap()
        .shared_folders[0]
        .share_id;

    let link_action = || iris_drive_core::ShareAction::CreateShareJoinLink {
        share_id,
        role: iris_drive_core::ShareRole::Reader,
    };
    let link = dispatch_cli_share_action(owner_dir.path(), link_action())
        .unwrap()
        .share_join_link
        .unwrap();
    assert!(link.starts_with("https://drive.iris.to/#/share-join/"));
    let again = dispatch_cli_share_action(owner_dir.path(), link_action()).unwrap();
    assert_eq!(again.share_join_link.as_deref(), Some(link.as_str()));

    let guest_dir = tempdir().unwrap();
    let guest = new_profile(guest_dir.path(), "Guest");
    services.prepare(guest_dir.path());
    let asked = dispatch_cli_share_action(
        guest_dir.path(),
        iris_drive_core::ShareAction::AcceptShareInvite { invite: link },
    )
    .unwrap();
    assert_eq!(asked.pending_joins.len(), 1);
    assert_eq!(asked.pending_joins[0].display_name, "Alpha");
    assert!(asked.shares.iter().all(|share| share.share_id != share_id));

    // The owner's Drive sees the request and lets the guest in.
    sync(owner_dir.path());
    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let member = iris_drive_core::project_share_access(
        &owner_config.shared_folder(share_id).unwrap().access,
    )
    .members
    .remove(&guest.state.profile_id.to_string())
    .unwrap();
    assert_eq!(member.role, iris_drive_core::ShareRole::Reader);

    // The guest's Drive sees it was let in and brings the folder down.
    sync(guest_dir.path());
    let guest_config = AppConfig::load_or_default(config_path_in(guest_dir.path())).unwrap();
    let joined = guest_config.shared_folder(share_id).unwrap();
    assert_eq!(joined.local_role, iris_drive_core::ShareRole::Reader);
    assert_eq!(
        iris_drive_core::share_action_state(guest_dir.path())
            .unwrap()
            .pending_joins,
        Vec::new()
    );
    assert_eq!(
        read_in_drive(guest_dir.path(), "Alpha/Plan.txt").as_deref(),
        Some("Shared plan")
    );
}

#[test]
fn disabled_links_let_nobody_else_in() {
    let services = super::sharing_test_network::SharingTestServices::new();
    let owner_dir = tempdir().unwrap();
    new_profile(owner_dir.path(), "Owner");
    services.prepare(owner_dir.path());
    cmd_shares_create(owner_dir.path(), "Projects/Alpha", None).unwrap();
    let share_id = AppConfig::load_or_default(config_path_in(owner_dir.path()))
        .unwrap()
        .shared_folders[0]
        .share_id;
    let link = dispatch_cli_share_action(
        owner_dir.path(),
        iris_drive_core::ShareAction::CreateShareJoinLink {
            share_id,
            role: iris_drive_core::ShareRole::Editor,
        },
    )
    .unwrap()
    .share_join_link
    .unwrap();
    cmd_shares_link(owner_dir.path(), &share_id.to_string(), "editor", true).unwrap();

    let guest_dir = tempdir().unwrap();
    let guest = new_profile(guest_dir.path(), "Guest");
    services.prepare(guest_dir.path());
    cmd_shares_accept(guest_dir.path(), &link).unwrap();
    sync(owner_dir.path());

    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    assert!(
        !iris_drive_core::project_share_access(
            &owner_config.shared_folder(share_id).unwrap().access
        )
        .members
        .contains_key(&guest.state.profile_id.to_string())
    );
}
