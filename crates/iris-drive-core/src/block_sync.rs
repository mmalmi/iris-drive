//! Block traversal policies for live sync.
//!
//! Iris Drive roots carry `.hashtree/prev` history links. History is useful, but
//! it must not block current file sync: deleting a placeholder should not require
//! downloading the deleted file's old bytes through the history chain.

use std::collections::{HashSet, VecDeque};

use futures::stream::{FuturesUnordered, StreamExt};
use hashtree_core::{
    Cid, EncryptionMode, Hash, HashTree, HashTreeError, Link, LinkType, Store, TreeNode,
    decode_tree_node, decrypt_object, is_tree_node,
};

use crate::indexer::should_ignore_name;
use crate::merge::{META_DIR, PREV_LINK_PATH};

/// Bounded block-fetch/upload concurrency for live Drive tree sync.
/// Keeps worst-case in-flight buffers (8 × 2 MiB chunks) safely within
/// desktop and mobile extension memory budgets while saturating RTT.
pub(crate) const LIVE_SYNC_BLOCK_CONCURRENCY: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum WalkKind {
    RootDir,
    UserDir,
    MetaDir,
    Content,
}

/// Collect blocks needed to read the current root.
///
/// This includes visible directory/file content (including internal directory
/// fanout pages) and current `.hashtree` metadata such as tombstones, root
/// metadata, and conflict records. It intentionally skips `.hashtree/prev`
/// targets because those are old history, not a prerequisite for applying the
/// current snapshot.
pub async fn collect_live_sync_hashes<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    concurrency: usize,
) -> Result<HashSet<Hash>, HashTreeError> {
    collect_live_sync_hashes_inner(tree, root, concurrency, true).await
}

/// Collect live-sync block hashes using local size metadata to avoid reading
/// or decrypting raw leaf data blobs. Legacy Blob links can also point to file
/// nodes, so the declared plaintext size alone cannot prove a link is a leaf.
///
/// Use for local-store traversals (Blossom uploads, backup checks, local block
/// availability probes) where callers check or stream the returned hashes
/// directly rather than relying on traversal reads to populate a write-back
/// store.
pub async fn collect_live_sync_hashes_without_leaf_reads<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    concurrency: usize,
) -> Result<HashSet<Hash>, HashTreeError> {
    collect_live_sync_hashes_inner(tree, root, concurrency, false).await
}

async fn collect_live_sync_hashes_inner<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    concurrency: usize,
    read_leaf_blobs: bool,
) -> Result<HashSet<Hash>, HashTreeError> {
    let concurrency = concurrency.max(1);
    let prev_name = PREV_LINK_PATH
        .rsplit_once('/')
        .map_or(PREV_LINK_PATH, |(_, name)| name);
    let store = tree.get_store();
    let mut hashes = HashSet::new();
    let mut visited = HashSet::new();
    let mut pending = VecDeque::new();
    let mut active = FuturesUnordered::new();

    pending.push_back((root.hash, root.key, WalkKind::RootDir, None));

    loop {
        while active.len() < concurrency {
            let Some((hash, key, kind, leaf_stored_size)) = pending.pop_front() else {
                break;
            };
            hashes.insert(hash);
            if !visited.insert((hash, kind)) {
                continue;
            }
            let store = &store;
            active.push(async move {
                if let Some(expected_size) = leaf_stored_size {
                    let stored_size = store
                        .blob_size(&hash)
                        .await
                        .map_err(|e| HashTreeError::Store(e.to_string()))?;
                    if stored_size == Some(expected_size) {
                        return Ok::<_, HashTreeError>((key, kind, None));
                    }
                }
                let data = store
                    .get(&hash)
                    .await
                    .map_err(|e| HashTreeError::Store(e.to_string()))?;
                Ok::<_, HashTreeError>((key, kind, data))
            });
        }

        let Some(result) = active.next().await else {
            break;
        };
        let (key, kind, Some(data)) = result? else {
            continue;
        };
        let plaintext = if let Some(key) = key {
            decrypt_object(&data, &key).map_err(|e| HashTreeError::Decryption(e.to_string()))?
        } else {
            data
        };
        if !is_tree_node(&plaintext) {
            continue;
        }
        let Ok(node) = decode_tree_node(&plaintext) else {
            continue;
        };

        let legacy_fanout = is_legacy_directory_fanout(&node);
        for link in &node.links {
            let Some(child_kind) = classify_link(&node, link, kind, legacy_fanout, prev_name)
            else {
                continue;
            };
            let leaf_stored_size = if !read_leaf_blobs
                && child_kind == WalkKind::Content
                && link.link_type == LinkType::Blob
            {
                let overhead = match link.key.map(|key| key.mode()) {
                    None => 0,
                    Some(EncryptionMode::LegacyChk | EncryptionMode::ChkV1) => {
                        hashtree_core::crypto::encrypted_size_chk(0) as u64
                    }
                    Some(EncryptionMode::RndV1) => hashtree_core::crypto::encrypted_size(0) as u64,
                };
                link.size.checked_add(overhead)
            } else {
                None
            };
            if !visited.contains(&(link.hash, child_kind)) {
                pending.push_back((link.hash, link.key, child_kind, leaf_stored_size));
            }
        }
    }

    Ok(hashes)
}

fn classify_link(
    node: &TreeNode,
    link: &Link,
    kind: WalkKind,
    legacy_fanout: bool,
    prev_name: &str,
) -> Option<WalkKind> {
    if node.node_type == LinkType::File {
        return Some(WalkKind::Content);
    }
    if matches!(
        kind,
        WalkKind::RootDir | WalkKind::UserDir | WalkKind::MetaDir
    ) && is_internal_directory_link(node, link, legacy_fanout)
    {
        return Some(kind);
    }
    let is_dir = matches!(link.link_type, LinkType::Dir | LinkType::Fanout);
    match kind {
        WalkKind::RootDir => {
            let name = link.name.as_deref()?;
            if name == META_DIR {
                Some(if is_dir {
                    WalkKind::MetaDir
                } else {
                    WalkKind::Content
                })
            } else if should_ignore_name(name) {
                None
            } else if is_dir {
                Some(WalkKind::UserDir)
            } else {
                Some(WalkKind::Content)
            }
        }
        WalkKind::UserDir => {
            let name = link.name.as_deref()?;
            if should_ignore_name(name) {
                None
            } else if is_dir {
                Some(WalkKind::UserDir)
            } else {
                Some(WalkKind::Content)
            }
        }
        WalkKind::MetaDir => {
            let name = link.name.as_deref()?;
            if name == prev_name {
                None
            } else {
                Some(WalkKind::Content)
            }
        }
        WalkKind::Content => Some(WalkKind::Content),
    }
}

fn is_internal_chunk_name(name: &str) -> bool {
    name.strip_prefix("_chunk_").is_some_and(|suffix| {
        !suffix.is_empty()
            && suffix.bytes().all(|byte| byte.is_ascii_digit())
            && suffix.parse::<usize>().is_ok()
    })
}

fn is_legacy_directory_fanout(node: &TreeNode) -> bool {
    node.node_type == LinkType::Dir
        && !node.links.is_empty()
        && node.links.iter().all(|link| {
            link.link_type == LinkType::Dir
                && link.name.as_deref().is_some_and(is_internal_chunk_name)
        })
}

fn is_internal_directory_link(node: &TreeNode, link: &Link, legacy_fanout: bool) -> bool {
    if node.node_type == LinkType::Fanout {
        return matches!(link.link_type, LinkType::Dir | LinkType::Fanout);
    }
    legacy_fanout
        && link.link_type == LinkType::Dir
        && link.name.as_deref().is_some_and(is_internal_chunk_name)
}

/// Verify every referenced encrypted block without retaining file bytes or
/// fetching the same blocks a second time for integrity checks.
pub(crate) async fn verify_reachable_blocks<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
) -> Result<(), HashTreeError> {
    let store = RequiredBlocks(tree.get_store().clone());
    let verified = HashTree::new(hashtree_core::HashTreeConfig::new(std::sync::Arc::new(
        store,
    )));
    hashtree_core::diff::collect_hashes(&verified, root, LIVE_SYNC_BLOCK_CONCURRENCY).await?;
    Ok(())
}

struct RequiredBlocks<S: Store>(std::sync::Arc<S>);

#[async_trait::async_trait]
impl<S: Store> Store for RequiredBlocks<S> {
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, hashtree_core::StoreError> {
        self.0.put(hash, data).await
    }

    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, hashtree_core::StoreError> {
        let bytes = self.0.get(hash).await?.ok_or_else(|| {
            hashtree_core::StoreError::Other("Required file block is unavailable".into())
        })?;
        if hashtree_core::sha256(&bytes) != *hash {
            return Err(hashtree_core::StoreError::Other(
                "File block failed its integrity check".into(),
            ));
        }
        Ok(Some(bytes))
    }

    async fn has(&self, hash: &Hash) -> Result<bool, hashtree_core::StoreError> {
        self.0.has(hash).await
    }

    async fn delete(&self, hash: &Hash) -> Result<bool, hashtree_core::StoreError> {
        self.0.delete(hash).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use hashtree_core::{
        DEFAULT_CHUNK_SIZE, DirEntry, HashTreeConfig, LinkType, MemoryStore, StoreError,
    };

    struct RecordingStore {
        inner: Arc<MemoryStore>,
        gets: Mutex<Vec<Hash>>,
    }

    impl RecordingStore {
        fn new(inner: Arc<MemoryStore>) -> Self {
            Self {
                inner,
                gets: Mutex::new(Vec::new()),
            }
        }

        fn gets(&self) -> Vec<Hash> {
            self.gets.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Store for RecordingStore {
        async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
            self.inner.put(hash, data).await
        }

        async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
            self.gets.lock().unwrap().push(*hash);
            self.inner.get(hash).await
        }

        async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
            self.inner.has(hash).await
        }

        async fn blob_size(&self, hash: &Hash) -> Result<Option<u64>, StoreError> {
            self.inner.blob_size(hash).await
        }

        async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
            self.inner.delete(hash).await
        }
    }

    #[tokio::test]
    async fn live_sync_preserves_small_legacy_blob_linked_chunked_files() {
        let store = Arc::new(MemoryStore::new());
        let tree = HashTree::new(HashTreeConfig::new(store.clone()).with_chunk_size(1024));
        let payload: Vec<u8> = (0..3073).map(|i| (i % 251) as u8).collect();
        let (file, size) = tree.put(&payload).await.unwrap();
        // Legacy directories label chunked files as Blob even when the writer
        // used a smaller chunk size than today's default.
        let root = tree
            .put_directory(vec![
                DirEntry::from_cid("legacy.bin", &file)
                    .with_size(size)
                    .with_link_type(LinkType::Blob),
            ])
            .await
            .unwrap();
        let hashes = collect_live_sync_hashes_without_leaf_reads(&tree, &root, 8)
            .await
            .unwrap();
        let expected = collect_live_sync_hashes(&tree, &root, 8).await.unwrap();
        assert_eq!(
            hashes, expected,
            "local-only traversal must retain legacy file children"
        );
        let destination = Arc::new(MemoryStore::new());
        for hash in hashes {
            destination
                .put(hash, store.get(&hash).await.unwrap().unwrap())
                .await
                .unwrap();
        }
        let reader = HashTree::new(HashTreeConfig::new(destination));
        let resolved = reader.resolve(&root, "legacy.bin").await.unwrap().unwrap();
        assert_eq!(reader.get(&resolved, None).await.unwrap().unwrap(), payload);
    }

    #[tokio::test]
    async fn live_sync_hashes_skip_prev_history_target() {
        let store = Arc::new(MemoryStore::new());
        let tree = HashTree::new(HashTreeConfig::new(store));
        let (file_cid, _) = tree.put(b"visible bytes").await.unwrap();
        let visible_root = tree
            .put_directory(vec![DirEntry {
                name: "visible.txt".to_string(),
                hash: file_cid.hash,
                key: file_cid.key,
                link_type: LinkType::File,
                size: 13,
                meta: None,
            }])
            .await
            .unwrap();
        let missing_prev = Cid {
            hash: [9; 32],
            key: None,
        };
        let root_with_history = crate::indexer::layer_prev_link(&tree, visible_root, &missing_prev)
            .await
            .unwrap();

        let hashes = collect_live_sync_hashes(&tree, &root_with_history, 4)
            .await
            .unwrap();

        assert!(hashes.contains(&root_with_history.hash));
        assert!(hashes.contains(&file_cid.hash));
        assert!(!hashes.contains(&missing_prev.hash));
    }

    #[tokio::test]
    async fn live_sync_hashes_include_fanout_directory_pages() {
        let store = Arc::new(MemoryStore::new());
        let tree = HashTree::new(HashTreeConfig::new(store.clone()));

        // >174 entries triggers hashtree-core LinkType::Fanout internal directory pages.
        let mut photo_entries = Vec::with_capacity(200);
        for i in 0..200 {
            let body = format!("photo-{i:04}");
            let (cid, size) = tree.put(body.as_bytes()).await.unwrap();
            photo_entries.push(
                DirEntry::from_cid(format!("img_{i:04}.jpg"), &cid)
                    .with_size(size)
                    .with_link_type(LinkType::Blob),
            );
        }
        let photos_cid = tree.put_directory(photo_entries).await.unwrap();
        let root = tree
            .put_directory(vec![
                DirEntry::from_cid("Photos", &photos_cid).with_link_type(LinkType::Dir),
            ])
            .await
            .unwrap();

        let collected = collect_live_sync_hashes_without_leaf_reads(&tree, &root, 8)
            .await
            .unwrap();

        // Copy only collected blocks to a fresh store and verify listing + reading succeed.
        let dest_store = Arc::new(MemoryStore::new());
        for hash in &collected {
            let bytes = store.get(hash).await.unwrap().expect("block must exist");
            dest_store.put(*hash, bytes).await.unwrap();
        }
        let dest_tree = HashTree::new(HashTreeConfig::new(dest_store));
        let listed_root = dest_tree.list_directory(&root).await.unwrap();
        assert_eq!(listed_root.len(), 1);
        let photos_entry = &listed_root[0];
        let listed_photos = dest_tree
            .list_directory(&Cid {
                hash: photos_entry.hash,
                key: photos_entry.key,
            })
            .await
            .unwrap();
        assert_eq!(listed_photos.len(), 200);
        // Must include root + photos fanout root + >=2 internal fanout pages + 200 photo blobs.
        assert!(collected.len() >= 204);
    }

    #[tokio::test]
    async fn live_sync_hashes_without_leaf_reads_skips_file_blob_reads() {
        let base_store = Arc::new(MemoryStore::new());
        let write_tree = HashTree::new(HashTreeConfig::new(base_store.clone()));

        let (small_cid, small_size) = write_tree.put(b"small blob").await.unwrap();
        let big_bytes = vec![0x5Au8; DEFAULT_CHUNK_SIZE * 2 + 123];
        let (big_cid, big_size) = write_tree.put(&big_bytes).await.unwrap();
        let root = write_tree
            .put_directory(vec![
                DirEntry::from_cid("small.txt", &small_cid)
                    .with_size(small_size)
                    .with_link_type(LinkType::Blob),
                DirEntry::from_cid("big.bin", &big_cid)
                    .with_size(big_size)
                    .with_link_type(LinkType::File),
            ])
            .await
            .unwrap();

        let full_hashes = collect_live_sync_hashes(&write_tree, &root, 8)
            .await
            .unwrap();

        let recording = Arc::new(RecordingStore::new(base_store));
        let read_tree = HashTree::new(HashTreeConfig::new(recording.clone()));
        let fast_hashes = collect_live_sync_hashes_without_leaf_reads(&read_tree, &root, 8)
            .await
            .unwrap();

        assert_eq!(fast_hashes, full_hashes);
        let gets = recording.gets();
        // Only root dir and big.bin's File tree node should have been read (2 blocks total),
        // NOT small.txt's blob or any of big.bin's 3 leaf chunks!
        assert_eq!(gets.len(), 2);
        assert!(gets.contains(&root.hash));
        assert!(gets.contains(&big_cid.hash));
        assert!(!gets.contains(&small_cid.hash));
    }
}
