use super::*;
use hashtree_core::LinkType;

#[tokio::test]
#[allow(clippy::too_many_lines)]
async fn verified_provider_snapshot_preserves_authorship_across_source_block_loss() {
    let dir = tempdir().unwrap();
    let mut owner = init_config_with_account(dir.path());
    let peer = Identity::generate(dir.path().join("peer")).pubkey_hex();
    approve_remote_app_key(dir.path(), &mut owner, &peer, "Other device");
    let local = tempdir().unwrap();
    std::fs::write(local.path().join("local"), b"old local").unwrap();
    let mut daemon = Daemon::open(dir.path()).unwrap();
    daemon.import_source_dir(local.path()).await.unwrap();
    let files = tempdir().unwrap();
    std::fs::write(files.path().join("remote"), b"authored elsewhere").unwrap();
    let meta = DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.into(),
        app_key_pubkey: peer.clone(),
        app_key_seq: 1,
        dck_generation: 1,
        local_only: false,
        parents: vec![],
        observed: BTreeMap::new(),
        created_at: 101,
    };
    let peer_root = crate::indexer::index_dir_with_history_and_meta(
        daemon.tree(),
        files.path(),
        None,
        101,
        Some(&meta),
    )
    .await
    .unwrap();
    let peer_ref = AppKeyRootRef::from_meta(peer_root.to_string(), 101, &meta);
    let mut config = daemon.config.clone();
    config.drives[0]
        .app_key_roots
        .insert(peer.clone(), peer_ref.clone());
    config.save(config_path_in(dir.path())).unwrap();
    let mut daemon = Daemon::open(dir.path()).unwrap();
    let canonical = daemon.provider_projection().await.unwrap().1.root_cid;
    daemon
        .materialize_primary_merged_root()
        .await
        .unwrap()
        .unwrap();
    let store = daemon.tree.get_store();
    let peer_bytes = store.get_sync(&peer_root.hash).unwrap().unwrap();
    store.delete_sync(&peer_root.hash).unwrap();

    let mut daemon = Daemon::open(dir.path()).unwrap();
    assert!(
        crate::primary_merged_root(daemon.tree(), daemon.config())
            .await
            .is_err()
    );
    let (_, visible, cached) = daemon.provider_projection().await.unwrap();
    assert!(cached);
    assert_eq!(visible.root_cid, canonical);
    let (new_file, _) = daemon.tree.put(b"local addition").await.unwrap();
    let changed = daemon
        .tree
        .set_entry(&canonical, &[], "remote", &new_file, 14, LinkType::File)
        .await
        .unwrap();
    assert!(
        daemon
            .import_visible_root_with_tombstone_base(changed, Some(canonical.clone()))
            .await
            .is_err()
    );
    let edited = daemon
        .tree
        .set_entry(&canonical, &[], "added", &new_file, 14, LinkType::File)
        .await
        .unwrap();
    let selected = BTreeSet::from(["added".into()]);
    let report = daemon
        .import_visible_root_with_tombstone_base_and_paths(
            edited.clone(),
            Some(canonical.clone()),
            Some(&selected),
        )
        .await
        .unwrap();
    let mut daemon = Daemon::open(dir.path()).unwrap();
    let (view, root, cached) = daemon.provider_projection().await.unwrap();
    assert!(cached);
    assert_eq!(root.root_cid, edited);
    assert_eq!(view.file_count(), 3);
    assert_eq!(daemon.config.drives[0].app_key_roots[&peer], peer_ref);
    let signed = Cid::parse(&report.root_cid).unwrap();
    let (authored, _) = crate::merge::walk_app_key_tree(daemon.tree(), &signed)
        .await
        .unwrap();
    assert_eq!(
        authored.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
        ["added", "local"]
    );
    assert!(
        daemon
            .import_visible_root_with_tombstone_base(edited.clone(), Some(canonical))
            .await
            .is_err()
    );

    // Any new remote head invalidates the cache, even when its sequence is equal.
    let mut divergent = daemon.config.clone();
    divergent.drives[0]
        .app_key_roots
        .get_mut(&peer)
        .unwrap()
        .root_cid = hashtree_core::to_hex(&[44; 32]);
    assert!(
        Daemon::open_with_config(dir.path(), divergent)
            .unwrap()
            .provider_projection()
            .await
            .is_err()
    );
    // A missing file block must not masquerade as a complete projection.
    let new_bytes = store.get_sync(&new_file.hash).unwrap().unwrap();
    store.delete_sync(&new_file.hash).unwrap();
    assert!(
        Daemon::open(dir.path())
            .unwrap()
            .provider_projection()
            .await
            .is_err()
    );
    store.put_sync(new_file.hash, &new_bytes).unwrap();

    // Once the original source reappears, the ordinary merge reproduces the
    // visible contents and their actual authors without consuming the cache.
    store.put_sync(peer_root.hash, &peer_bytes).unwrap();
    let daemon = Daemon::open(dir.path()).unwrap();
    let (view, verified, cached) = daemon.provider_projection().await.unwrap();
    assert!(!cached);
    assert_eq!(verified.root_cid, edited);
    assert!(
        view.view
            .files
            .iter()
            .any(|f| f.path == "remote" && f.source_app_key_pubkey == peer)
    );
}
