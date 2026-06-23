use super::*;

#[test]
fn relays_can_be_edited_from_cli() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();

    idrive(dir.path())
        .args(["relays", "add", "relay.example"])
        .assert()
        .success();
    let added: serde_json::Value =
        serde_json::from_slice(&idrive(dir.path()).arg("relays").output().unwrap().stdout).unwrap();
    assert_eq!(
        added.as_array().unwrap().last().unwrap(),
        "wss://relay.example"
    );

    idrive(dir.path())
        .args(["relays", "update", "relay.example", "wss://relay2.example"])
        .assert()
        .success();
    let updated: serde_json::Value =
        serde_json::from_slice(&idrive(dir.path()).arg("relays").output().unwrap().stdout).unwrap();
    assert!(
        updated
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("wss://relay2.example"))
    );
    assert!(
        !updated
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("wss://relay.example"))
    );

    idrive(dir.path())
        .args(["relays", "remove", "wss://relay2.example"])
        .assert()
        .success();
    let removed: serde_json::Value =
        serde_json::from_slice(&idrive(dir.path()).arg("relays").output().unwrap().stdout).unwrap();
    assert!(
        !removed
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("wss://relay2.example"))
    );

    idrive(dir.path())
        .args(["relays", "reset"])
        .assert()
        .success();
    let reset: serde_json::Value =
        serde_json::from_slice(&idrive(dir.path()).arg("relays").output().unwrap().stdout).unwrap();
    let default_relays: Vec<_> = iris_drive_core::config::DEFAULT_RELAYS
        .iter()
        .map(|relay| serde_json::json!(relay))
        .collect();
    assert_eq!(reset, serde_json::json!(default_relays));
}

#[test]
fn drives_lists_primary_after_init() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    idrive(dir.path())
        .arg("drives")
        .assert()
        .success()
        .stdout(contains("main"))
        .stdout(contains("owner"))
        .stdout(contains("My Drive"));
}

#[test]
fn drives_before_init_shows_empty_message() {
    let dir = tempdir().unwrap();
    idrive(dir.path())
        .arg("drives")
        .assert()
        .success()
        .stdout(contains("idrive init"));
}

#[test]
fn index_command_prints_root_cid_and_count() {
    let dir = tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), b"alpha").unwrap();
    std::fs::write(dir.path().join("b.txt"), b"beta").unwrap();
    let cfg = tempdir().unwrap();
    let out = idrive(cfg.path())
        .arg("index")
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let body = String::from_utf8(out.stdout).unwrap();
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["top_level_entries"], 2);
    let root_cid = Cid::parse(v["root_cid"].as_str().unwrap()).unwrap();
    assert!(
        root_cid.key.is_some(),
        "index roots should be private by default"
    );
}

#[test]
fn import_persists_to_blocks_dir_and_advances_root() {
    let work = tempdir().unwrap();
    std::fs::create_dir(work.path().join("junk")).unwrap();
    std::fs::write(work.path().join("junk").join("hello.txt"), b"hi there").unwrap();
    std::fs::write(work.path().join("junk").join("again.txt"), b"still here").unwrap();

    let cfg = tempdir().unwrap();
    idrive(cfg.path()).arg("init").assert().success();

    let out = idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    let root_cid = v["root_cid"].as_str().unwrap();
    assert_eq!(v["top_level_entries"], 1);
    assert!(cfg.path().join("blocks").is_dir());
    assert!(!root_cid.is_empty());

    // status now reports the recorded root CID on the primary drive.
    let status_out = idrive(cfg.path()).arg("status").output().unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(status_out.stdout).unwrap()).unwrap();
    assert_eq!(v["drives"][0]["last_root_cid"], root_cid);
    assert!(v["drives"][0].get("working_dir").is_none());
    assert_eq!(v["hashtree"]["current_root_cid"], root_cid);
    assert_eq!(v["hashtree"]["current_root_private"], true);
    assert!(v["drives"][0].get("owner_pubkey").is_none());
    let drive_scope = v["drives"][0]["root_scope_id"].as_str().unwrap();
    assert!(
        drive_scope
            .parse::<iris_drive_core::IrisProfileId>()
            .is_ok()
    );
    assert_eq!(
        v["hashtree"]["drive_iris_to_url"],
        format!("https://drive.iris.to/#/{drive_scope}/main")
    );
    assert_eq!(
        v["hashtree"]["files_iris_to_url"],
        v["hashtree"]["drive_iris_to_url"]
    );
    let snapshot_url = v["hashtree"]["snapshot_url"].as_str().unwrap();
    assert_eq!(v["hashtree"]["permalink_url"], snapshot_url);
    let nhash = snapshot_url
        .strip_prefix("https://drive.iris.to/#/")
        .expect("snapshot link should use drive.iris.to nhash route");
    let decoded = nhash_decode(nhash).expect("decode snapshot link nhash");
    let cid = Cid::parse(root_cid).expect("parse root cid");
    assert_eq!(decoded.hash, cid.hash);
    assert_eq!(decoded.decrypt_key, cid.key);
    assert_eq!(v["hashtree"]["file_count"], 2);
    assert_eq!(v["hashtree"]["top_level_entries"], 1);
    assert_eq!(v["hashtree"]["visible_file_bytes"], 18);
    assert!(v["hashtree"]["local_block_count"].as_u64().unwrap() > 0);
    assert!(v["hashtree"]["local_block_bytes"].as_u64().unwrap() > 0);
    assert_eq!(v["network"]["published_app_key_roots"], 1);
    assert_eq!(v["peers"][0]["has_root"], true);
    assert_eq!(v["peers"][0]["root_cid"], root_cid);
    assert_eq!(v["peers"][0]["root_private"], true);
}

#[test]
fn import_before_init_errors() {
    let work = tempdir().unwrap();
    let cfg = tempdir().unwrap();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .failure();
}

#[test]
fn npub_is_stable_across_invocations() {
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    let one = String::from_utf8(idrive(dir.path()).arg("whoami").output().unwrap().stdout).unwrap();
    let two = String::from_utf8(idrive(dir.path()).arg("whoami").output().unwrap().stdout).unwrap();
    assert_eq!(one, two);
}

#[test]
fn list_after_import_shows_merged_view() {
    let cfg = tempdir().unwrap();
    let work = tempdir().unwrap();
    std::fs::write(work.path().join("alpha.txt"), b"alpha").unwrap();
    std::fs::write(work.path().join("beta.txt"), b"beta-bytes").unwrap();

    idrive(cfg.path()).arg("init").assert().success();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();

    let out = idrive(cfg.path()).arg("list").output().unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["authorized_app_keys"], 1);
    assert_eq!(v["app_key_roots_present"], 1);
    let files = v["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    let paths: Vec<&str> = files.iter().map(|f| f["path"].as_str().unwrap()).collect();
    assert_eq!(paths, vec!["alpha.txt", "beta.txt"]);
    // Sizes recorded.
    assert_eq!(files[0]["size"], 5);
    assert_eq!(files[1]["size"], 10);
}

#[test]
fn list_uses_projected_view_for_provider_collision_copies() {
    let cfg = tempdir().unwrap();
    let work = tempdir().unwrap();
    std::fs::write(work.path().join("photo.png"), b"real image").unwrap();
    std::fs::write(work.path().join("photo (2).png"), b"").unwrap();
    std::fs::write(work.path().join("photo copy.png"), b"real image").unwrap();
    std::fs::write(work.path().join("photo copy (2).png"), b"").unwrap();

    idrive(cfg.path()).arg("init").assert().success();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();

    assert_list_paths(cfg.path(), &["photo.png"]);
}

#[test]
fn provider_commands_operate_on_virtual_root() {
    let cfg = tempdir().unwrap();
    let work = tempdir().unwrap();
    std::fs::create_dir(work.path().join("docs")).unwrap();
    std::fs::write(work.path().join("docs").join("note.txt"), b"hello virtual").unwrap();

    idrive(cfg.path()).arg("init").assert().success();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();

    let listed = run_json(cfg.path(), &["provider", "list"]);
    let entries = listed["entries"].as_array().unwrap();
    let paths: Vec<&str> = entries
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    assert!(paths.contains(&"docs"));
    assert!(paths.contains(&"docs/note.txt"));
    assert_eq!(listed["file_count"], 1);
    let original_note_version = entries
        .iter()
        .find(|entry| entry["path"] == "docs/note.txt")
        .and_then(|entry| entry["version"].as_str())
        .expect("provider list includes per-entry version");
    assert!(!original_note_version.is_empty());

    let scratch = tempdir().unwrap();
    let original = scratch.path().join("note.txt");
    idrive(cfg.path())
        .args(["provider", "read", "docs/note.txt"])
        .arg(&original)
        .assert()
        .success();
    assert_eq!(std::fs::read(&original).unwrap(), b"hello virtual");

    let cache = tempdir().unwrap();
    let cache_json = run_json(
        cfg.path(),
        &["provider", "hydrate-cache", cache.path().to_str().unwrap()],
    );
    assert_eq!(cache_json["file_count"], 1);
    assert_eq!(
        std::fs::read(cache.path().join("docs").join("note.txt")).unwrap(),
        b"hello virtual"
    );

    std::fs::write(
        cfg.path().join("daemon.lock"),
        format!("{}\n", std::process::id()),
    )
    .unwrap();

    let source = scratch.path().join("new.txt");
    std::fs::write(&source, b"from provider").unwrap();
    idrive(cfg.path())
        .args(["provider", "write", "docs/new.txt"])
        .arg(&source)
        .assert()
        .success();

    let created = scratch.path().join("new-out.txt");
    idrive(cfg.path())
        .args(["provider", "read", "docs/new.txt"])
        .arg(&created)
        .assert()
        .success();
    assert_eq!(std::fs::read(&created).unwrap(), b"from provider");
    let relisted = run_json(cfg.path(), &["provider", "list"]);
    let new_note_version = relisted["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == "docs/new.txt")
        .and_then(|entry| entry["version"].as_str())
        .expect("new provider file includes per-entry version");
    assert!(!new_note_version.is_empty());
    assert_ne!(original_note_version, new_note_version);

    idrive(cfg.path())
        .args(["provider", "rename", "docs/new.txt", "docs/renamed.txt"])
        .assert()
        .success();
    let renamed = scratch.path().join("renamed.txt");
    idrive(cfg.path())
        .args(["provider", "read", "docs/renamed.txt"])
        .arg(&renamed)
        .assert()
        .success();
    assert_eq!(std::fs::read(&renamed).unwrap(), b"from provider");

    idrive(cfg.path())
        .args(["provider", "delete", "docs/renamed.txt"])
        .assert()
        .success();
    idrive(cfg.path())
        .args(["provider", "read", "docs/renamed.txt"])
        .arg(scratch.path().join("missing.txt"))
        .assert()
        .failure();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn relay_publish_sends_profile_ops_without_legacy_app_keys_roster() {
    let relay = LocalNostrRelay::spawn().await;
    let blossom = LocalBlossomServer::spawn().await;

    let cfg_a = tempdir().unwrap();
    let cfg_b = tempdir().unwrap();
    let work_a = tempdir().unwrap();

    configure_local_blossom(cfg_a.path(), &blossom.url);
    configure_local_blossom(cfg_b.path(), &blossom.url);

    let init_a = run_json(cfg_a.path(), &["init", "--label", "device-a"]);
    let owner_invite = init_a["app_key_link_invite"]["url"].as_str().unwrap();

    let linked_b = run_json(cfg_b.path(), &["link", owner_invite, "--label", "device-b"]);
    let device_b_request = linked_b["app_key_link_request"]["url"]
        .as_str()
        .unwrap()
        .to_string();

    let approved = run_json(cfg_a.path(), &["approve", &device_b_request]);
    assert_eq!(approved["roster_size"], 2);

    std::fs::write(work_a.path().join("from-a.txt"), b"hello from a").unwrap();
    run_json(cfg_a.path(), &["import", work_a.path().to_str().unwrap()]);
    let publish_a = run_json(
        cfg_a.path(),
        &["publish", "--relay", &relay.url, "--timeout", "2"],
    );
    assert!(publish_a.get("published_app_keys").is_none());
    assert!(
        publish_a["published_profile_roster_ops"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );
    assert_eq!(publish_a["published_drive_root"], true);
    assert!(
        publish_a["blossom_upload"]["uploaded"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );

    let sync_b = run_json(
        cfg_b.path(),
        &["sync", "--relay", &relay.url, "--timeout", "2"],
    );
    assert!(sync_b.get("app_keys_event_applied").is_none());
    assert!(
        sync_b["profile_roster_ops_applied"]
            .as_u64()
            .unwrap_or_default()
            > 0
    );
    assert_eq!(sync_b["drive_root_events_applied"], 1);
    assert_list_paths(cfg_b.path(), &["from-a.txt"]);
}

fn assert_list_paths(config_dir: &std::path::Path, expected: &[&str]) {
    let listing = run_json(config_dir, &["list"]);
    let paths: Vec<&str> = listing["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, expected, "listing: {listing:#}");
}

#[test]
fn list_before_import_is_empty() {
    let cfg = tempdir().unwrap();
    idrive(cfg.path()).arg("init").assert().success();
    let out = idrive(cfg.path()).arg("list").output().unwrap();
    assert!(out.status.success());
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    assert_eq!(v["authorized_app_keys"], 1);
    assert_eq!(v["app_key_roots_present"], 0);
    assert_eq!(v["files"].as_array().unwrap().len(), 0);
}

#[test]
fn list_before_init_errors() {
    let cfg = tempdir().unwrap();
    idrive(cfg.path()).arg("list").assert().failure();
}

#[test]
fn delete_then_reimport_marks_path_suppressed() {
    let cfg = tempdir().unwrap();
    let work = tempdir().unwrap();
    std::fs::write(work.path().join("keep.txt"), b"k").unwrap();
    std::fs::write(work.path().join("drop.txt"), b"d").unwrap();
    idrive(cfg.path()).arg("init").assert().success();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();

    // Delete one file, re-import, list should hide it and show as suppressed.
    std::fs::remove_file(work.path().join("drop.txt")).unwrap();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();

    let out = idrive(cfg.path()).arg("list").output().unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    let paths: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["keep.txt"]);
    let suppressed: Vec<&str> = v["suppressed_by_tombstone"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s.as_str().unwrap())
        .collect();
    assert_eq!(suppressed, vec!["drop.txt"]);
}

#[test]
fn deleted_file_can_return_and_tombstone_drops() {
    let cfg = tempdir().unwrap();
    let work = tempdir().unwrap();
    std::fs::write(work.path().join("file.txt"), b"v1").unwrap();
    idrive(cfg.path()).arg("init").assert().success();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();
    std::fs::remove_file(work.path().join("file.txt")).unwrap();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();

    // File comes back.
    std::fs::write(work.path().join("file.txt"), b"v2-back").unwrap();
    idrive(cfg.path())
        .arg("import")
        .arg(work.path())
        .assert()
        .success();

    let out = idrive(cfg.path()).arg("list").output().unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&String::from_utf8(out.stdout).unwrap()).unwrap();
    let paths: Vec<&str> = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, vec!["file.txt"]);
    assert!(v["suppressed_by_tombstone"].as_array().unwrap().is_empty());
}

#[test]
fn restore_after_init_errors_without_force_path() {
    // For now restore refuses to overwrite an existing install.
    let dir = tempdir().unwrap();
    idrive(dir.path()).arg("init").assert().success();
    let recovery_phrase = iris_drive_core::recovery_phrase::generate_recovery_phrase().unwrap();
    idrive(dir.path())
        .args(["restore", &recovery_phrase])
        .assert()
        .failure();
}
