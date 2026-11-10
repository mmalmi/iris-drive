#[allow(clippy::wildcard_imports)]
use super::*;
use hashtree_core::{HashTreeConfig, MemoryStore};
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn mount_import_stores_each_directory_once() {
    const FILES: usize = 300;
    let store = Arc::new(MemoryStore::new());
    let tree = HashTree::new(HashTreeConfig::new(store.clone()));

    let source = tempdir().unwrap();
    std::fs::create_dir(source.path().join("flat")).unwrap();
    for index in 0..FILES {
        let name = format!("flat/{index:04}.txt");
        std::fs::write(source.path().join(name), format!("file {index}")).unwrap();
    }
    // This device already published every file, and the mounted view shows
    // exactly that root.
    let published = index_dir(&tree, source.path()).await.unwrap();
    let (added, size) = tree.put(b"added through the mount").await.unwrap();
    let edited = tree
        .set_entry(
            &published,
            &["flat"],
            "added.txt",
            &added,
            size,
            LinkType::Blob,
        )
        .await
        .unwrap();

    let blocks_before = store.size();
    let delta = local_visible_root_for_mount_import(
        &tree,
        &edited,
        Some(&published),
        &published,
        Some(&published),
        None,
    )
    .await
    .unwrap();
    let blocks_written = store.size() - blocks_before;

    assert_eq!(delta.root, edited);
    assert!(delta.tombstone_paths.is_empty());
    // The root and `flat` (as fan-out pages at most), not one rewritten
    // directory per carried-over file.
    assert!(
        blocks_written <= 8,
        "importing one new file next to {FILES} stored {blocks_written} new blocks"
    );
}
