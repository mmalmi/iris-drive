//! Block traversal policies for live sync.
//!
//! Iris Drive roots carry `.hashtree/prev` history links. History is useful, but
//! it must not block current file sync: deleting a placeholder should not require
//! downloading the deleted file's old bytes through the history chain.

use std::collections::HashSet;

use futures::future::BoxFuture;
use hashtree_core::diff::collect_hashes;
use hashtree_core::{Cid, Hash, HashTree, HashTreeError, LinkType, Store};

use crate::indexer::should_ignore_name;
use crate::merge::{META_DIR, PREV_LINK_PATH};

/// Collect blocks needed to read the current root.
///
/// This includes visible directory/file content and current `.hashtree`
/// metadata such as tombstones, root metadata, and conflict records. It
/// intentionally skips `.hashtree/prev` targets because those are old history,
/// not a prerequisite for applying the current snapshot.
pub async fn collect_live_sync_hashes<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    concurrency: usize,
) -> Result<HashSet<Hash>, HashTreeError> {
    let mut hashes = HashSet::new();
    collect_live_dir_hashes(tree, root, "", concurrency, &mut hashes).await?;
    Ok(hashes)
}

fn collect_live_dir_hashes<'a, S: Store>(
    tree: &'a HashTree<S>,
    dir: &'a Cid,
    prefix: &'a str,
    concurrency: usize,
    hashes: &'a mut HashSet<Hash>,
) -> BoxFuture<'a, Result<(), HashTreeError>> {
    Box::pin(async move {
        hashes.insert(dir.hash);
        let entries = tree.list_directory(dir).await?;
        for entry in entries {
            let path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{prefix}/{}", entry.name)
            };
            let child = Cid {
                hash: entry.hash,
                key: entry.key,
            };

            if prefix.is_empty() && entry.name == META_DIR {
                if entry.link_type == LinkType::Dir {
                    collect_meta_dir_hashes(tree, &child, META_DIR, concurrency, hashes).await?;
                } else {
                    collect_entry_hashes(tree, &child, concurrency, hashes).await?;
                }
                continue;
            }

            if should_ignore_name(&entry.name) {
                continue;
            }

            if entry.link_type == LinkType::Dir {
                collect_live_dir_hashes(tree, &child, &path, concurrency, hashes).await?;
            } else {
                collect_entry_hashes(tree, &child, concurrency, hashes).await?;
            }
        }
        Ok(())
    })
}

fn collect_meta_dir_hashes<'a, S: Store>(
    tree: &'a HashTree<S>,
    dir: &'a Cid,
    prefix: &'a str,
    concurrency: usize,
    hashes: &'a mut HashSet<Hash>,
) -> BoxFuture<'a, Result<(), HashTreeError>> {
    Box::pin(async move {
        hashes.insert(dir.hash);
        let entries = tree.list_directory(dir).await?;
        for entry in entries {
            let path = format!("{prefix}/{}", entry.name);
            if path == PREV_LINK_PATH {
                continue;
            }

            let child = Cid {
                hash: entry.hash,
                key: entry.key,
            };
            if entry.link_type == LinkType::Dir {
                collect_meta_dir_hashes(tree, &child, &path, concurrency, hashes).await?;
            } else {
                collect_entry_hashes(tree, &child, concurrency, hashes).await?;
            }
        }
        Ok(())
    })
}

async fn collect_entry_hashes<S: Store>(
    tree: &HashTree<S>,
    cid: &Cid,
    concurrency: usize,
    hashes: &mut HashSet<Hash>,
) -> Result<(), HashTreeError> {
    hashes.extend(collect_hashes(tree, cid, concurrency).await?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashtree_core::{DirEntry, HashTreeConfig, LinkType, MemoryStore};

    #[tokio::test]
    async fn live_sync_hashes_skip_prev_history_target() {
        let store = std::sync::Arc::new(MemoryStore::new());
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
}
