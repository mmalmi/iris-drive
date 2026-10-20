use std::sync::Arc;

use hashtree_core::{DirEntry, HashTree, HashTreeConfig, MemoryStore};
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
