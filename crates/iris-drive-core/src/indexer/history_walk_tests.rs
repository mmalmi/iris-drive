#[allow(clippy::wildcard_imports)]
use super::*;
use async_trait::async_trait;
use hashtree_core::{Hash, HashTreeConfig, MemoryStore, StoreError};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tempfile::tempdir;

#[derive(Default)]
struct RecordingStore {
    inner: MemoryStore,
    reads: Mutex<HashMap<Hash, usize>>,
}

impl RecordingStore {
    fn reads(&self, hash: &Hash) -> usize {
        self.reads.lock().unwrap().get(hash).copied().unwrap_or(0)
    }
}

#[async_trait]
impl Store for RecordingStore {
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
        self.inner.put(hash, data).await
    }

    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        *self.reads.lock().unwrap().entry(*hash).or_default() += 1;
        self.inner.get(hash).await
    }

    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.inner.has(hash).await
    }

    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.inner.delete(hash).await
    }
}

#[tokio::test]
async fn history_layer_walks_previous_root_once() {
    let store = Arc::new(RecordingStore::default());
    let tree = HashTree::new(HashTreeConfig::new(store.clone()).public());

    let dir = tempdir().unwrap();
    std::fs::create_dir(dir.path().join("docs")).unwrap();
    std::fs::write(dir.path().join("docs/keep.txt"), b"keep").unwrap();
    std::fs::write(dir.path().join("docs/gone.txt"), b"gone").unwrap();
    let previous = index_dir(&tree, dir.path()).await.unwrap();
    let previous_docs = tree.resolve(&previous, "docs").await.unwrap().unwrap();

    std::fs::remove_file(dir.path().join("docs/gone.txt")).unwrap();
    let current = index_dir(&tree, dir.path()).await.unwrap();

    store.reads.lock().unwrap().clear();
    let layered = layer_history_and_meta_on_root(&tree, current, Some(&previous), 1_000, None)
        .await
        .unwrap();

    // Deriving the deletions and attaching history share one walk.
    assert_eq!(store.reads(&previous_docs.hash), 1);
    let (files, tombstones) = walk_app_key_tree(&tree, &layered).await.unwrap();
    assert_eq!(
        files.iter().map(|f| f.path.as_str()).collect::<Vec<_>>(),
        vec!["docs/keep.txt"]
    );
    assert_eq!(
        tombstones
            .iter()
            .map(|t| (t.path.as_str(), t.tombstoned_at))
            .collect::<Vec<_>>(),
        vec![("docs/gone.txt", 1_000)]
    );
}
