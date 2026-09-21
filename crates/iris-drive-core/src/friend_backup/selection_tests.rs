use std::sync::Arc;

use hashtree_core::{
    BlobReply, BlobRequest, Cid, DirEntry, HashTree, HashTreeConfig, Link, LinkType, MemoryStore,
    Store, TreeNode, encrypt_chk, sha256,
};
use hashtree_fs::FsBlobStore;
use nostr_sdk::{Keys, nips::nip44};

use super::super::storage::{FriendBackupError, FriendBackupStore, PreparedBackup, prepare_backup};

fn capsule(root: &Cid) -> String {
    let keys = Keys::generate();
    nip44::encrypt(
        keys.secret_key(),
        &keys.public_key(),
        root.to_string(),
        nip44::Version::V2,
    )
    .unwrap()
}

async fn encrypted_node(store: &FsBlobStore, node: &TreeNode) -> Cid {
    let bytes = hashtree_core::encode_tree_node(node).unwrap();
    let (ciphertext, key) = encrypt_chk(&bytes).unwrap();
    let hash = sha256(&ciphertext);
    store.put(hash, ciphertext).await.unwrap();
    Cid {
        hash,
        key: Some(key),
    }
}

#[tokio::test]
async fn encrypted_file_with_plaintext_chunk_is_rejected_before_export() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FsBlobStore::new(dir.path().join("source")).unwrap());
    let tree = HashTree::new(HashTreeConfig::new(store.clone()));
    let secret = b"private bytes stored without a cipher";
    let raw_hash = tree.put_blob(secret).await.unwrap();
    let file = encrypted_node(
        &store,
        &TreeNode::file(vec![Link::new(raw_hash).with_size(secret.len() as u64)]),
    )
    .await;
    let root = tree
        .put_directory(vec![
            DirEntry::new("private.txt", file.hash)
                .with_key(file.key.unwrap())
                .with_link_type(LinkType::File)
                .with_size(secret.len() as u64),
        ])
        .await
        .unwrap();
    let result = prepare_backup(&tree, &root, capsule(&root), dir.path().join("export")).await;
    assert!(
        matches!(result, Err(FriendBackupError::Invalid(message)) if message.contains("unencrypted"))
    );
    assert!(!dir.path().join("export").exists());
}

#[tokio::test]
async fn backup_restores_directory_fanout_and_chunked_file_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FsBlobStore::new(dir.path().join("source")).unwrap());
    let tree = HashTree::new(
        HashTreeConfig::new(store)
            .with_max_links(2)
            .with_chunk_size(4),
    );
    let mut entries = Vec::new();
    for index in 0..9 {
        let bytes = format!("file {index} contains many encrypted chunks");
        let (file, size) = tree.put(bytes.as_bytes()).await.unwrap();
        entries.push(
            DirEntry::new(format!("file-{index}"), file.hash)
                .with_key(file.key.unwrap())
                .with_link_type(LinkType::File)
                .with_size(size),
        );
    }
    let root = tree.put_directory(entries).await.unwrap();
    assert_eq!(
        tree.get_tree_node_by_cid(&root)
            .await
            .unwrap()
            .unwrap()
            .node_type,
        LinkType::Fanout
    );
    let prepared = prepare_backup(&tree, &root, capsule(&root), dir.path().join("export"))
        .await
        .unwrap();
    let restored = HashTree::new(HashTreeConfig::new(restored_store(&prepared).await));
    let listing = restored.list_directory_required(&root).await.unwrap();
    assert_eq!(listing.len(), 9);
    for (index, entry) in listing.iter().enumerate() {
        assert_eq!(
            restored
                .get(
                    &Cid {
                        hash: entry.hash,
                        key: entry.key
                    },
                    None
                )
                .await
                .unwrap()
                .unwrap(),
            format!("file {index} contains many encrypted chunks").as_bytes()
        );
    }
}

#[tokio::test]
async fn fanout_preserves_live_metadata_but_does_not_fetch_old_history() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FsBlobStore::new(dir.path().join("source")).unwrap());
    let tree = HashTree::new(HashTreeConfig::new(store).with_max_links(2));
    let (file, size) = tree.put(b"current").await.unwrap();
    let missing_history = Cid {
        hash: [7; 32],
        key: None,
    };
    let metadata = tree
        .put_directory(vec![
            DirEntry::new("prev", missing_history.hash).with_link_type(LinkType::Dir),
            DirEntry::new("current", file.hash)
                .with_key(file.key.unwrap())
                .with_link_type(LinkType::File)
                .with_size(size),
            DirEntry::new("other", file.hash)
                .with_key(file.key.unwrap())
                .with_link_type(LinkType::File)
                .with_size(size),
        ])
        .await
        .unwrap();
    let root = tree
        .put_directory(vec![
            DirEntry::new(".hashtree", metadata.hash)
                .with_key(metadata.key.unwrap())
                .with_link_type(LinkType::Dir),
            DirEntry::new("visible", file.hash)
                .with_key(file.key.unwrap())
                .with_link_type(LinkType::File)
                .with_size(size),
            DirEntry::new("also-visible", file.hash)
                .with_key(file.key.unwrap())
                .with_link_type(LinkType::File)
                .with_size(size),
        ])
        .await
        .unwrap();
    let prepared = prepare_backup(&tree, &root, capsule(&root), dir.path().join("export"))
        .await
        .unwrap();
    assert!(
        !prepared
            .manifest
            .entries
            .iter()
            .any(|entry| entry.hash == hex::encode(missing_history.hash))
    );
    let restored = HashTree::new(HashTreeConfig::new(restored_store(&prepared).await));
    assert_eq!(
        restored.list_directory_required(&root).await.unwrap().len(),
        3
    );
    assert_eq!(
        restored
            .list_directory_required(&metadata)
            .await
            .unwrap()
            .len(),
        3
    );
}

async fn restored_store(prepared: &PreparedBackup) -> Arc<MemoryStore> {
    let store = Arc::new(MemoryStore::new());
    let route = prepared.route();
    for entry in &prepared.manifest.entries {
        let mut hash = [0; 32];
        hex::decode_to_slice(&entry.hash, &mut hash).unwrap();
        assert!(
            prepared.store.get(&hash).await.unwrap().is_none(),
            "export duplicated file data"
        );
        let BlobReply::Data(bytes) = route.route(BlobRequest { hash, htl: 0 }).await.unwrap()
        else {
            panic!("selected block missing")
        };
        store.put(hash, bytes).await.unwrap();
    }
    store
}

#[tokio::test]
async fn manifest_limit_is_enforced_before_fetching_selected_children() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FsBlobStore::new(dir.path().join("source")).unwrap());
    let tree = HashTree::new(HashTreeConfig::new(store.clone()));
    let missing_child = Link::new([9; 32])
        .with_name("missing")
        .with_key([8; 32])
        .with_size(1);
    let root = encrypted_node(&store, &TreeNode::dir(vec![missing_child])).await;
    let result = super::collect_with_limit(&tree, &root, 1).await;
    assert!(
        matches!(result, Err(FriendBackupError::Invalid(message)) if message.contains("exceeds 1 blobs"))
    );
}

#[tokio::test]
async fn actual_manifest_limit_bounds_a_wide_directory_before_child_reads() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(FsBlobStore::new(dir.path().join("source")).unwrap());
    let tree = HashTree::new(HashTreeConfig::new(store.clone()));
    let links = (0..super::MAX_MANIFEST_ENTRIES)
        .map(|index| {
            let hash = sha256(&index.to_le_bytes());
            Link::new(hash)
                .with_name(format!("{index}"))
                .with_key([8; 32])
                .with_size(1)
        })
        .collect();
    let root = encrypted_node(&store, &TreeNode::dir(links)).await;
    let result = super::collect_backup_hashes(&tree, &root).await;
    assert!(
        matches!(result, Err(FriendBackupError::Invalid(message)) if message.contains("exceeds 65536 blobs"))
    );
}

#[cfg(unix)]
#[tokio::test]
async fn hosted_backup_directories_are_private() {
    use nostr_sdk::nips::nip19::ToBech32;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let source = Arc::new(FsBlobStore::new(dir.path().join("source")).unwrap());
    let tree = HashTree::new(HashTreeConfig::new(source));
    let (file, size) = tree.put(b"private").await.unwrap();
    let root = tree
        .put_directory(vec![
            DirEntry::from_cid("file", &file)
                .with_link_type(LinkType::File)
                .with_size(size),
        ])
        .await
        .unwrap();
    let prepared = prepare_backup(&tree, &root, capsule(&root), dir.path().join("export"))
        .await
        .unwrap();
    let owner = Keys::generate().public_key();
    let base = dir.path().join("host");
    let host = FriendBackupStore::open(&base).unwrap();
    host.retain(
        &owner.to_bech32().unwrap(),
        1_000_000,
        &prepared.manifest,
        prepared.route().as_ref(),
    )
    .await
    .unwrap();
    for path in [&base, &base.join(owner.to_hex())] {
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
}
