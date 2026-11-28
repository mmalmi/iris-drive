use std::sync::Arc;

use hashtree_core::{Cid, DirEntry, HashTree, HashTreeConfig, LinkType, MemoryStore, Store};

use super::{collect_live_sync_hashes, collect_live_sync_hashes_without_leaf_reads};

async fn check_restored_file(tree: &HashTree<MemoryStore>, root: &Cid, name: &str, bytes: &[u8]) {
    for hashes in [
        collect_live_sync_hashes(tree, root, 2).await.unwrap(),
        collect_live_sync_hashes_without_leaf_reads(tree, root, 2)
            .await
            .unwrap(),
    ] {
        let dest = Arc::new(MemoryStore::new());
        for hash in hashes {
            let block = tree.get_store().get(&hash).await.unwrap().unwrap();
            dest.put(hash, block).await.unwrap();
        }
        let restored = HashTree::new(HashTreeConfig::new(dest));
        let file = restored.resolve(root, name).await.unwrap().unwrap();
        assert_eq!(restored.get(&file, None).await.unwrap().unwrap(), bytes);
    }
}

#[tokio::test]
async fn byte_chunked_directory_restores_its_files_and_listing_chunks() {
    let tree = HashTree::new(
        HashTreeConfig::new(Arc::new(MemoryStore::new()))
            .public()
            .with_chunk_size(32),
    );
    let bytes = b"family photo";
    let (file, size) = tree.put(bytes).await.unwrap();
    let dir = tree
        .put_directory(vec![DirEntry::from_cid("photo", &file).with_size(size)])
        .await
        .unwrap();
    let manifest = tree.get_store().get(&dir.hash).await.unwrap().unwrap();
    assert!(manifest.len() > 32);
    let (chunked_dir, _) = tree.put(&manifest).await.unwrap();
    assert_eq!(
        tree.list_directory_required(&chunked_dir)
            .await
            .unwrap()
            .len(),
        1
    );
    check_restored_file(&tree, &chunked_dir, "photo", bytes).await;
}

#[tokio::test]
async fn historical_keyed_plaintext_directory_restores_its_files() {
    let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());
    let bytes = b"thumbnail";
    let (file, size) = tree.put(bytes).await.unwrap();
    let mut dir = tree
        .put_directory(vec![DirEntry::from_cid("photo", &file).with_size(size)])
        .await
        .unwrap();
    dir.key = Some([7; 32].into());
    assert_eq!(tree.list_directory_required(&dir).await.unwrap().len(), 1);
    check_restored_file(&tree, &dir, "photo", bytes).await;
}

#[tokio::test]
async fn blob_named_file_with_equal_manifest_and_payload_sizes_restores_its_chunks() {
    let tree = HashTree::new(
        HashTreeConfig::new(Arc::new(MemoryStore::new()))
            .public()
            .with_chunk_size(64),
    );
    // Two public links with short lengths use a fixed-size encoding.
    let (probe, _) = tree.put(&[3; 100]).await.unwrap();
    let length = tree
        .get_store()
        .get(&probe.hash)
        .await
        .unwrap()
        .unwrap()
        .len();
    assert!((65..=128).contains(&length));
    let bytes = (0u8..=127).take(length).collect::<Vec<_>>();
    let (file, size) = tree.put(&bytes).await.unwrap();
    assert_eq!(
        tree.get_store().blob_size(&file.hash).await.unwrap(),
        Some(size)
    );
    let root = tree
        .put_directory(vec![
            DirEntry::from_cid("photo", &file)
                .with_size(size)
                .with_link_type(LinkType::Blob),
        ])
        .await
        .unwrap();
    check_restored_file(&tree, &root, "photo", &bytes).await;
}

#[tokio::test]
async fn file_chunks_containing_manifest_bytes_remain_literal() {
    let store = Arc::new(MemoryStore::new());
    let tree = HashTree::new(HashTreeConfig::new(store.clone()).public());
    let (other, size) = tree.put(b"unrelated file").await.unwrap();
    let encoded = tree
        .put_directory(vec![DirEntry::from_cid("other", &other).with_size(size)])
        .await
        .unwrap();
    let mut bytes = store.get(&encoded.hash).await.unwrap().unwrap();
    let files = HashTree::new(
        HashTreeConfig::new(store)
            .public()
            .with_chunk_size(bytes.len()),
    );
    bytes.push(42);
    let (file, size) = files.put(&bytes).await.unwrap();
    let root = files
        .put_directory(vec![
            DirEntry::from_cid("file", &file)
                .with_size(size)
                .with_link_type(LinkType::File),
        ])
        .await
        .unwrap();
    let hashes = collect_live_sync_hashes(&files, &root, 2).await.unwrap();
    assert!(!hashes.contains(&other.hash));
    check_restored_file(&files, &root, "file", &bytes).await;
}
