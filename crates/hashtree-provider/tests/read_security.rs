use std::sync::Arc;

use hashtree_core::{Cid, DirEntry, HashTree, HashTreeConfig, MemoryStore};
use hashtree_provider::{HashTreeProviderFs, ProviderError, ProviderFs};

#[tokio::test]
async fn encrypted_read_rejects_directory_size_larger_than_payload() {
    let tree = Arc::new(HashTree::new(HashTreeConfig::new(Arc::new(
        MemoryStore::new(),
    ))));
    let (cid, _) = tree.put(b"short").await.unwrap();
    assert!(cid.key.is_some());
    let root = tree
        .put_directory(vec![DirEntry::from_cid("bad.txt", &cid).with_size(100)])
        .await
        .unwrap();
    let provider = HashTreeProviderFs::open(tree, root).await.unwrap();

    // A peer controls the declared size independently of the encrypted bytes.
    let result = provider.read(&"bad.txt".into(), 50, 10).await;
    assert!(matches!(result, Err(ProviderError::Backend(_))));
}

#[tokio::test]
async fn read_large_requested_range_does_not_overflow() {
    for encrypted in [false, true] {
        let mut config = HashTreeConfig::new(Arc::new(MemoryStore::new()));
        config.encrypted = encrypted;
        let provider = HashTreeProviderFs::fresh(Arc::new(HashTree::new(config)))
            .await
            .unwrap();
        let item = provider
            .create_file(&String::new(), "file.txt")
            .await
            .unwrap();
        provider.write(&item.id, 0, b"contents").await.unwrap();

        assert_eq!(
            provider.read(&item.id, 1, u64::MAX).await.unwrap(),
            b"ontents"
        );
    }
}

#[tokio::test]
async fn encrypted_range_read_only_fetches_overlapping_chunks_and_checks_size() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use hashtree_core::{Hash, Store, StoreError};

    struct CountingStore {
        inner: MemoryStore,
        gets: AtomicUsize,
    }

    #[async_trait]
    impl Store for CountingStore {
        async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
            self.inner.put(hash, data).await
        }
        async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
            self.gets.fetch_add(1, Ordering::Relaxed);
            self.inner.get(hash).await
        }
        async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
            self.inner.has(hash).await
        }
        async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
            self.inner.delete(hash).await
        }
    }

    let store = Arc::new(CountingStore {
        inner: MemoryStore::new(),
        gets: AtomicUsize::new(0),
    });
    // 1 KiB chunks, 100 KiB encrypted file -> 100 leaf chunks.
    let tree = Arc::new(HashTree::new(
        HashTreeConfig::new(store.clone()).with_chunk_size(1024),
    ));
    let payload: Vec<u8> = (0..100 * 1024).map(|i| (i % 251) as u8).collect();
    let (cid, size) = tree.put(&payload).await.unwrap();
    assert!(cid.key.is_some());
    let root = tree
        .put_directory(vec![
            DirEntry::from_cid("video.bin", &cid)
                .with_size(size)
                .with_link_type(hashtree_core::LinkType::File),
            // Under-declared size must still be rejected.
            DirEntry::from_cid("short-declared.bin", &cid)
                .with_size(512)
                .with_link_type(hashtree_core::LinkType::File),
        ])
        .await
        .unwrap();
    let provider = HashTreeProviderFs::open(tree, root).await.unwrap();

    store.gets.store(0, Ordering::Relaxed);
    let slice = provider
        .read(&"video.bin".into(), 50 * 1024, 128)
        .await
        .unwrap();
    let gets = store.gets.load(Ordering::Relaxed);
    assert_eq!(slice, &payload[50 * 1024..50 * 1024 + 128]);
    assert!(
        gets <= 8,
        "128-byte range read should touch only path + 1 leaf chunk, did {gets} store reads"
    );

    let mismatch = provider.read(&"short-declared.bin".into(), 0, 128).await;
    assert!(matches!(mismatch, Err(ProviderError::Backend(_))));
}

#[tokio::test]
async fn stream_read_to_path_verifies_size_and_cleans_temp_files_on_error() {
    let store = Arc::new(MemoryStore::new());
    let tree = Arc::new(HashTree::new(
        HashTreeConfig::new(store.clone()).with_chunk_size(1024),
    ));
    let payload: Vec<u8> = (0..10 * 1024 + 37).map(|i| (i % 241) as u8).collect();
    let (cid, size) = tree.put(&payload).await.unwrap();
    let (empty_cid, _) = tree.put(b"").await.unwrap();
    let missing_cid = Cid {
        hash: [0xEE; 32],
        key: None,
    };
    let root = tree
        .put_directory(vec![
            DirEntry::from_cid("video.bin", &cid)
                .with_size(size)
                .with_link_type(hashtree_core::LinkType::File),
            DirEntry::from_cid("empty.bin", &empty_cid).with_size(0),
            DirEntry::from_cid("missing-empty.bin", &missing_cid).with_size(0),
            DirEntry::from_cid("bad-size.bin", &cid)
                .with_size(size - 1)
                .with_link_type(hashtree_core::LinkType::File),
        ])
        .await
        .unwrap();
    let provider = HashTreeProviderFs::open(tree, root).await.unwrap();

    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("nested/video.bin");
    let item = provider.read_file_to_path("video.bin", &out).await.unwrap();
    assert_eq!(item.size, size);
    assert_eq!(std::fs::read(&out).unwrap(), payload);

    let empty_out = dir.path().join("empty.bin");
    provider
        .read_file_to_path("empty.bin", &empty_out)
        .await
        .unwrap();
    assert!(std::fs::read(&empty_out).unwrap().is_empty());

    let missing_out = dir.path().join("missing-empty.bin");
    assert!(matches!(
        provider
            .read_file_to_path("missing-empty.bin", &missing_out)
            .await,
        Err(ProviderError::NotFound)
    ));
    assert!(!missing_out.exists());

    let bad_out = dir.path().join("bad.bin");
    assert!(matches!(
        provider.read_file_to_path("bad-size.bin", &bad_out).await,
        Err(ProviderError::Backend(_))
    ));
    assert!(!bad_out.exists());
    let temp_leftovers = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
        .count();
    assert_eq!(temp_leftovers, 0);
}
