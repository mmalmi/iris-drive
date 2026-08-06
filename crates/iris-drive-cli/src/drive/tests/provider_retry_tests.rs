use super::*;

#[test]
fn transient_store_reads_are_retryable() {
    for message in [
        "index: tree: Store error: IO error: The system cannot find the file specified. (os error 2)",
        "index: tree: Store error: IO error: No such file or directory (os error 2)",
        "index: tree: Missing chunk: abc123",
        "local store is missing provider root block abc123",
    ] {
        assert!(provider_retry::provider_import_error_message_is_retryable(
            message
        ));
    }
    assert!(!provider_retry::provider_import_error_message_is_retryable(
        "config: invalid json"
    ));
}

#[test]
fn merged_view_retry_reloads_a_superseded_root() {
    let config_dir = tempfile::tempdir().unwrap();
    let (_account, remote, _remote_meta) = init_config_with_remote_device(config_dir.path());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    runtime.block_on(async {
        let daemon = Daemon::open(config_dir.path()).unwrap();
        let missing_blob = Cid::public([42; 32]);
        let tombstone =
            hashtree_core::DirEntry::from_cid("gone.txt".to_string(), &missing_blob).with_size(10);
        let tombstones = daemon.tree().put_directory(vec![tombstone]).await.unwrap();
        let tombstones = hashtree_core::DirEntry::from_cid("tombstones".to_string(), &tombstones)
            .with_link_type(LinkType::Dir);
        let meta = daemon.tree().put_directory(vec![tombstones]).await.unwrap();
        let meta =
            hashtree_core::DirEntry::from_cid(iris_drive_core::merge::META_DIR.to_string(), &meta)
                .with_link_type(LinkType::Dir);
        let missing_root = daemon.tree().put_directory(vec![meta]).await.unwrap();
        let empty_root = daemon.tree().put_directory(Vec::new()).await.unwrap();
        let (current_hash, current_size) = daemon.tree().put(b"new").await.unwrap();
        let current_root = daemon
            .tree()
            .set_entry(
                &empty_root,
                &[],
                "current.txt",
                &current_hash,
                current_size,
                LinkType::Blob,
            )
            .await
            .unwrap();

        let mut config = AppConfig::load_or_default(config_path_in(config_dir.path())).unwrap();
        let mut drive = config.drive(PRIMARY_DRIVE_ID).unwrap().clone();
        drive.app_key_roots.insert(
            remote.clone(),
            AppKeyRootRef::legacy(missing_root.to_string(), 100, 1),
        );
        config.upsert_drive(drive.clone());
        config.save(config_path_in(config_dir.path())).unwrap();
        let mut stale_daemon = Daemon::open(config_dir.path()).unwrap();
        assert!(
            iris_drive_core::merge::walk_app_key_tree(stale_daemon.tree(), &missing_root)
                .await
                .is_err(),
            "test setup should expose a missing nested block",
        );

        drive.app_key_roots.insert(
            remote,
            AppKeyRootRef::legacy(current_root.to_string(), 101, 1),
        );
        config.upsert_drive(drive);
        config.save(config_path_in(config_dir.path())).unwrap();

        let (merged, _) = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            provider_retry::primary_merged_view_and_root_with_retry(&mut stale_daemon),
        )
        .await
        .expect("provider retry stayed pinned to a superseded root")
        .unwrap();
        let paths = merged
            .view
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(paths, vec!["current.txt"]);
    });
}

#[test]
fn retry_budget_covers_peer_root_warmup() {
    let retry_budget_ms: u64 = provider_retry::PROVIDER_IMPORT_RETRY_DELAYS_MS.iter().sum();

    assert!(retry_budget_ms >= 60_000);
    assert!(
        provider_retry::PROVIDER_IMPORT_RETRY_DELAYS_MS
            .iter()
            .all(|delay| *delay <= 16_000)
    );
}
