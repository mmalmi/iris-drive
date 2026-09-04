use super::*;

mod multi_app_key;
mod ordinary_delete;

struct KindReplacementFixture {
    _config_dir: tempfile::TempDir,
    remote_app_key: String,
    daemon: Daemon,
    projected_root: Cid,
}

impl KindReplacementFixture {
    async fn from_remote_source(source_dir: &Path) -> Self {
        let config_dir = tempdir().unwrap();
        let mut account = init_config_with_account(config_dir.path());
        let remote_app_key =
            Identity::generate(config_dir.path().join("remote-kind.key")).pubkey_hex();
        approve_remote_app_key(
            config_dir.path(),
            &mut account,
            &remote_app_key,
            "remote-kind",
        );

        let daemon = Daemon::open(config_dir.path()).unwrap();
        let remote_meta = DriveRootMeta {
            schema: DriveRootMeta::SCHEMA,
            drive_id: PRIMARY_DRIVE_ID.into(),
            app_key_pubkey: remote_app_key.clone(),
            app_key_seq: 1,
            dck_generation: 1,
            local_only: false,
            parents: Vec::new(),
            observed: BTreeMap::new(),
            created_at: 100,
        };
        let remote_root = crate::indexer::index_dir_with_history_and_meta(
            daemon.tree(),
            source_dir,
            None,
            remote_meta.created_at,
            Some(&remote_meta),
        )
        .await
        .unwrap();
        drop(daemon);

        let mut config = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
        let mut drive = config.drive(PRIMARY_DRIVE_ID).unwrap().clone();
        drive.app_key_roots.insert(
            remote_app_key.clone(),
            AppKeyRootRef::from_meta(
                remote_root.to_string(),
                remote_meta.created_at,
                &remote_meta,
            ),
        );
        config.upsert_drive(drive);
        config.save(config_path_in(config_dir.path())).unwrap();

        let daemon = Daemon::open(config_dir.path()).unwrap();
        let projected_root = crate::primary_merged_root(daemon.tree(), daemon.config())
            .await
            .unwrap()
            .root_cid;
        Self {
            _config_dir: config_dir,
            remote_app_key,
            daemon,
            projected_root,
        }
    }

    async fn import_edited_source(&mut self, source_dir: &Path) -> Cid {
        let edited_root = crate::indexer::index_dir(self.daemon.tree(), source_dir)
            .await
            .unwrap();
        self.daemon
            .import_visible_root_with_tombstone_base(edited_root, Some(self.projected_root.clone()))
            .await
            .unwrap();
        let projected_root = crate::primary_merged_root(self.daemon.tree(), self.daemon.config())
            .await
            .unwrap()
            .root_cid;
        self.projected_root.clone_from(&projected_root);
        projected_root
    }

    async fn local_tombstones(&self) -> Vec<crate::merge::AppKeyTombstone> {
        let account = self.daemon.config().profile.as_ref().unwrap();
        let local_root = self
            .daemon
            .config()
            .drive(PRIMARY_DRIVE_ID)
            .unwrap()
            .app_key_roots
            .get(&account.app_key_pubkey)
            .unwrap();
        let (_, tombstones) = crate::merge::walk_app_key_tree(
            self.daemon.tree(),
            &Cid::parse(&local_root.root_cid).unwrap(),
        )
        .await
        .unwrap();
        tombstones
    }

    async fn local_tombstone_paths(&self) -> Vec<String> {
        self.local_tombstones()
            .await
            .into_iter()
            .map(|tombstone| tombstone.path)
            .collect()
    }

    async fn local_path_kind_replacements(&self) -> Option<BTreeMap<String, i64>> {
        let account = self.daemon.config().profile.as_ref().unwrap();
        let local_root = self
            .daemon
            .config()
            .drive(PRIMARY_DRIVE_ID)
            .unwrap()
            .app_key_roots
            .get(&account.app_key_pubkey)
            .unwrap();
        crate::indexer::read_path_kind_replacements(
            self.daemon.tree(),
            &Cid::parse(&local_root.root_cid).unwrap(),
        )
        .await
        .unwrap()
    }
}

#[tokio::test]
async fn provider_directory_to_file_replacement_preserves_complete_directory_conflict() {
    let remote_source = tempdir().unwrap();
    std::fs::create_dir_all(remote_source.path().join("draft/nested/empty")).unwrap();
    std::fs::write(
        remote_source.path().join("draft/old.txt"),
        b"remote directory bytes",
    )
    .unwrap();
    let mut fixture = KindReplacementFixture::from_remote_source(remote_source.path()).await;
    let old_entry = visible_entry(&fixture.daemon, &fixture.projected_root, "draft/old.txt").await;

    let edited_source = tempdir().unwrap();
    std::fs::write(edited_source.path().join("draft"), b"new replacement file").unwrap();
    let merged_root = fixture.import_edited_source(edited_source.path()).await;
    assert!(
        fixture
            .local_tombstone_paths()
            .await
            .iter()
            .any(|path| path == "draft"),
        "the production provider import must author an exact-path kind barrier"
    );
    assert!(
        !fixture
            .local_tombstone_paths()
            .await
            .iter()
            .any(|path| path == "draft/old.txt"),
        "new replacements must preserve the subtree without descendant tombstones"
    );
    let replacements = fixture
        .local_path_kind_replacements()
        .await
        .expect("new provider roots are explicitly role-aware");
    assert_eq!(replacements.len(), 1);
    assert!(replacements.contains_key("draft"));
    assert_eq!(
        replacements["draft"],
        fixture
            .local_tombstones()
            .await
            .into_iter()
            .find(|tombstone| tombstone.path == "draft")
            .unwrap()
            .tombstoned_at
    );

    assert_eq!(
        visible_file_bytes(&fixture.daemon, &merged_root, "draft").await,
        b"new replacement file"
    );
    let conflict_dir = format!("draft (conflict from {})", fixture.remote_app_key);
    let preserved_path = format!("{conflict_dir}/old.txt");
    assert_eq!(
        visible_file_bytes(&fixture.daemon, &merged_root, &preserved_path).await,
        b"remote directory bytes"
    );
    assert_eq!(
        visible_entry(&fixture.daemon, &merged_root, &preserved_path)
            .await
            .meta,
        old_entry.meta,
        "the conflict copy must retain whole-file hash and modification metadata"
    );
    let nested_empty = fixture
        .daemon
        .tree()
        .resolve(&merged_root, &format!("{conflict_dir}/nested/empty"))
        .await
        .unwrap()
        .expect("nested empty directory survives the production provider import");
    assert!(fixture.daemon.tree().is_dir(&nested_empty).await.unwrap());
}

#[tokio::test]
async fn provider_directory_to_file_preserves_all_concurrent_child_versions() {
    let config_dir = tempdir().unwrap();
    let mut account = init_config_with_account(config_dir.path());
    let remote_a = Identity::generate(config_dir.path().join("remote-a.key")).pubkey_hex();
    let remote_b = Identity::generate(config_dir.path().join("remote-b.key")).pubkey_hex();
    approve_remote_app_key(config_dir.path(), &mut account, &remote_a, "remote-a");
    approve_remote_app_key(config_dir.path(), &mut account, &remote_b, "remote-b");

    let daemon = Daemon::open(config_dir.path()).unwrap();
    let source_a = tempdir().unwrap();
    std::fs::create_dir(source_a.path().join("draft")).unwrap();
    std::fs::write(
        source_a.path().join("draft/note.txt"),
        b"concurrent version A",
    )
    .unwrap();
    let meta_a = remote_root_meta(&remote_a, 100);
    let root_a = crate::indexer::index_dir_with_history_and_meta(
        daemon.tree(),
        source_a.path(),
        None,
        meta_a.created_at,
        Some(&meta_a),
    )
    .await
    .unwrap();

    let source_b = tempdir().unwrap();
    std::fs::create_dir(source_b.path().join("draft")).unwrap();
    std::fs::write(
        source_b.path().join("draft/note.txt"),
        b"concurrent version B",
    )
    .unwrap();
    let meta_b = remote_root_meta(&remote_b, 101);
    let root_b = crate::indexer::index_dir_with_history_and_meta(
        daemon.tree(),
        source_b.path(),
        None,
        meta_b.created_at,
        Some(&meta_b),
    )
    .await
    .unwrap();
    drop(daemon);

    let mut config = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
    let mut drive = config.drive(PRIMARY_DRIVE_ID).unwrap().clone();
    drive.app_key_roots.insert(
        remote_a.clone(),
        AppKeyRootRef::from_meta(root_a.to_string(), meta_a.created_at, &meta_a),
    );
    drive.app_key_roots.insert(
        remote_b.clone(),
        AppKeyRootRef::from_meta(root_b.to_string(), meta_b.created_at, &meta_b),
    );
    config.upsert_drive(drive);
    config.save(config_path_in(config_dir.path())).unwrap();

    let mut daemon = Daemon::open(config_dir.path()).unwrap();
    let projected_root = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap()
        .root_cid;
    let initial_draft = daemon
        .tree()
        .resolve(&projected_root, "draft")
        .await
        .unwrap()
        .expect("the concurrent directory versions are initially visible");
    let mut initial_contents = Vec::new();
    for entry in daemon.tree().list_directory(&initial_draft).await.unwrap() {
        if entry.link_type == hashtree_core::LinkType::Dir {
            continue;
        }
        initial_contents.push(
            daemon
                .tree()
                .get(
                    &Cid {
                        hash: entry.hash,
                        key: entry.key,
                    },
                    None,
                )
                .await
                .unwrap()
                .unwrap(),
        );
    }
    initial_contents.sort();
    assert_eq!(
        initial_contents,
        vec![
            b"concurrent version A".to_vec(),
            b"concurrent version B".to_vec()
        ]
    );

    let edited_source = tempdir().unwrap();
    std::fs::write(edited_source.path().join("draft"), b"replacement file").unwrap();
    let edited_root = crate::indexer::index_dir(daemon.tree(), edited_source.path())
        .await
        .unwrap();
    daemon
        .import_visible_root_with_tombstone_base(edited_root, Some(projected_root))
        .await
        .unwrap();
    let merged_root = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap()
        .root_cid;

    assert_eq!(
        visible_file_bytes(&daemon, &merged_root, "draft").await,
        b"replacement file"
    );
    let conflict_dir = format!("draft (conflict from {remote_b})");
    let conflict_root = daemon
        .tree()
        .resolve(&merged_root, &conflict_dir)
        .await
        .unwrap()
        .expect("the replaced directory remains visible as a conflict");
    let mut preserved_contents = Vec::new();
    for entry in daemon.tree().list_directory(&conflict_root).await.unwrap() {
        if entry.link_type == hashtree_core::LinkType::Dir {
            continue;
        }
        preserved_contents.push(
            daemon
                .tree()
                .get(
                    &Cid {
                        hash: entry.hash,
                        key: entry.key,
                    },
                    None,
                )
                .await
                .unwrap()
                .unwrap(),
        );
    }
    preserved_contents.sort();
    assert_eq!(
        preserved_contents,
        vec![
            b"concurrent version A".to_vec(),
            b"concurrent version B".to_vec()
        ],
        "every version visible before the kind replacement remains visible afterward"
    );
}

#[tokio::test]
async fn provider_directory_to_file_does_not_revive_previously_deleted_child() {
    let remote_source = tempdir().unwrap();
    std::fs::create_dir(remote_source.path().join("draft")).unwrap();
    std::fs::write(
        remote_source.path().join("draft/keep.txt"),
        b"visible at replacement",
    )
    .unwrap();
    std::fs::write(
        remote_source.path().join("draft/deleted.txt"),
        b"deleted before replacement",
    )
    .unwrap();
    let mut fixture = KindReplacementFixture::from_remote_source(remote_source.path()).await;

    let first_edit = tempdir().unwrap();
    std::fs::create_dir(first_edit.path().join("draft")).unwrap();
    std::fs::write(
        first_edit.path().join("draft/keep.txt"),
        b"visible at replacement",
    )
    .unwrap();
    let after_delete = fixture.import_edited_source(first_edit.path()).await;
    assert_eq!(
        visible_file_bytes(&fixture.daemon, &after_delete, "draft/keep.txt").await,
        b"visible at replacement"
    );
    assert!(
        fixture
            .daemon
            .tree()
            .resolve(&after_delete, "draft/deleted.txt")
            .await
            .unwrap()
            .is_none()
    );
    let deleted_at = fixture
        .local_tombstones()
        .await
        .into_iter()
        .find(|tombstone| tombstone.path == "draft/deleted.txt")
        .expect("first provider edit tombstones deleted child")
        .tombstoned_at;

    let second_edit = tempdir().unwrap();
    std::fs::write(second_edit.path().join("draft"), b"replacement file").unwrap();
    let merged_root = fixture.import_edited_source(second_edit.path()).await;
    let tombstones = fixture.local_tombstones().await;
    let replacement_deleted_at = tombstones
        .iter()
        .find(|tombstone| tombstone.path == "draft")
        .expect("kind replacement authors an exact-path barrier")
        .tombstoned_at;
    let local_app_key = &fixture
        .daemon
        .config()
        .profile
        .as_ref()
        .unwrap()
        .app_key_pubkey;
    let replacement_published_at = fixture
        .daemon
        .config()
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots
        .get(local_app_key)
        .unwrap()
        .published_at;
    assert!(
        replacement_deleted_at > deleted_at,
        "carried and newly-created tombstones have distinct causal import times"
    );
    assert_eq!(replacement_deleted_at, replacement_published_at);
    assert!(
        tombstones
            .iter()
            .all(|tombstone| tombstone.path != "draft/keep.txt"),
        "the exact barrier replaces fresh descendant tombstones"
    );

    assert_eq!(
        visible_file_bytes(&fixture.daemon, &merged_root, "draft").await,
        b"replacement file"
    );
    let conflict_dir = format!("draft (conflict from {})", fixture.remote_app_key);
    assert_eq!(
        visible_file_bytes(
            &fixture.daemon,
            &merged_root,
            &format!("{conflict_dir}/keep.txt"),
        )
        .await,
        b"visible at replacement"
    );
    assert!(
        fixture
            .daemon
            .tree()
            .resolve(&merged_root, &format!("{conflict_dir}/deleted.txt"))
            .await
            .unwrap()
            .is_none(),
        "a carried historical tombstone must not revive content deleted before kind replacement"
    );
}

#[tokio::test]
async fn provider_directory_to_file_does_not_revive_previously_deleted_empty_directory() {
    let remote_source = tempdir().unwrap();
    std::fs::create_dir_all(remote_source.path().join("draft/keep-empty")).unwrap();
    std::fs::create_dir_all(remote_source.path().join("draft/deleted-empty")).unwrap();
    let mut fixture = KindReplacementFixture::from_remote_source(remote_source.path()).await;

    let first_edit = tempdir().unwrap();
    std::fs::create_dir_all(first_edit.path().join("draft/keep-empty")).unwrap();
    let after_delete = fixture.import_edited_source(first_edit.path()).await;
    let keep_empty = fixture
        .daemon
        .tree()
        .resolve(&after_delete, "draft/keep-empty")
        .await
        .unwrap()
        .expect("the surviving empty directory remains visible");
    assert!(fixture.daemon.tree().is_dir(&keep_empty).await.unwrap());
    assert!(
        fixture
            .daemon
            .tree()
            .resolve(&after_delete, "draft/deleted-empty")
            .await
            .unwrap()
            .is_none()
    );
    let deleted_at = fixture
        .local_tombstones()
        .await
        .into_iter()
        .find(|tombstone| tombstone.path == "draft/deleted-empty")
        .expect("first provider edit tombstones deleted empty directory")
        .tombstoned_at;

    let second_edit = tempdir().unwrap();
    std::fs::write(second_edit.path().join("draft"), b"replacement file").unwrap();
    let merged_root = fixture.import_edited_source(second_edit.path()).await;
    let tombstones = fixture.local_tombstones().await;
    let replacement_deleted_at = tombstones
        .iter()
        .find(|tombstone| tombstone.path == "draft")
        .expect("kind replacement authors an exact-path barrier")
        .tombstoned_at;
    assert!(replacement_deleted_at > deleted_at);
    assert!(
        tombstones
            .iter()
            .all(|tombstone| tombstone.path != "draft/keep-empty")
    );

    let conflict_dir = format!("draft (conflict from {})", fixture.remote_app_key);
    let preserved_empty = fixture
        .daemon
        .tree()
        .resolve(&merged_root, &format!("{conflict_dir}/keep-empty"))
        .await
        .unwrap()
        .expect("the empty directory visible at replacement is preserved");
    assert!(
        fixture
            .daemon
            .tree()
            .is_dir(&preserved_empty)
            .await
            .unwrap()
    );
    assert!(
        fixture
            .daemon
            .tree()
            .resolve(&merged_root, &format!("{conflict_dir}/deleted-empty"))
            .await
            .unwrap()
            .is_none(),
        "a carried directory tombstone must not revive an empty directory deleted before replacement"
    );
}

#[tokio::test]
async fn provider_file_to_directory_replacement_preserves_file_conflict() {
    let remote_source = tempdir().unwrap();
    std::fs::write(remote_source.path().join("draft"), b"remote file bytes").unwrap();
    let mut fixture = KindReplacementFixture::from_remote_source(remote_source.path()).await;
    let old_entry = visible_entry(&fixture.daemon, &fixture.projected_root, "draft").await;

    let edited_source = tempdir().unwrap();
    std::fs::create_dir_all(edited_source.path().join("draft/nested/empty")).unwrap();
    std::fs::write(
        edited_source.path().join("draft/new.txt"),
        b"new directory bytes",
    )
    .unwrap();
    let merged_root = fixture.import_edited_source(edited_source.path()).await;
    assert!(
        fixture
            .local_tombstone_paths()
            .await
            .iter()
            .any(|path| path == "draft"),
        "the production provider import must exercise its real file tombstone"
    );

    assert_eq!(
        visible_file_bytes(&fixture.daemon, &merged_root, "draft/new.txt").await,
        b"new directory bytes"
    );
    let conflict_path = format!("draft (conflict from {})", fixture.remote_app_key);
    assert_eq!(
        visible_file_bytes(&fixture.daemon, &merged_root, &conflict_path).await,
        b"remote file bytes"
    );
    assert_eq!(
        visible_entry(&fixture.daemon, &merged_root, &conflict_path)
            .await
            .meta,
        old_entry.meta,
        "the conflict copy must retain whole-file hash and modification metadata"
    );
    let nested_empty = fixture
        .daemon
        .tree()
        .resolve(&merged_root, "draft/nested/empty")
        .await
        .unwrap()
        .expect("new nested empty directory remains visible");
    assert!(fixture.daemon.tree().is_dir(&nested_empty).await.unwrap());
}

#[tokio::test]
async fn provider_recreated_empty_directory_keeps_its_carried_barrier() {
    let remote_source = tempdir().unwrap();
    std::fs::create_dir(remote_source.path().join("draft")).unwrap();
    let mut fixture = KindReplacementFixture::from_remote_source(remote_source.path()).await;

    let deleted_source = tempdir().unwrap();
    let after_delete = fixture.import_edited_source(deleted_source.path()).await;
    assert!(
        fixture
            .daemon
            .tree()
            .resolve(&after_delete, "draft")
            .await
            .unwrap()
            .is_none()
    );
    let deleted_at = fixture
        .local_tombstones()
        .await
        .into_iter()
        .find(|tombstone| tombstone.path == "draft")
        .expect("deleting the empty directory authors a barrier")
        .tombstoned_at;

    let recreated_source = tempdir().unwrap();
    std::fs::create_dir(recreated_source.path().join("draft")).unwrap();
    let after_recreate = fixture.import_edited_source(recreated_source.path()).await;
    let recreated = fixture
        .daemon
        .tree()
        .resolve(&after_recreate, "draft")
        .await
        .unwrap()
        .expect("a root's own visible directory wins over its carried marker");
    assert!(fixture.daemon.tree().is_dir(&recreated).await.unwrap());
    assert_eq!(
        fixture
            .local_tombstones()
            .await
            .into_iter()
            .find(|tombstone| tombstone.path == "draft")
            .expect("the old barrier remains in the recreated root")
            .tombstoned_at,
        deleted_at
    );
    assert_eq!(
        fixture.local_path_kind_replacements().await,
        Some(BTreeMap::new()),
        "ordinary delete/recreate retains the barrier but clears replacement role"
    );
}

#[tokio::test]
async fn provider_unrelated_later_edit_retains_active_replacement_role_and_conflict() {
    let remote_source = tempdir().unwrap();
    std::fs::create_dir(remote_source.path().join("draft")).unwrap();
    std::fs::write(
        remote_source.path().join("draft/old.txt"),
        b"remote directory bytes",
    )
    .unwrap();
    let mut fixture = KindReplacementFixture::from_remote_source(remote_source.path()).await;

    let replacement_source = tempdir().unwrap();
    std::fs::write(
        replacement_source.path().join("draft"),
        b"replacement bytes",
    )
    .unwrap();
    fixture
        .import_edited_source(replacement_source.path())
        .await;
    let initial_generation = fixture.local_path_kind_replacements().await.unwrap()["draft"];

    let conflict_dir = format!("draft (conflict from {})", fixture.remote_app_key);
    let (unrelated, unrelated_size) = fixture.daemon.tree().put_file(b"later edit").await.unwrap();
    let edited_root = fixture
        .daemon
        .tree()
        .set_entry(
            &fixture.projected_root,
            &[],
            "unrelated.txt",
            &unrelated,
            unrelated_size,
            hashtree_core::LinkType::File,
        )
        .await
        .unwrap();
    fixture
        .daemon
        .import_visible_root_with_tombstone_base(edited_root, Some(fixture.projected_root.clone()))
        .await
        .unwrap();
    let merged_root = crate::primary_merged_root(fixture.daemon.tree(), fixture.daemon.config())
        .await
        .unwrap()
        .root_cid;
    fixture.projected_root.clone_from(&merged_root);

    assert_eq!(
        fixture.local_path_kind_replacements().await.unwrap(),
        BTreeMap::from([("draft".to_string(), initial_generation)]),
        "same-kind and unrelated edits retain the durable path role generation"
    );
    assert_eq!(
        visible_file_bytes(
            &fixture.daemon,
            &merged_root,
            &format!("{conflict_dir}/old.txt"),
        )
        .await,
        b"remote directory bytes"
    );
    assert_eq!(
        visible_file_bytes(&fixture.daemon, &merged_root, "unrelated.txt").await,
        b"later edit"
    );
}

#[tokio::test]
async fn provider_tracks_two_active_path_kind_replacements() {
    let remote_source = tempdir().unwrap();
    for path in ["alpha", "beta"] {
        std::fs::create_dir(remote_source.path().join(path)).unwrap();
        std::fs::write(
            remote_source.path().join(path).join("old.txt"),
            format!("old {path}"),
        )
        .unwrap();
    }
    let mut fixture = KindReplacementFixture::from_remote_source(remote_source.path()).await;

    let edited_source = tempdir().unwrap();
    std::fs::write(edited_source.path().join("alpha"), b"new alpha").unwrap();
    std::fs::write(edited_source.path().join("beta"), b"new beta").unwrap();
    let merged_root = fixture.import_edited_source(edited_source.path()).await;
    let replacements = fixture.local_path_kind_replacements().await.unwrap();
    assert_eq!(replacements.len(), 2);
    assert_eq!(replacements["alpha"], replacements["beta"]);

    for path in ["alpha", "beta"] {
        assert_eq!(
            visible_file_bytes(
                &fixture.daemon,
                &merged_root,
                &format!("{path} (conflict from {})/old.txt", fixture.remote_app_key),
            )
            .await,
            format!("old {path}").as_bytes()
        );
    }
}

#[tokio::test]
async fn provider_delete_recreate_after_kind_replacement_does_not_revive_old_subtree() {
    let remote_source = tempdir().unwrap();
    std::fs::create_dir(remote_source.path().join("draft")).unwrap();
    std::fs::write(
        remote_source.path().join("draft/old.txt"),
        b"must stay deleted",
    )
    .unwrap();
    let mut fixture = KindReplacementFixture::from_remote_source(remote_source.path()).await;

    let replacement_source = tempdir().unwrap();
    std::fs::write(
        replacement_source.path().join("draft"),
        b"first replacement",
    )
    .unwrap();
    fixture
        .import_edited_source(replacement_source.path())
        .await;
    assert!(
        fixture
            .local_path_kind_replacements()
            .await
            .unwrap()
            .contains_key("draft")
    );

    let deleted_source = tempdir().unwrap();
    let after_delete = fixture.import_edited_source(deleted_source.path()).await;
    assert!(
        fixture
            .daemon
            .tree()
            .resolve(&after_delete, "draft")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fixture.local_path_kind_replacements().await,
        Some(BTreeMap::new())
    );

    let recreated_source = tempdir().unwrap();
    std::fs::write(recreated_source.path().join("draft"), b"recreated file").unwrap();
    let after_recreate = fixture.import_edited_source(recreated_source.path()).await;
    assert_eq!(
        visible_file_bytes(&fixture.daemon, &after_recreate, "draft").await,
        b"recreated file"
    );
    assert!(
        fixture
            .daemon
            .tree()
            .resolve(
                &after_recreate,
                &format!("draft (conflict from {})", fixture.remote_app_key),
            )
            .await
            .unwrap()
            .is_none(),
        "cleared replacement role turns the carried exact marker back into a deletion barrier"
    );
}

fn remote_root_meta(app_key_pubkey: &str, created_at: i64) -> DriveRootMeta {
    DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.into(),
        app_key_pubkey: app_key_pubkey.into(),
        app_key_seq: 1,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at,
    }
}

async fn visible_entry(daemon: &Daemon, root: &Cid, path: &str) -> hashtree_core::TreeEntry {
    let (parent_path, name) = path.rsplit_once('/').unwrap_or(("", path));
    let parent = if parent_path.is_empty() {
        root.clone()
    } else {
        daemon
            .tree()
            .resolve(root, parent_path)
            .await
            .unwrap()
            .expect("visible parent exists")
    };
    let entries = daemon.tree().list_directory(&parent).await.unwrap();
    let available = entries
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();
    entries
        .into_iter()
        .find(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("visible entry {path} exists; available: {available:?}"))
}

async fn visible_file_bytes(daemon: &Daemon, root: &Cid, path: &str) -> Vec<u8> {
    let entry = visible_entry(daemon, root, path).await;
    assert_ne!(entry.link_type, hashtree_core::LinkType::Dir);
    daemon
        .tree()
        .get(
            &Cid {
                hash: entry.hash,
                key: entry.key,
            },
            None,
        )
        .await
        .unwrap()
        .expect("visible file bytes exist")
}
