use super::FfiApp;
use crate::NativeAppAction;
use hashtree_provider::{HashTreeProviderFs, ProviderFs};
use iris_drive_core::AppConfig;
use iris_drive_core::paths::config_path_in;
use nostr_sdk::JsonUtil;
use std::path::Path;

#[test]
fn import_file_action_writes_shared_file_into_provider_root() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "iPhone".to_owned(),
    });

    let source = dir.path().join("share-source.txt");
    std::fs::write(&source, b"from share sheet").unwrap();
    let state = app.dispatch(NativeAppAction::ImportFile {
        display_name: "Shared note.txt".to_owned(),
        source_path: source.display().to_string(),
    });

    assert!(state.error.is_empty(), "{}", state.error);
    assert_eq!(state.ui.file_count, 1);
    assert_eq!(state.ui.visible_file_bytes, 16);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let daemon = iris_drive_core::Daemon::open(dir.path()).unwrap();
        let visible = iris_drive_core::primary_merged_root(daemon.tree(), daemon.config())
            .await
            .unwrap();
        let provider = HashTreeProviderFs::open(daemon.tree_handle(), visible.root_cid)
            .await
            .unwrap();
        let path = "Shared note.txt".to_owned();
        let item = provider.item(&path).await.unwrap();
        let bytes = provider.read(&path, 0, item.size).await.unwrap();
        assert_eq!(bytes, b"from share sheet");
    });
}

#[test]
fn provider_list_includes_summary_and_change_key() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "iPhone".to_owned(),
    });
    let source = dir.path().join("nested.txt");
    std::fs::write(&source, b"nested bytes").unwrap();

    assert!(
        super::native_provider_mkdir_json(&dir.path().display().to_string(), "Reports")["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
    assert!(
        super::native_provider_write_json(
            &dir.path().display().to_string(),
            "Reports/nested.txt",
            &source.display().to_string(),
        )["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    let provider = super::native_provider_list_json(&dir.path().display().to_string());

    assert_eq!(provider["file_count"], 1);
    assert_eq!(provider["visible_file_bytes"], 12);
    assert_eq!(
        provider["directory_paths"].as_array().unwrap(),
        &vec![serde_json::json!("Reports")]
    );
    assert!(
        provider["change_key"]
            .as_str()
            .is_some_and(|key| { key.contains("Reports/nested.txt") && key.contains("file") })
    );
    let entries = provider["entries"].as_array().unwrap();
    let reports = entries
        .iter()
        .find(|entry| entry["path"] == "Reports")
        .unwrap();
    assert_eq!(reports["parent_path"], "");
    assert_eq!(reports["display_name"], "Reports");
    let nested = entries
        .iter()
        .find(|entry| entry["path"] == "Reports/nested.txt")
        .unwrap();
    assert_eq!(nested["parent_path"], "Reports");
    assert_eq!(nested["display_name"], "nested.txt");
}

#[test]
fn provider_resolve_path_normalizes_name_and_avoids_collisions() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        device_label: "iPhone".to_owned(),
    });
    let source = dir.path().join("shared.txt");
    std::fs::write(&source, b"first").unwrap();

    assert!(
        super::native_provider_mkdir_json(&dir.path().display().to_string(), "Reports")["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );
    assert!(
        super::native_provider_write_json(
            &dir.path().display().to_string(),
            "Reports/Shared_file.txt",
            &source.display().to_string(),
        )["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    let resolved = super::native_provider_resolve_path_json(
        &dir.path().display().to_string(),
        "/Reports/",
        "Shared/file.txt",
        "",
    );

    assert_eq!(resolved["parent_path"], "Reports");
    assert_eq!(resolved["display_name"], "Shared_file (2).txt");
    assert_eq!(resolved["path"], "Reports/Shared_file (2).txt");
    assert!(resolved["error"].as_str().unwrap_or_default().is_empty());
}

#[test]
fn provider_normalize_path_validates_native_document_paths() {
    let valid = super::native_provider_normalize_path_json("Reports/note.txt");
    assert_eq!(valid["path"], "Reports/note.txt");
    assert_eq!(valid["parent_path"], "Reports");
    assert_eq!(valid["display_name"], "note.txt");
    assert_eq!(valid["error"], "");

    let invalid = super::native_provider_normalize_path_json("/Reports/note.txt");
    assert_eq!(invalid["path"], "");
    assert!(
        invalid["error"]
            .as_str()
            .unwrap()
            .contains("canonical provider path")
    );
}

#[test]
fn native_sync_applies_remote_drive_root_into_provider_listing() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner_state = owner_app.dispatch(NativeAppAction::CreateProfile {
        device_label: "Mac".to_owned(),
    });
    let owner_account = owner_state.ui.account.unwrap();

    let source_dir = tempfile::tempdir().unwrap();
    std::fs::write(source_dir.path().join("owner-note.txt"), b"from owner").unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let mut daemon = iris_drive_core::Daemon::open(owner_dir.path()).unwrap();
        daemon.import_source_dir(source_dir.path()).await.unwrap();
    });

    let linked_dir = tempfile::tempdir().unwrap();
    let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
    let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
        owner_pubkey: owner_account.device_link_invite,
        device_label: "Phone".to_owned(),
    });
    let linked_account = linked.ui.account.unwrap();
    let approved = owner_app.dispatch(NativeAppAction::ApproveDevice {
        request: linked_account.device_link_request,
        label: "Phone".to_owned(),
    });
    assert!(approved.error.is_empty(), "{}", approved.error);

    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let app_keys_event = nostr_sdk::Event::from_json(
        &owner_config
            .account
            .as_ref()
            .unwrap()
            .app_keys_event
            .as_ref()
            .unwrap()
            .event_json,
    )
    .unwrap();
    let mut linked_config = AppConfig::load_or_default(config_path_in(linked_dir.path())).unwrap();
    iris_drive_core::relay_sync::apply_remote_app_keys_event(&mut linked_config, &app_keys_event)
        .unwrap();
    linked_config
        .save(config_path_in(linked_dir.path()))
        .unwrap();

    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir.path())).unwrap();
    let owner_account_state = owner_config.account.as_ref().unwrap();
    let owner =
        iris_drive_core::Account::load(owner_account_state.clone(), owner_dir.path()).unwrap();
    let drive = owner_config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .unwrap();
    let root = drive
        .device_roots
        .get(&owner_account_state.device_pubkey)
        .unwrap();
    let authorized = owner_account_state
        .app_keys
        .as_ref()
        .unwrap()
        .devices
        .iter()
        .map(|device| device.pubkey.clone())
        .collect::<Vec<_>>();
    let drive_root_event = iris_drive_core::nostr_events::build_drive_root_event(
        owner.device.keys(),
        &owner_account_state.owner_pubkey,
        iris_drive_core::PRIMARY_DRIVE_ID,
        root,
        &authorized,
    )
    .unwrap();
    copy_blocks(owner_dir.path(), linked_dir.path());

    super::run_native_sync_once_with_drive_root_events_for_test(
        linked_dir.path(),
        &[drive_root_event],
    )
    .unwrap();

    let provider = super::native_provider_list_json(&linked_dir.path().display().to_string());
    let entries = provider["entries"].as_array().unwrap();
    assert!(
        entries
            .iter()
            .any(|entry| entry["path"] == "owner-note.txt")
    );
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

#[test]
fn provider_modified_at_index_ignores_unix_epoch_sentinel() {
    let mut index = std::collections::BTreeMap::new();
    crate::provider_metadata::remember_provider_modified_at(&mut index, "old-note.txt", 1);
    crate::provider_metadata::remember_provider_modified_at(
        &mut index,
        "new-note.txt",
        1_700_000_000,
    );

    assert!(!index.contains_key("old-note.txt"));
    assert_eq!(index.get("new-note.txt"), Some(&1_700_000_000));
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
