//! Saving a local file into the tree chunk by chunk, as the share sheet,
//! Files app and photo backup do for large videos.

use std::sync::Arc;

use hashtree_core::{HashTree, HashTreeConfig, MemoryStore, sha256, to_hex};
use hashtree_provider::{HashTreeProviderFs, ProviderError, ProviderFs};

fn tree() -> Arc<HashTree<MemoryStore>> {
    Arc::new(HashTree::new(
        HashTreeConfig::new(Arc::new(MemoryStore::new())).public(),
    ))
}

/// Several megabytes that do not repeat, so the tree splits them into chunks.
fn video_bytes() -> Vec<u8> {
    let mut state: u32 = 7;
    (0..5 * 1024 * 1024 + 123)
        .map(|_| {
            state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
            (state >> 16).to_le_bytes()[0]
        })
        .collect()
}

#[tokio::test]
async fn a_local_file_is_stored_whole_with_its_hash() {
    let tr = tree();
    let fs = HashTreeProviderFs::fresh(tr.clone()).await.unwrap();
    let root = fs.root().await;
    let file = fs.create_file(&root, "Trip.mov").await.unwrap();
    let bytes = video_bytes();
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("Trip.mov");
    std::fs::write(&source, &bytes).unwrap();

    let stored = fs.write_file_from_path(&file.id, &source).await.unwrap();

    assert_eq!(stored, bytes.len() as u64);
    assert_eq!(fs.item(&file.id).await.unwrap().size, bytes.len() as u64);
    assert_eq!(
        fs.read(&file.id, 0, bytes.len() as u64).await.unwrap(),
        bytes
    );
    let listing = tr.list_directory(&fs.current_root().await).await.unwrap();
    let meta = listing
        .iter()
        .find(|entry| entry.name == "Trip.mov")
        .and_then(|entry| entry.meta.clone())
        .unwrap();
    assert_eq!(
        meta.get("whole_file_hash").and_then(|value| value.as_str()),
        Some(to_hex(&sha256(&bytes)).as_str())
    );
    assert!(meta.contains_key("modified_at"));
}

#[tokio::test]
async fn saving_over_a_file_replaces_all_of_it() {
    let fs = HashTreeProviderFs::fresh(tree()).await.unwrap();
    let root = fs.root().await;
    let file = fs.create_file(&root, "notes.txt").await.unwrap();
    fs.write(&file.id, 0, b"a much longer first version")
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("notes.txt");
    std::fs::write(&source, b"short").unwrap();

    fs.write_file_from_path(&file.id, &source).await.unwrap();

    assert_eq!(fs.read(&file.id, 0, 64).await.unwrap(), b"short");
}

#[tokio::test]
async fn a_folder_or_a_missing_source_is_refused() {
    let fs = HashTreeProviderFs::fresh(tree()).await.unwrap();
    let root = fs.root().await;
    let folder = fs.create_dir(&root, "Photos").await.unwrap();
    let file = fs.create_file(&root, "a.txt").await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("a.txt");
    std::fs::write(&source, b"x").unwrap();

    assert!(matches!(
        fs.write_file_from_path(&folder.id, &source).await,
        Err(ProviderError::IsDir)
    ));
    assert!(matches!(
        fs.write_file_from_path(&file.id, &dir.path().join("gone.txt"))
            .await,
        Err(ProviderError::Backend(_))
    ));
    assert_eq!(fs.read(&file.id, 0, 8).await.unwrap(), b"");
}
