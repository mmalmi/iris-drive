use super::FfiApp;
use crate::NativeAppAction;
use hashtree_provider::{HashTreeProviderFs, ProviderFs};
use iris_drive_core::AppConfig;
use iris_drive_core::paths::config_path_in;
use std::path::Path;

fn apply_owner_profile_roster_to_linked_config(owner_dir: &Path, linked_dir: &Path) {
    let owner_config = AppConfig::load_or_default(config_path_in(owner_dir)).unwrap();
    let owner_state = owner_config.profile.as_ref().unwrap();
    let roster_frame = iris_drive_core::app_key_link_transport::AppKeyLinkRosterFrame {
        schema: 1,
        profile_id: owner_state.profile_id,
        admin_app_key_pubkey: owner_state.app_key_pubkey.clone(),
        profile_roster_ops: owner_state.profile_roster_ops.clone(),
        sent_at: 123,
    };
    let mut linked_config = AppConfig::load_or_default(config_path_in(linked_dir)).unwrap();
    iris_drive_core::relay_sync::apply_app_key_link_roster_frame(
        &mut linked_config,
        &roster_frame,
        &owner_state.app_key_pubkey,
    )
    .unwrap();
    linked_config.save(config_path_in(linked_dir)).unwrap();
}

fn mark_daemon_live(config_dir: &Path) {
    std::fs::write(
        iris_drive_core::daemon_liveness::daemon_lock_path(config_dir),
        format!("{}\n", std::process::id()),
    )
    .unwrap();
}

#[test]
fn native_provider_mutation_reports_daemon_unavailable_without_live_lock() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });

    let result = super::native_provider_mkdir_json(&dir.path().display().to_string(), "Reports");
    let error = result["error"].as_str().unwrap_or_default();

    assert!(
        error.contains("daemon is unavailable"),
        "unexpected provider mutation result: {result:#}"
    );
}

#[test]
fn provider_mutation_liveness_policy_keeps_desktop_daemon_gate_but_allows_mobile_process() {
    use crate::native_provider::{ProviderMutationLiveness, provider_mutation_liveness_for_target};

    assert_eq!(
        provider_mutation_liveness_for_target("macos"),
        ProviderMutationLiveness::RequireDaemonLock
    );
    assert_eq!(
        provider_mutation_liveness_for_target("linux"),
        ProviderMutationLiveness::RequireDaemonLock
    );
    assert_eq!(
        provider_mutation_liveness_for_target("windows"),
        ProviderMutationLiveness::RequireDaemonLock
    );
    assert_eq!(
        provider_mutation_liveness_for_target("android"),
        ProviderMutationLiveness::InProcessProvider
    );
    assert_eq!(
        provider_mutation_liveness_for_target("ios"),
        ProviderMutationLiveness::InProcessProvider
    );
}

#[test]
fn native_foreground_sync_skips_temporary_fips_endpoint_on_mobile() {
    use crate::native_provider::native_sync_starts_direct_fips_for_target;

    assert!(native_sync_starts_direct_fips_for_target("macos"));
    assert!(native_sync_starts_direct_fips_for_target("linux"));
    assert!(native_sync_starts_direct_fips_for_target("windows"));
    assert!(!native_sync_starts_direct_fips_for_target("android"));
    assert!(!native_sync_starts_direct_fips_for_target("ios"));
}

#[test]
fn import_file_action_writes_shared_file_into_provider_root() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    mark_daemon_live(dir.path());

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
fn import_file_action_preserves_identical_shared_file_copy() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    mark_daemon_live(dir.path());

    let source = dir.path().join("share-source.txt");
    std::fs::write(&source, b"same share bytes").unwrap();
    for _ in 0..2 {
        let state = app.dispatch(NativeAppAction::ImportFile {
            display_name: "Shared note.txt".to_owned(),
            source_path: source.display().to_string(),
        });
        assert!(state.error.is_empty(), "{}", state.error);
    }

    let provider = super::native_provider_list_json(&dir.path().display().to_string());
    assert_eq!(provider["file_count"], 2);
    let paths = provider["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == "file")
        .map(|entry| entry["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["Shared note (2).txt", "Shared note.txt"]);
}

#[test]
fn native_provider_write_preserves_identical_collision_copy() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    mark_daemon_live(dir.path());

    let source = dir.path().join("empty.png");
    std::fs::write(&source, b"").unwrap();
    let data_dir = dir.path().display().to_string();

    let first =
        super::native_provider_write_json(&data_dir, "photo.png", &source.display().to_string());
    assert!(
        first["error"].as_str().unwrap_or_default().is_empty(),
        "unexpected first write result: {first:#}"
    );

    let second = super::native_provider_write_json(
        &data_dir,
        "photo (2) (3).png",
        &source.display().to_string(),
    );
    assert!(
        second["error"].as_str().unwrap_or_default().is_empty(),
        "unexpected second write result: {second:#}"
    );
    assert_eq!(second["path"], "photo (2) (3).png");

    let provider = super::native_provider_list_json(&data_dir);
    assert_eq!(provider["file_count"], 2);
    let paths = provider["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == "file")
        .map(|entry| entry["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["photo (2) (3).png", "photo.png"]);
}

#[test]
fn native_provider_write_rejects_probable_os_placeholder_collision() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    mark_daemon_live(dir.path());

    let real_source = dir.path().join("real.png");
    std::fs::write(&real_source, b"real image bytes").unwrap();
    let empty_source = dir.path().join("empty.png");
    std::fs::write(&empty_source, b"").unwrap();
    let data_dir = dir.path().display().to_string();

    let first = super::native_provider_write_json(
        &data_dir,
        "photo.png",
        &real_source.display().to_string(),
    );
    assert!(
        first["error"].as_str().unwrap_or_default().is_empty(),
        "unexpected first write result: {first:#}"
    );
    let ordinary_empty = super::native_provider_write_json(
        &data_dir,
        "photo copy (2).png",
        &empty_source.display().to_string(),
    );
    assert!(
        ordinary_empty["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty(),
        "unexpected ordinary empty copy result: {ordinary_empty:#}"
    );

    let placeholder = super::native_provider_write_json(
        &data_dir,
        "photo copy (2) (2).png",
        &empty_source.display().to_string(),
    );
    assert!(
        placeholder["error"]
            .as_str()
            .unwrap_or_default()
            .contains("placeholder copy"),
        "unexpected placeholder write result: {placeholder:#}"
    );

    let provider = super::native_provider_list_json(&data_dir);
    assert_eq!(provider["file_count"], 2);
    let paths = provider["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == "file")
        .map(|entry| entry["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(paths, vec!["photo copy (2).png", "photo.png"]);
}

#[test]
fn import_file_action_preserves_identical_collision_copy() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    mark_daemon_live(dir.path());

    let occupied = dir.path().join("occupied.txt");
    std::fs::write(&occupied, b"different existing bytes").unwrap();
    assert!(
        super::native_provider_write_json(
            &dir.path().display().to_string(),
            "Shared note.txt",
            &occupied.display().to_string(),
        )["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    let source = dir.path().join("share-source.txt");
    std::fs::write(&source, b"same share bytes").unwrap();
    for _ in 0..2 {
        let state = app.dispatch(NativeAppAction::ImportFile {
            display_name: "Shared note.txt".to_owned(),
            source_path: source.display().to_string(),
        });
        assert!(state.error.is_empty(), "{}", state.error);
    }

    let provider = super::native_provider_list_json(&dir.path().display().to_string());
    assert_eq!(provider["file_count"], 3);
    let paths = provider["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["kind"] == "file")
        .map(|entry| entry["path"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            "Shared note (2).txt",
            "Shared note (3).txt",
            "Shared note.txt"
        ]
    );
}

#[test]
fn import_content_link_action_downloads_into_provider_root() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    mark_daemon_live(dir.path());
    let _download = crate::native_provider::download_content_link_bytes_for_test(
        b"from public drive link".to_vec(),
    );
    let link = format!(
        "https://drive.iris.to/#/{}/sites/docs/Freenet%20paper.pdf?fullscreen=1",
        iris_drive_core::gateway::IRIS_SITES_PORTAL_NPUB
    );

    let state = app.dispatch(NativeAppAction::ImportContentLink { link });

    assert!(state.error.is_empty(), "{}", state.error);
    assert_eq!(state.ui.file_count, 1);
    assert_eq!(state.ui.visible_file_bytes, 22);
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let daemon = iris_drive_core::Daemon::open(dir.path()).unwrap();
        let visible = iris_drive_core::primary_merged_root(daemon.tree(), daemon.config())
            .await
            .unwrap();
        let provider = HashTreeProviderFs::open(daemon.tree_handle(), visible.root_cid)
            .await
            .unwrap();
        let path = "Freenet paper.pdf".to_owned();
        let item = provider.item(&path).await.unwrap();
        let bytes = provider.read(&path, 0, item.size).await.unwrap();
        assert_eq!(bytes, b"from public drive link");
    });
}

#[test]
fn provider_list_includes_summary_and_change_key() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    mark_daemon_live(dir.path());
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

    let state = app.refresh();
    assert_eq!(state.ui.file_count, 1);
    assert_eq!(state.ui.visible_file_bytes, 12);
    assert_eq!(state.ui.provider_directory_paths, vec!["Reports"]);
    assert!(state.ui.provider_change_key.contains("Reports/nested.txt"));
}

#[test]
fn provider_list_includes_modified_at_for_empty_directories() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    mark_daemon_live(dir.path());

    assert!(
        super::native_provider_mkdir_json(&dir.path().display().to_string(), "Empty")["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    let provider = super::native_provider_list_json(&dir.path().display().to_string());
    let entries = provider["entries"].as_array().unwrap();
    let empty = entries
        .iter()
        .find(|entry| entry["path"] == "Empty")
        .expect("provider list includes empty directory");
    assert!(
        empty["modified_at"]
            .as_i64()
            .is_some_and(|modified_at| modified_at >= 946_684_800),
        "provider list should include non-epoch directory modification time: {empty:#?}"
    );
}

#[test]
fn app_constructor_defers_provider_summary_until_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    mark_daemon_live(dir.path());
    let source = dir.path().join("startup.txt");
    std::fs::write(&source, b"startup bytes").unwrap();

    assert!(
        super::native_provider_write_json(
            &dir.path().display().to_string(),
            "startup.txt",
            &source.display().to_string(),
        )["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    let restarted = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let initial = restarted.state();
    assert!(initial.ui.setup_complete);
    assert!(initial.ui.profile.is_some());
    assert_eq!(initial.ui.file_count, 0);
    assert_eq!(initial.ui.visible_file_bytes, 0);
    assert!(initial.ui.provider_change_key.is_empty());

    let refreshed = restarted.refresh();
    assert_eq!(refreshed.ui.file_count, 1);
    assert_eq!(refreshed.ui.visible_file_bytes, 13);
    assert!(refreshed.ui.provider_change_key.contains("startup.txt"));
}

#[test]
fn refresh_profile_skips_provider_summary_until_full_refresh() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    mark_daemon_live(dir.path());
    let source = dir.path().join("approval-fast-path.txt");
    std::fs::write(&source, b"fast profile refresh").unwrap();

    assert!(
        super::native_provider_write_json(
            &dir.path().display().to_string(),
            "approval-fast-path.txt",
            &source.display().to_string(),
        )["error"]
            .as_str()
            .unwrap_or_default()
            .is_empty()
    );

    let profile_refresh = app.dispatch(NativeAppAction::RefreshProfile);
    assert!(profile_refresh.ui.setup_complete);
    assert_eq!(profile_refresh.ui.file_count, 0);
    assert_eq!(profile_refresh.ui.visible_file_bytes, 0);
    assert!(profile_refresh.ui.provider_change_key.is_empty());

    let full_refresh = app.dispatch(NativeAppAction::Refresh);
    assert_eq!(full_refresh.ui.file_count, 1);
    assert_eq!(full_refresh.ui.visible_file_bytes, 20);
    assert!(
        full_refresh
            .ui
            .provider_change_key
            .contains("approval-fast-path.txt")
    );
}

#[test]
fn provider_resolve_path_normalizes_name_and_avoids_collisions() {
    let dir = tempfile::tempdir().unwrap();
    let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());
    let _ = app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "iPhone".to_owned(),
    });
    mark_daemon_live(dir.path());
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
fn provider_child_document_json_uses_core_path_relation() {
    let child = super::native_provider_is_child_document_json("/Reports/", "/Reports/monthly.pdf");
    assert_eq!(child["is_child"], true);
    assert_eq!(child["error"], "");

    let sibling =
        super::native_provider_is_child_document_json("Reports", "Reports-old/monthly.pdf");
    assert_eq!(sibling["is_child"], false);
    assert_eq!(sibling["error"], "");

    let invalid = super::native_provider_is_child_document_json("Reports", "Reports\\monthly.pdf");
    assert_eq!(invalid["is_child"], false);
    assert!(
        invalid["error"]
            .as_str()
            .unwrap_or_default()
            .contains("unsafe provider path")
    );
}

#[test]
fn native_sync_applies_remote_drive_root_into_provider_listing() {
    let owner_dir = tempfile::tempdir().unwrap();
    let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
    let owner_state = owner_app.dispatch(NativeAppAction::CreateProfile {
        app_key_label: "Mac".to_owned(),
    });
    let owner_account = owner_state.ui.profile.unwrap();

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
    let drive = owner_config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .unwrap();
    let root = drive
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
    let drive_root_event = iris_drive_core::nostr_events::build_drive_root_event(
        owner.app_key.keys(),
        &owner_account_state.root_scope_id(),
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
