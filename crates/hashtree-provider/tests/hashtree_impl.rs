//! Tests for `HashTreeProviderFs`: every `ProviderFs` mutation method
//! exercised against a real `HashTree<MemoryStore>`. Verifies that
//! mutations land in the htree (the new root is queryable), that anchors
//! advance, and that the diff API surfaces changes correctly.

use std::sync::Arc;
use std::sync::Mutex;

use hashtree_core::{Cid, HashTree, HashTreeConfig, MemoryStore, sha256, to_hex};
use hashtree_provider::{
    HashTreeProviderFs, ItemKind, PathChange, ProviderError, ProviderFs, RootObserver,
};

fn tree() -> Arc<HashTree<MemoryStore>> {
    Arc::new(HashTree::new(
        HashTreeConfig::new(Arc::new(MemoryStore::new())).public(),
    ))
}

async fn fresh() -> HashTreeProviderFs<MemoryStore> {
    HashTreeProviderFs::fresh(tree()).await.unwrap()
}

#[tokio::test]
async fn fresh_provider_root_lists_empty() {
    let fs = fresh().await;
    let root = fs.root().await;
    let listing = fs.read_dir(&root).await.unwrap();
    assert!(listing.is_empty());
}

#[tokio::test]
async fn create_then_read_file() {
    let fs = fresh().await;
    let root = fs.root().await;
    let f = fs.create_file(&root, "hello.txt").await.unwrap();
    fs.write(&f.id, 0, b"hi there").await.unwrap();
    let bytes = fs.read(&f.id, 0, 8).await.unwrap();
    assert_eq!(bytes, b"hi there");
    let item = fs.item(&f.id).await.unwrap();
    assert_eq!(item.size, 8);
    assert_eq!(item.kind, ItemKind::File);
}

#[tokio::test]
async fn file_write_stamps_modified_at_metadata() {
    let tr = tree();
    let fs = HashTreeProviderFs::fresh(tr.clone()).await.unwrap();
    let root = fs.root().await;
    let f = fs.create_file(&root, "hello.txt").await.unwrap();
    fs.write(&f.id, 0, b"hi there").await.unwrap();

    let listing = tr.list_directory(&fs.current_root().await).await.unwrap();
    let modified_at = listing
        .iter()
        .find(|entry| entry.name == "hello.txt")
        .and_then(|entry| entry.meta.as_ref())
        .and_then(|meta| meta.get("modified_at"))
        .and_then(|value| value.as_i64());
    assert!(
        modified_at.is_some_and(|value| value >= 946_684_800),
        "provider writes should stamp a non-epoch modified_at, got {modified_at:?}"
    );
}

#[tokio::test]
async fn create_dir_stamps_modified_at_metadata() {
    let tr = tree();
    let fs = HashTreeProviderFs::fresh(tr.clone()).await.unwrap();
    let root = fs.root().await;
    fs.create_dir(&root, "docs").await.unwrap();

    let listing = tr.list_directory(&fs.current_root().await).await.unwrap();
    let modified_at = listing
        .iter()
        .find(|entry| entry.name == "docs")
        .and_then(|entry| entry.meta.as_ref())
        .and_then(|meta| meta.get("modified_at"))
        .and_then(|value| value.as_i64());
    assert!(
        modified_at.is_some_and(|value| value >= 946_684_800),
        "provider directories should stamp a non-epoch modified_at, got {modified_at:?}"
    );
}

#[tokio::test]
async fn file_write_stamps_whole_file_hash_metadata() {
    let tr = tree();
    let fs = HashTreeProviderFs::fresh(tr.clone()).await.unwrap();
    let root = fs.root().await;
    let f = fs.create_file(&root, "hello.txt").await.unwrap();
    fs.write(&f.id, 0, b"hi there").await.unwrap();

    let listing = tr.list_directory(&fs.current_root().await).await.unwrap();
    let whole_file_hash = listing
        .iter()
        .find(|entry| entry.name == "hello.txt")
        .and_then(|entry| entry.meta.as_ref())
        .and_then(|meta| meta.get("whole_file_hash"))
        .and_then(|value| value.as_str());
    assert_eq!(whole_file_hash, Some(to_hex(&sha256(b"hi there")).as_str()));
}

#[tokio::test]
async fn create_in_subdir() {
    let fs = fresh().await;
    let root = fs.root().await;
    let sub = fs.create_dir(&root, "sub").await.unwrap();
    let f = fs.create_file(&sub.id, "a.txt").await.unwrap();
    fs.write(&f.id, 0, b"alpha").await.unwrap();
    let listing = fs.read_dir(&sub.id).await.unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].name, "a.txt");
    let bytes = fs.read(&f.id, 0, 5).await.unwrap();
    assert_eq!(bytes, b"alpha");
}

#[tokio::test]
async fn duplicate_create_errors() {
    let fs = fresh().await;
    let root = fs.root().await;
    fs.create_file(&root, "x").await.unwrap();
    match fs.create_file(&root, "x").await {
        Err(ProviderError::AlreadyExists) => {}
        other => panic!("expected AlreadyExists, got {other:?}"),
    }
}

#[tokio::test]
async fn read_at_offset_returns_slice() {
    let fs = fresh().await;
    let root = fs.root().await;
    let f = fs.create_file(&root, "x").await.unwrap();
    fs.write(&f.id, 0, b"abcdefgh").await.unwrap();
    assert_eq!(fs.read(&f.id, 2, 3).await.unwrap(), b"cde");
    assert_eq!(fs.read(&f.id, 7, 10).await.unwrap(), b"h");
    assert_eq!(fs.read(&f.id, 8, 4).await.unwrap(), b"");
}

#[tokio::test]
async fn truncate_shrinks_and_extends() {
    let fs = fresh().await;
    let root = fs.root().await;
    let f = fs.create_file(&root, "x").await.unwrap();
    fs.write(&f.id, 0, b"abcdef").await.unwrap();
    fs.truncate(&f.id, 3).await.unwrap();
    assert_eq!(fs.read(&f.id, 0, 100).await.unwrap(), b"abc");
    fs.truncate(&f.id, 5).await.unwrap();
    assert_eq!(fs.read(&f.id, 0, 100).await.unwrap(), b"abc\x00\x00");
}

#[tokio::test]
async fn write_at_offset_extends() {
    let fs = fresh().await;
    let root = fs.root().await;
    let f = fs.create_file(&root, "x").await.unwrap();
    fs.write(&f.id, 5, b"hi").await.unwrap();
    let bytes = fs.read(&f.id, 0, 100).await.unwrap();
    assert_eq!(bytes, b"\x00\x00\x00\x00\x00hi");
}

#[tokio::test]
async fn remove_file() {
    let fs = fresh().await;
    let root = fs.root().await;
    fs.create_file(&root, "x").await.unwrap();
    fs.remove(&root, "x").await.unwrap();
    match fs.lookup(&root, "x").await {
        Err(ProviderError::NotFound) => {}
        other => panic!("expected NotFound, got {other:?}"),
    }
}

#[tokio::test]
async fn remove_empty_dir() {
    let fs = fresh().await;
    let root = fs.root().await;
    fs.create_dir(&root, "d").await.unwrap();
    fs.remove(&root, "d").await.unwrap();
    assert!(fs.read_dir(&root).await.unwrap().is_empty());
}

#[tokio::test]
async fn remove_non_empty_dir_errors() {
    let fs = fresh().await;
    let root = fs.root().await;
    let d = fs.create_dir(&root, "d").await.unwrap();
    fs.create_file(&d.id, "x").await.unwrap();
    match fs.remove(&root, "d").await {
        Err(ProviderError::NotEmpty) => {}
        other => panic!("expected NotEmpty, got {other:?}"),
    }
}

#[tokio::test]
async fn rename_in_same_dir() {
    let fs = fresh().await;
    let root = fs.root().await;
    let f = fs.create_file(&root, "old").await.unwrap();
    fs.write(&f.id, 0, b"hi").await.unwrap();
    fs.rename(&root, "old", &root, "new").await.unwrap();
    assert!(fs.lookup(&root, "old").await.is_err());
    let item = fs.lookup(&root, "new").await.unwrap();
    let bytes = fs.read(&item.id, 0, 2).await.unwrap();
    assert_eq!(bytes, b"hi");
}

#[tokio::test]
async fn rename_cross_dir() {
    let fs = fresh().await;
    let root = fs.root().await;
    let a = fs.create_dir(&root, "a").await.unwrap();
    let b = fs.create_dir(&root, "b").await.unwrap();
    let f = fs.create_file(&a.id, "x").await.unwrap();
    fs.write(&f.id, 0, b"hi").await.unwrap();
    fs.rename(&a.id, "x", &b.id, "y").await.unwrap();
    let item = fs.lookup(&b.id, "y").await.unwrap();
    assert_eq!(fs.read(&item.id, 0, 2).await.unwrap(), b"hi");
    assert!(fs.lookup(&a.id, "x").await.is_err());
}

#[tokio::test]
async fn rename_onto_existing_errors() {
    let fs = fresh().await;
    let root = fs.root().await;
    fs.create_file(&root, "a").await.unwrap();
    fs.create_file(&root, "b").await.unwrap();
    match fs.rename(&root, "a", &root, "b").await {
        Err(ProviderError::AlreadyExists) => {}
        other => panic!("expected AlreadyExists, got {other:?}"),
    }
}

#[tokio::test]
async fn anchor_advances_on_mutation() {
    let fs = fresh().await;
    let root = fs.root().await;
    let a1 = fs.anchor().await;
    fs.create_file(&root, "x").await.unwrap();
    let a2 = fs.anchor().await;
    assert_ne!(a1, a2);
}

#[tokio::test]
async fn changes_since_emits_added() {
    let fs = fresh().await;
    let root = fs.root().await;
    let before = fs.anchor().await;
    fs.create_file(&root, "x").await.unwrap();
    let changes = fs.changes_since(Some(&before)).await.unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path(), "x");
    assert!(matches!(changes[0], PathChange::Added { .. }));
}

#[tokio::test]
async fn changes_since_emits_modified() {
    let fs = fresh().await;
    let root = fs.root().await;
    let f = fs.create_file(&root, "x").await.unwrap();
    fs.write(&f.id, 0, b"v1").await.unwrap();
    let before = fs.anchor().await;
    fs.write(&f.id, 0, b"v22").await.unwrap();
    let changes = fs.changes_since(Some(&before)).await.unwrap();
    assert_eq!(changes.len(), 1);
    assert!(matches!(changes[0], PathChange::Modified { .. }));
}

#[tokio::test]
async fn opening_at_existing_root_preserves_state() {
    let tr = tree();
    let fs1 = HashTreeProviderFs::fresh(tr.clone()).await.unwrap();
    let root = fs1.root().await;
    let f = fs1.create_file(&root, "x").await.unwrap();
    fs1.write(&f.id, 0, b"persisted").await.unwrap();
    let root_cid = fs1.current_root().await;

    let fs2 = HashTreeProviderFs::open(tr, root_cid).await.unwrap();
    let item = fs2.lookup(&fs2.root().await, "x").await.unwrap();
    let bytes = fs2.read(&item.id, 0, 9).await.unwrap();
    assert_eq!(bytes, b"persisted");
}

#[tokio::test]
async fn opening_at_non_directory_root_errors() {
    let tr = tree();
    // Put a blob and try to open it as a root.
    let (cid, _) = tr.put(b"not a directory").await.unwrap();
    match HashTreeProviderFs::open(tr, cid).await {
        Err(ProviderError::InvalidRoot(_)) => {}
        Err(other) => panic!("expected InvalidRoot, got {other:?}"),
        Ok(_) => panic!("expected error, got Ok"),
    }
}

#[tokio::test]
async fn observer_sees_each_new_root() {
    struct Counter(Mutex<Vec<Cid>>);
    impl RootObserver for Counter {
        fn on_new_root(&self, new_root: &Cid) -> Result<(), ProviderError> {
            self.0.lock().unwrap().push(new_root.clone());
            Ok(())
        }
    }

    let tr = tree();
    let observer = Arc::new(Counter(Mutex::new(Vec::new())));
    let root_cid = tr.put_directory(Vec::new()).await.unwrap();
    let fs = HashTreeProviderFs::open_with_observer(tr, root_cid, Some(observer.clone()))
        .await
        .unwrap();
    let root = fs.root().await;
    fs.create_file(&root, "a").await.unwrap();
    fs.create_file(&root, "b").await.unwrap();
    fs.remove(&root, "a").await.unwrap();
    assert_eq!(observer.0.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn invalid_names_rejected() {
    let fs = fresh().await;
    let root = fs.root().await;
    assert!(matches!(
        fs.create_file(&root, "").await,
        Err(ProviderError::InvalidName)
    ));
    assert!(matches!(
        fs.create_file(&root, "with/slash").await,
        Err(ProviderError::InvalidName)
    ));
}

#[tokio::test]
async fn read_directory_as_file_errors() {
    let fs = fresh().await;
    let root = fs.root().await;
    let d = fs.create_dir(&root, "d").await.unwrap();
    match fs.read(&d.id, 0, 10).await {
        Err(ProviderError::IsDir) => {}
        other => panic!("expected IsDir, got {other:?}"),
    }
}
