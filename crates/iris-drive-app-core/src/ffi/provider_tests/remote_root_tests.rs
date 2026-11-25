//! A linked device lists the Drive after sync applies another device's root.

use std::path::Path;

use iris_drive_core::AppConfig;
use iris_drive_core::paths::config_path_in;
use nostr_sdk::Event;

use super::apply_owner_profile_roster_to_linked_config;
use crate::NativeAppAction;
use crate::ffi::FfiApp;

/// An owner whose Drive holds `owner_files`, a device linked to it, and the
/// owner's signed Drive root event. Nothing of the owner's Drive is stored on
/// the linked device yet.
struct LinkedDevices {
    owner_dir: tempfile::TempDir,
    linked_dir: tempfile::TempDir,
    owner_root: Event,
    owner_root_cid: String,
}

fn linked_devices(owner_files: &Path) -> LinkedDevices {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner_state = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let owner_account = owner_state.ui.profile.unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let mut daemon = iris_drive_core::Daemon::open(owner_dir.path()).unwrap();
        daemon.import_source_dir(owner_files).await.unwrap();
    });

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        link_target: owner_account.app_key_link_invite,
        app_key_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.profile.unwrap();
    let approved = owner_app.dispatch(NativeAppAction::ApproveDevice {
        request: linked_account.app_key_link_request,
        label: "Phone".to_owned(),
    });
    assert!(approved.error.is_empty(), "{}", approved.error);
    apply_owner_profile_roster_to_linked_config(owner_dir.path(), linked_dir.path());

    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let owner_account_state = owner_config.profile.as_ref().unwrap();
    let owner =
        iris_drive_core::Profile::load(owner_account_state.clone(), owner_dir.path()).unwrap();
    let root = owner_config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&owner_account_state.app_key_pubkey)
        .unwrap();
    let authorized = owner_account_state
        .app_keys
        .as_ref()
        .unwrap()
        .app_actors
        .iter()
        .map(|device| device.pubkey.clone())
        .collect::<Vec<_>>();
    let owner_root = iris_drive_core::nostr_events::build_drive_root_event(
        owner.app_key.keys(),
        &owner_account_state.root_scope_id(),
        iris_drive_core::PRIMARY_DRIVE_ID,
        root,
        &authorized,
    )
    .unwrap();
    LinkedDevices {
        owner_root_cid: root.root_cid.clone(),
        owner_dir,
        linked_dir,
        owner_root,
    }
}

#[test]
fn native_sync_applies_remote_drive_root_into_provider_listing() {
    let source_dir = tempfile::tempdir().unwrap();
    std::fs::write(source_dir.path().join("owner-note.txt"), b"from owner").unwrap();
    let devices = linked_devices(source_dir.path());
    copy_blocks(devices.owner_dir.path(), devices.linked_dir.path());

    crate::ffi::run_native_sync_once_with_drive_root_events_for_test(
        devices.linked_dir.path(),
        &[devices.owner_root],
    )
    .unwrap();

    let provider =
        crate::ffi::native_provider_list_json(&devices.linked_dir.path().display().to_string());
    let entries = provider["entries"].as_array().unwrap();
    let owner_note = entries
        .iter()
        .find(|entry| entry["path"] == "owner-note.txt")
        .expect("provider list includes owner note");
    assert!(
        owner_note["modified_at"]
            .as_i64()
            .is_some_and(|modified_at| modified_at > 0),
        "provider list should include non-epoch modification time: {owner_note:#?}"
    );
}

/// The physical gate's share sheet: sync recorded the other device's new root,
/// the app was suspended before its blocks arrived, and the share extension
/// lists where the shared file can go.
#[test]
fn share_destinations_list_a_peer_root_applied_before_its_blocks_arrive() {
    use crate::ffi::tests::sharing_network::LocalBlossomServer;

    let source_dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(source_dir.path().join("Owner folder")).unwrap();
    std::fs::write(
        source_dir.path().join("Owner folder/owner-note.txt"),
        b"from owner",
    )
    .unwrap();
    let devices = linked_devices(source_dir.path());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let blossom = runtime.block_on(LocalBlossomServer::spawn());
    let mut owner_config =
        AppConfig::load_or_default(config_path_in(devices.owner_dir.path())).unwrap();
    owner_config.blossom_servers = vec![blossom.url.clone()];
    runtime
        .block_on(
            crate::native_provider::upload_current_app_key_root_to_blossom(
                devices.owner_dir.path(),
                &owner_config,
                &devices.owner_root_cid,
            ),
        )
        .unwrap();
    let mut linked_config =
        AppConfig::load_or_default(config_path_in(devices.linked_dir.path())).unwrap();
    linked_config.blossom_servers = vec![blossom.url.clone()];
    linked_config
        .save(config_path_in(devices.linked_dir.path()))
        .unwrap();

    crate::ffi::run_native_sync_once_with_drive_root_events_for_test(
        devices.linked_dir.path(),
        &[devices.owner_root],
    )
    .unwrap();

    let linked = devices.linked_dir.path().display().to_string();
    let provider = crate::ffi::native_provider_list_json(&linked);
    assert_eq!(provider.get("error"), None, "{provider}");
    assert_eq!(
        provider["save_folders"],
        serde_json::json!(["Owner folder"])
    );
    assert!(
        provider["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["path"] == "Owner folder/owner-note.txt"),
        "{provider}"
    );
}

fn copy_blocks(from: &Path, to: &Path) {
    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for entry in std::fs::read_dir(from).unwrap() {
            let entry = entry.unwrap();
            let from_path = entry.path();
            let to_path = to.join(entry.file_name());
            if from_path.is_dir() {
                copy_dir(&from_path, &to_path);
            } else {
                std::fs::copy(&from_path, &to_path).unwrap();
            }
        }
    }

    copy_dir(&from.join("blocks"), &to.join("blocks"));
}
