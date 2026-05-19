//! Walking the `._prev` revision chain.
//!
//! Each directory written by `index_dir_with_history` carries an
//! optional `._prev` child entry whose `Cid` (hash + key) points at
//! the prior version of that directory. The chain is walkable as far
//! back as the blocks remain in the local store; when a previous
//! version's blocks have been GC'd the walk simply stops there.
//!
//! See [`crate::merge::PREV_LINK_NAME`] for the convention.

use hashtree_core::{Cid, HashTree, HashTreeError, LinkType, Store};
use thiserror::Error;

use crate::merge::{META_DIR, PREV_LINK_PATH};

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("tree: {0}")]
    Tree(#[from] HashTreeError),
}

/// Walk the `._prev` chain starting from `root`, newest-first, up to
/// `limit` entries (use `usize::MAX` for unbounded). The returned
/// vector always begins with `root` itself. The walk stops when:
///
/// - a directory has no `._prev` child (chain terminus / first commit),
/// - the blocks for a `._prev` target are missing locally (GC'd or
///   never downloaded — terminates cleanly),
/// - `limit` is reached.
///
/// Errors only on actual tree-decoding faults; missing-block stops
/// silently.
pub async fn walk_history<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    limit: usize,
) -> Result<Vec<Cid>, HistoryError> {
    let prev_name = PREV_LINK_PATH
        .rsplit_once('/')
        .map_or(PREV_LINK_PATH, |(_, n)| n);
    let mut chain = Vec::new();
    let mut current = root.clone();
    while chain.len() < limit {
        chain.push(current.clone());
        // Walk through .hashtree/ to find the prev link.
        let Ok(entries) = tree.list_directory(&current).await else {
            break; // missing block — chain truncates here
        };
        let Some(meta) = entries
            .into_iter()
            .find(|e| e.name == META_DIR && e.link_type == LinkType::Dir)
        else {
            break;
        };
        let meta_cid = Cid {
            hash: meta.hash,
            key: meta.key,
        };
        let Ok(meta_entries) = tree.list_directory(&meta_cid).await else {
            break;
        };
        let Some(prev) = meta_entries
            .into_iter()
            .find(|e| e.name == prev_name && e.link_type == LinkType::Dir)
        else {
            break;
        };
        current = Cid {
            hash: prev.hash,
            key: prev.key,
        };
    }
    Ok(chain)
}

/// Walk back `steps` revisions and return the CID at that point. `0`
/// returns `current` itself, `1` returns its `._prev`, etc. Errors if
/// the chain runs out before `steps` is reached.
pub async fn revision_at<S: Store>(
    tree: &HashTree<S>,
    current: &Cid,
    steps: usize,
) -> Result<Cid, HistoryError> {
    let chain = walk_history(tree, current, steps.saturating_add(1)).await?;
    chain
        .get(steps)
        .cloned()
        .ok_or_else(|| HistoryError::Tree(HashTreeError::PathNotFound(format!("rev -{steps}"))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexer::{index_dir, index_dir_with_history};
    use hashtree_core::{HashTreeConfig, MemoryStore};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn new_tree() -> HashTree<MemoryStore> {
        HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public())
    }

    #[tokio::test]
    async fn first_root_has_no_chain() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"a").unwrap();
        let tree = new_tree();
        let root = index_dir(&tree, dir.path()).await.unwrap();
        let chain = walk_history(&tree, &root, 10).await.unwrap();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].hash, root.hash);
    }

    #[tokio::test]
    async fn three_imports_yield_three_links() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("v1.txt"), b"a").unwrap();
        let tree = new_tree();
        let r1 = index_dir(&tree, dir.path()).await.unwrap();

        std::fs::write(dir.path().join("v2.txt"), b"b").unwrap();
        let r2 = index_dir_with_history(&tree, dir.path(), Some(&r1), 1000)
            .await
            .unwrap();

        std::fs::write(dir.path().join("v3.txt"), b"c").unwrap();
        let r3 = index_dir_with_history(&tree, dir.path(), Some(&r2), 2000)
            .await
            .unwrap();

        let chain = walk_history(&tree, &r3, 10).await.unwrap();
        let hashes: Vec<[u8; 32]> = chain.iter().map(|c| c.hash).collect();
        assert_eq!(hashes, vec![r3.hash, r2.hash, r1.hash]);
    }

    #[tokio::test]
    async fn limit_truncates_chain() {
        let dir = tempdir().unwrap();
        let tree = new_tree();
        std::fs::write(dir.path().join("v1.txt"), b"a").unwrap();
        let r1 = index_dir(&tree, dir.path()).await.unwrap();
        std::fs::write(dir.path().join("v2.txt"), b"b").unwrap();
        let r2 = index_dir_with_history(&tree, dir.path(), Some(&r1), 1000)
            .await
            .unwrap();
        std::fs::write(dir.path().join("v3.txt"), b"c").unwrap();
        let r3 = index_dir_with_history(&tree, dir.path(), Some(&r2), 2000)
            .await
            .unwrap();

        let chain = walk_history(&tree, &r3, 2).await.unwrap();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].hash, r3.hash);
        assert_eq!(chain[1].hash, r2.hash);
    }

    #[tokio::test]
    async fn revision_at_returns_correct_root() {
        let dir = tempdir().unwrap();
        let tree = new_tree();
        std::fs::write(dir.path().join("v1.txt"), b"a").unwrap();
        let r1 = index_dir(&tree, dir.path()).await.unwrap();
        std::fs::write(dir.path().join("v2.txt"), b"b").unwrap();
        let r2 = index_dir_with_history(&tree, dir.path(), Some(&r1), 1000)
            .await
            .unwrap();

        let zero = revision_at(&tree, &r2, 0).await.unwrap();
        let one = revision_at(&tree, &r2, 1).await.unwrap();
        assert_eq!(zero.hash, r2.hash);
        assert_eq!(one.hash, r1.hash);
    }

    #[tokio::test]
    async fn chain_terminus_returns_none_or_error_for_out_of_range() {
        let dir = tempdir().unwrap();
        let tree = new_tree();
        std::fs::write(dir.path().join("only.txt"), b"x").unwrap();
        let r1 = index_dir(&tree, dir.path()).await.unwrap();
        // r1 is first revision — only one entry in chain.
        let chain = walk_history(&tree, &r1, 10).await.unwrap();
        assert_eq!(chain.len(), 1);
        // Asking for step 1 errors out (only 0 exists).
        assert!(revision_at(&tree, &r1, 1).await.is_err());
    }

    #[tokio::test]
    async fn current_view_skips_prev_entry() {
        // Verifies the chain link doesn't pollute the user-visible
        // listing — walk_device_tree (used by `idrive list`) filters
        // out ._prev.
        let dir = tempdir().unwrap();
        let tree = new_tree();
        std::fs::write(dir.path().join("v1.txt"), b"a").unwrap();
        let r1 = index_dir(&tree, dir.path()).await.unwrap();
        std::fs::write(dir.path().join("v2.txt"), b"b").unwrap();
        let r2 = index_dir_with_history(&tree, dir.path(), Some(&r1), 1000)
            .await
            .unwrap();
        let (files, _) = crate::merge::walk_device_tree(&tree, &r2).await.unwrap();
        let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, vec!["v1.txt", "v2.txt"]);
        assert!(!names.iter().any(|n| n.starts_with(META_DIR)));
    }
}
