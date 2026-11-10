#[allow(clippy::wildcard_imports)]
use super::*;
use crate::config::{AppConfig, AppKeyRootRef, Drive};
use crate::indexer::{index_dir_with_history_and_meta, local_visible_root_for_mount_import};
use crate::merge::walk_app_key_tree;
use crate::profile::Profile;
use crate::root_meta::DriveRootMeta;
use hashtree_core::{HashTreeConfig, MemoryStore};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;
use tempfile::tempdir;

#[tokio::test]
async fn primary_merged_root_from_view_uses_supplied_projection() {
    let cfg_dir = tempdir().unwrap();
    let account = Profile::create(cfg_dir.path(), Some("provider-list-test".into())).unwrap();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());

    let first_source = tempdir().unwrap();
    std::fs::write(first_source.path().join("one.txt"), b"one").unwrap();
    let first_meta = test_meta(&account.state.app_key_pubkey, 1);
    let first_root =
        index_dir_with_history_and_meta(&tree, first_source.path(), None, 1, Some(&first_meta))
            .await
            .unwrap();

    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    upsert_root(
        &mut config,
        &account.state,
        first_root.to_string(),
        &first_meta,
    );
    let first_view = primary_merged_view(&tree, &config).await.unwrap();

    let second_source = tempdir().unwrap();
    std::fs::write(second_source.path().join("one.txt"), b"one").unwrap();
    std::fs::write(second_source.path().join("two.txt"), b"two").unwrap();
    let second_meta = test_meta(&account.state.app_key_pubkey, 2);
    let second_root =
        index_dir_with_history_and_meta(&tree, second_source.path(), None, 2, Some(&second_meta))
            .await
            .unwrap();
    upsert_root(
        &mut config,
        &account.state,
        second_root.to_string(),
        &second_meta,
    );

    let supplied = primary_merged_root_from_view(&tree, &config, &first_view)
        .await
        .unwrap();
    assert_eq!(
        visible_paths(&tree, &supplied.root_cid).await,
        vec!["one.txt"]
    );
    assert_eq!(supplied.file_count, 1);

    let recomputed = primary_merged_root(&tree, &config).await.unwrap();
    assert_eq!(
        visible_paths(&tree, &recomputed.root_cid).await,
        vec!["one.txt", "two.txt"]
    );
}

#[tokio::test]
async fn primary_merged_root_stores_each_directory_once() {
    const FILES: usize = 300;
    let cfg_dir = tempdir().unwrap();
    let account = Profile::create(cfg_dir.path(), Some("provider-list-test".into())).unwrap();
    let store = Arc::new(MemoryStore::new());
    let tree = HashTree::new(HashTreeConfig::new(store.clone()));

    let source = tempdir().unwrap();
    std::fs::create_dir(source.path().join("flat")).unwrap();
    for index in 0..FILES {
        let name = format!("flat/{index:04}.txt");
        std::fs::write(source.path().join(name), format!("file {index}")).unwrap();
    }
    let meta = test_meta(&account.state.app_key_pubkey, 1);
    let root = index_dir_with_history_and_meta(&tree, source.path(), None, 1, Some(&meta))
        .await
        .unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    upsert_root(&mut config, &account.state, root.to_string(), &meta);

    let view = primary_merged_view(&tree, &config).await.unwrap();
    let blocks_before = store.size();
    let merged = primary_merged_root_from_view(&tree, &config, &view)
        .await
        .unwrap();
    let blocks_written = store.size() - blocks_before;

    assert_eq!(merged.file_count, FILES);
    assert_eq!(visible_paths(&tree, &merged.root_cid).await.len(), FILES);
    // The root and `flat` (as fan-out pages at most), not one rewritten
    // directory per projected file.
    assert!(
        blocks_written <= 8,
        "projecting {FILES} files stored {blocks_written} new blocks"
    );
}

#[tokio::test]
async fn primary_merged_root_from_view_does_not_walk_sources_again() {
    let cfg_dir = tempdir().unwrap();
    let account = Profile::create(cfg_dir.path(), Some("provider-list-test".into())).unwrap();
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())));

    let source = tempdir().unwrap();
    std::fs::create_dir_all(source.path().join("docs/empty")).unwrap();
    std::fs::write(source.path().join("docs/a.txt"), b"a").unwrap();
    std::fs::write(source.path().join("top.txt"), b"top").unwrap();
    let meta = test_meta(&account.state.app_key_pubkey, 1);
    let root = index_dir_with_history_and_meta(&tree, source.path(), None, 1, Some(&meta))
        .await
        .unwrap();
    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    upsert_root(&mut config, &account.state, root.to_string(), &meta);

    let view = primary_merged_view(&tree, &config).await.unwrap();
    let expected = primary_merged_root_from_view(&tree, &config, &view)
        .await
        .unwrap();

    // A store holding none of the source blocks can still materialize the
    // root: everything it needs was gathered while building the view.
    let blank = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())));
    let from_cache = primary_merged_root_from_view(&blank, &config, &view)
        .await
        .unwrap();
    assert_eq!(from_cache.root_cid, expected.root_cid);

    // A view assembled by hand carries no walk and gets the same root.
    let by_hand = PrimaryMergedView {
        sources: SourceCache::default(),
        ..view.clone()
    };
    let rewalked = primary_merged_root_from_view(&tree, &config, &by_hand)
        .await
        .unwrap();
    assert_eq!(rewalked.root_cid, expected.root_cid);
    assert!(
        primary_merged_root_from_view(&blank, &config, &by_hand)
            .await
            .is_err()
    );
}

/// Prints wall time and store growth for the merged projection and the
/// mount-import delta on a larger drive. See `docs/EXPERIMENTS.md`.
#[tokio::test]
#[ignore = "manual scaling probe; run with --ignored --nocapture"]
async fn projection_and_mount_import_scaling_probe() {
    const LIVE_FILES: usize = 2_000;
    const DELETED_FILES: usize = 300;
    const DIRECTORIES: usize = 20;

    let cfg_dir = tempdir().unwrap();
    let account = Profile::create(cfg_dir.path(), Some("probe".into())).unwrap();
    let store = Arc::new(MemoryStore::new());
    let tree = HashTree::new(HashTreeConfig::new(store.clone()));

    let source = tempdir().unwrap();
    for directory in 0..DIRECTORIES {
        std::fs::create_dir(source.path().join(format!("d{directory:02}"))).unwrap();
    }
    std::fs::create_dir(source.path().join("gone")).unwrap();
    for index in 0..LIVE_FILES {
        let name = format!("d{:02}/f{index:05}.txt", index % DIRECTORIES);
        std::fs::write(source.path().join(name), format!("file {index}")).unwrap();
    }
    for index in 0..DELETED_FILES {
        std::fs::write(source.path().join(format!("gone/g{index:05}.txt")), b"gone").unwrap();
    }
    let first_meta = test_meta(&account.state.app_key_pubkey, 1);
    let first = index_dir_with_history_and_meta(&tree, source.path(), None, 1, Some(&first_meta))
        .await
        .unwrap();
    std::fs::remove_dir_all(source.path().join("gone")).unwrap();
    let meta = test_meta(&account.state.app_key_pubkey, 2);
    let started = Instant::now();
    let published =
        index_dir_with_history_and_meta(&tree, source.path(), Some(&first), 2, Some(&meta))
            .await
            .unwrap();
    println!("index_with_history ms={}", started.elapsed().as_millis());

    let mut config = AppConfig {
        profile: Some(account.state.clone()),
        ..AppConfig::default()
    };
    upsert_root(&mut config, &account.state, published.to_string(), &meta);

    let started = Instant::now();
    let view = primary_merged_view(&tree, &config).await.unwrap();
    println!(
        "merged_view ms={} files={} tombstones={}",
        started.elapsed().as_millis(),
        view.view.files.len(),
        view.view.tombstones.len()
    );

    let (blocks, bytes) = (store.size(), store.total_bytes());
    let started = Instant::now();
    let merged = primary_merged_root_from_view(&tree, &config, &view)
        .await
        .unwrap();
    println!(
        "merged_root ms={} new_blocks={} new_bytes={} cid={}",
        started.elapsed().as_millis(),
        store.size() - blocks,
        store.total_bytes() - bytes,
        merged.root_cid
    );
    assert_eq!(merged.file_count, LIVE_FILES);

    let (added, size) = tree.put(b"added through the mount").await.unwrap();
    let edited = tree
        .set_entry(
            &merged.root_cid,
            &["d00"],
            "added.txt",
            &added,
            size,
            LinkType::Blob,
        )
        .await
        .unwrap();
    let (blocks, bytes) = (store.size(), store.total_bytes());
    let started = Instant::now();
    let delta = local_visible_root_for_mount_import(
        &tree,
        &edited,
        Some(&published),
        &merged.root_cid,
        Some(&merged.root_cid),
        None,
    )
    .await
    .unwrap();
    println!(
        "mount_import ms={} new_blocks={} new_bytes={} cid={}",
        started.elapsed().as_millis(),
        store.size() - blocks,
        store.total_bytes() - bytes,
        delta.root
    );
}

fn test_meta(app_key_pubkey: &str, seq: u64) -> DriveRootMeta {
    DriveRootMeta {
        schema: DriveRootMeta::SCHEMA,
        drive_id: PRIMARY_DRIVE_ID.to_string(),
        app_key_pubkey: app_key_pubkey.to_owned(),
        app_key_seq: seq,
        dck_generation: 1,
        local_only: false,
        parents: Vec::new(),
        observed: BTreeMap::new(),
        created_at: i64::try_from(seq).unwrap(),
    }
}

fn upsert_root(
    config: &mut AppConfig,
    account: &crate::profile::ProfileState,
    root_cid: String,
    meta: &DriveRootMeta,
) {
    let mut drive = config
        .drive(PRIMARY_DRIVE_ID)
        .cloned()
        .unwrap_or_else(|| Drive::primary(account.root_scope_id()));
    drive.app_key_roots.insert(
        account.app_key_pubkey.clone(),
        AppKeyRootRef::from_meta(root_cid, i64::try_from(meta.app_key_seq).unwrap(), meta),
    );
    config.upsert_drive(drive);
}

async fn visible_paths<S: hashtree_core::Store>(tree: &HashTree<S>, root: &Cid) -> Vec<String> {
    let (files, _) = walk_app_key_tree(tree, root).await.unwrap();
    files.into_iter().map(|entry| entry.path).collect()
}
