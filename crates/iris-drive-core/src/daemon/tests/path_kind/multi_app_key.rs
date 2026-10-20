use super::*;
use crate::root_meta::RootObservation;

#[tokio::test]
async fn unrelated_descendant_republish_keeps_remote_path_kind_replacements_canonical() {
    let config_dir = tempdir().unwrap();
    let mut local_profile = init_config_with_account(config_dir.path());
    let local_app_key = local_profile.state.app_key_pubkey.clone();
    let replacing_app_key =
        Identity::generate(config_dir.path().join("remote-replacer.key")).pubkey_hex();
    approve_remote_app_key(
        config_dir.path(),
        &mut local_profile,
        &replacing_app_key,
        "remote-replacer",
    );

    let mut daemon = Daemon::open(config_dir.path()).unwrap();
    let original_source = tempdir().unwrap();
    std::fs::create_dir(original_source.path().join("kind-folder")).unwrap();
    std::fs::write(
        original_source.path().join("kind-folder/original.txt"),
        b"original directory child",
    )
    .unwrap();
    std::fs::write(
        original_source.path().join("kind-file"),
        b"original file bytes",
    )
    .unwrap();
    daemon
        .import_source_dir(original_source.path())
        .await
        .unwrap();

    let original_ref = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&local_app_key)
        .unwrap()
        .clone();
    let original_root = Cid::parse(&original_ref.root_cid).unwrap();

    let replacement_source = tempdir().unwrap();
    std::fs::write(
        replacement_source.path().join("kind-folder"),
        b"replacement file bytes",
    )
    .unwrap();
    std::fs::create_dir(replacement_source.path().join("kind-file")).unwrap();
    std::fs::write(
        replacement_source.path().join("kind-file/replacement.txt"),
        b"replacement directory child",
    )
    .unwrap();
    let replacement_visible = crate::indexer::index_dir(daemon.tree(), replacement_source.path())
        .await
        .unwrap();
    let replacement_generation = original_ref.published_at + 1;
    let replacement_meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.into(),
        app_key_pubkey: replacing_app_key.clone(),
        app_key_seq: 1,
        dck_generation: original_ref.dck_generation,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::from([(
            local_app_key.clone(),
            RootObservation {
                app_key_seq: original_ref.app_key_seq,
                root_cid: original_ref.root_cid.clone(),
            },
        )]),
        created_at: replacement_generation,
    };
    let replacement_paths = BTreeSet::from(["kind-file".to_string(), "kind-folder".to_string()]);
    let replacement_root =
        crate::indexer::layer_history_and_meta_on_root_with_tombstone_base_paths_and_replacements(
            daemon.tree(),
            replacement_visible,
            None,
            Some(&original_root),
            replacement_generation,
            Some(&replacement_meta),
            Some(&replacement_paths),
            Some(&replacement_paths),
        )
        .await
        .unwrap();
    assert_eq!(
        crate::indexer::read_path_kind_replacements(daemon.tree(), &replacement_root)
            .await
            .unwrap(),
        Some(BTreeMap::from([
            ("kind-file".to_string(), replacement_generation),
            ("kind-folder".to_string(), replacement_generation),
        ])),
        "the remote replacement root owns both exact durable roles"
    );
    drop(daemon);

    let mut config = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    let mut drive = config.drive(PRIMARY_DRIVE_ID).unwrap().clone();
    drive.app_key_roots.insert(
        replacing_app_key.clone(),
        AppKeyRootRef::from_meta(
            replacement_root.to_string(),
            replacement_meta.created_at,
            &replacement_meta,
        ),
    );
    config.upsert_drive(drive);
    config.save(config_path_in(config_dir.path())).unwrap();

    let mut daemon = Daemon::open(config_dir.path()).unwrap();
    let projected = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap()
        .root_cid;
    let folder_conflict = format!("kind-folder (conflict from {local_app_key})");
    let file_conflict = format!("kind-file (conflict from {local_app_key})");
    assert_eq!(
        visible_file_bytes(&daemon, &projected, "kind-folder").await,
        b"replacement file bytes"
    );
    assert_eq!(
        visible_file_bytes(
            &daemon,
            &projected,
            &format!("{folder_conflict}/original.txt"),
        )
        .await,
        b"original directory child"
    );
    assert_eq!(
        visible_file_bytes(&daemon, &projected, "kind-file/replacement.txt").await,
        b"replacement directory child"
    );
    assert_eq!(
        visible_file_bytes(&daemon, &projected, &file_conflict).await,
        b"original file bytes"
    );

    let (unrelated, unrelated_size) = daemon
        .tree()
        .put_file(b"unrelated local edit")
        .await
        .unwrap();
    let edited = daemon
        .tree()
        .set_entry(
            &projected,
            &[],
            "unrelated.txt",
            &unrelated,
            unrelated_size,
            hashtree_core::LinkType::File,
        )
        .await
        .unwrap();
    daemon
        .import_visible_root_with_tombstone_base(edited, Some(projected))
        .await
        .unwrap();
    let unrelated_local_root = daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(&local_app_key)
        .unwrap();
    assert_eq!(
        crate::indexer::read_path_kind_replacements(
            daemon.tree(),
            &Cid::parse(&unrelated_local_root.root_cid).unwrap(),
        )
        .await
        .unwrap(),
        Some(BTreeMap::new()),
        "the unrelated descendant is role-aware but claims neither replacement"
    );

    let reprojected = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap()
        .root_cid;
    assert_eq!(
        visible_file_bytes(&daemon, &reprojected, "kind-folder").await,
        b"replacement file bytes",
        "the active remote directory-to-file role remains canonical"
    );
    assert_eq!(
        visible_file_bytes(
            &daemon,
            &reprojected,
            &format!("{folder_conflict}/original.txt"),
        )
        .await,
        b"original directory child"
    );
    assert_eq!(
        visible_file_bytes(&daemon, &reprojected, "kind-file/replacement.txt").await,
        b"replacement directory child",
        "the active remote file-to-directory role remains canonical"
    );
    assert_eq!(
        visible_file_bytes(&daemon, &reprojected, &file_conflict).await,
        b"original file bytes"
    );
    assert_eq!(
        visible_file_bytes(&daemon, &reprojected, "unrelated.txt").await,
        b"unrelated local edit"
    );

    let top_level_names = daemon
        .tree()
        .list_directory(&reprojected)
        .await
        .unwrap()
        .into_iter()
        .map(|entry| entry.name)
        .collect::<Vec<_>>();
    assert_eq!(
        top_level_names
            .iter()
            .filter(|name| name.starts_with("kind-folder (conflict from "))
            .count(),
        1,
        "the original directory conflict is materialized exactly once"
    );
    assert_eq!(
        top_level_names
            .iter()
            .filter(|name| name.starts_with("kind-file (conflict from "))
            .count(),
        1,
        "the original file conflict is materialized exactly once"
    );

    // An explicit later delete/recreate is different from the unrelated
    // republish above. Its fresh exact barriers must retire the remote roles at
    // these paths even though the remote root still advertises them.
    let deleted = daemon
        .tree()
        .remove_entry(&reprojected, &[], "kind-folder")
        .await
        .unwrap();
    let deleted = daemon
        .tree()
        .remove_entry(&deleted, &[], "kind-file")
        .await
        .unwrap();
    let explicitly_mutated = BTreeSet::from(["kind-file".to_string(), "kind-folder".to_string()]);
    daemon
        .import_visible_root_with_tombstone_base_and_paths(
            deleted,
            Some(reprojected),
            Some(&explicitly_mutated),
        )
        .await
        .unwrap();
    let after_delete = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap()
        .root_cid;
    assert!(
        daemon
            .tree()
            .resolve(&after_delete, "kind-folder")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        daemon
            .tree()
            .resolve(&after_delete, "kind-file")
            .await
            .unwrap()
            .is_none()
    );

    let recreated_child = daemon
        .tree()
        .put_file(b"explicitly recreated directory child")
        .await
        .unwrap();
    let recreated_directory = daemon
        .tree()
        .put_directory(vec![
            hashtree_core::DirEntry::from_cid("recreated.txt", &recreated_child.0)
                .with_size(recreated_child.1)
                .with_link_type(hashtree_core::LinkType::File),
        ])
        .await
        .unwrap();
    let recreated = daemon
        .tree()
        .set_entry(
            &after_delete,
            &[],
            "kind-folder",
            &recreated_directory,
            0,
            hashtree_core::LinkType::Dir,
        )
        .await
        .unwrap();
    let recreated_file = daemon
        .tree()
        .put_file(b"explicitly recreated file")
        .await
        .unwrap();
    let recreated = daemon
        .tree()
        .set_entry(
            &recreated,
            &[],
            "kind-file",
            &recreated_file.0,
            recreated_file.1,
            hashtree_core::LinkType::File,
        )
        .await
        .unwrap();
    daemon
        .import_visible_root_with_tombstone_base_and_paths(
            recreated,
            Some(after_delete),
            Some(&explicitly_mutated),
        )
        .await
        .unwrap();
    let after_recreate = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap()
        .root_cid;
    assert_eq!(
        visible_file_bytes(&daemon, &after_recreate, "kind-folder/recreated.txt").await,
        b"explicitly recreated directory child"
    );
    assert_eq!(
        visible_file_bytes(&daemon, &after_recreate, "kind-file").await,
        b"explicitly recreated file"
    );
    assert!(
        daemon
            .tree()
            .resolve(&after_recreate, &folder_conflict)
            .await
            .unwrap()
            .is_none(),
        "ordinary delete/recreate must not recover the old directory conflict"
    );
    assert!(
        daemon
            .tree()
            .resolve(&after_recreate, &file_conflict)
            .await
            .unwrap()
            .is_none(),
        "ordinary delete/recreate must not recover the old file conflict"
    );
}
