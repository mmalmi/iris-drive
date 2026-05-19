//! Walk a local directory and build the equivalent hashtree directory.
//!
//! Used in two situations:
//! - First-time import: a user points iris-drive at an existing folder.
//! - Sync engine: compute the local CID before publishing a new root.
//!
//! The indexer is deterministic — the same on-disk tree always produces
//! the same root CID — and tests exercise that property directly.

use std::path::Path;

use hashtree_core::{Cid, DirEntry, HashTree, HashTreeError, LinkType, Store};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("tree: {0}")]
    Tree(#[from] HashTreeError),
    #[error("non-utf8 file name at {0}")]
    NonUtf8(String),
    #[error("path is not a directory: {0}")]
    NotADirectory(String),
}

/// Index a directory recursively, returning the root htree CID.
///
/// Symlinks are not followed; they are silently skipped for v1 — Drive
/// and Dropbox both ignore symlinks too. Files unreadable due to
/// permissions surface as `IndexError::Io`.
pub async fn index_dir<S: Store>(
    tree: &HashTree<S>,
    dir: &Path,
) -> Result<Cid, IndexError> {
    if !dir.is_dir() {
        return Err(IndexError::NotADirectory(dir.display().to_string()));
    }
    index_dir_inner(tree, dir).await
}

fn index_dir_inner<'a, S: Store>(
    tree: &'a HashTree<S>,
    dir: &'a Path,
) -> futures::future::BoxFuture<'a, Result<Cid, IndexError>> {
    Box::pin(async move {
        let mut entries: Vec<DirEntry> = Vec::new();
        let mut children: Vec<(String, std::path::PathBuf)> = Vec::new();

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name
                .into_string()
                .map_err(|s| IndexError::NonUtf8(s.to_string_lossy().into_owned()))?;
            children.push((name, entry.path()));
        }

        // Sort for determinism.
        children.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, path) in children {
            let metadata = match std::fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(e) => return Err(IndexError::Io(e)),
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                let child_cid = index_dir_inner(tree, &path).await?;
                let mut e = DirEntry::from_cid(name, &child_cid);
                e.link_type = LinkType::Dir;
                entries.push(e);
            } else if metadata.is_file() {
                let bytes = std::fs::read(&path)?;
                let size = bytes.len() as u64;
                let (cid, _) = tree.put(&bytes).await?;
                let link_type = if size > hashtree_core::DEFAULT_CHUNK_SIZE as u64 {
                    LinkType::File
                } else {
                    LinkType::Blob
                };
                let mut e = DirEntry::from_cid(name, &cid).with_size(size);
                e.link_type = link_type;
                entries.push(e);
            }
        }

        let cid = tree.put_directory(entries).await?;
        Ok(cid)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hashtree_core::{HashTreeConfig, MemoryStore};
    use std::sync::Arc;
    use tempfile::tempdir;

    fn new_tree() -> HashTree<MemoryStore> {
        HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public())
    }

    #[tokio::test]
    async fn empty_dir_indexes_to_empty_htree_dir() {
        let dir = tempdir().unwrap();
        let tree = new_tree();
        let cid = index_dir(&tree, dir.path()).await.unwrap();
        let listing = tree.list_directory(&cid).await.unwrap();
        assert!(listing.is_empty());
    }

    #[tokio::test]
    async fn single_file_appears_with_correct_name_and_size() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), b"hi there").unwrap();
        let tree = new_tree();
        let cid = index_dir(&tree, dir.path()).await.unwrap();
        let listing = tree.list_directory(&cid).await.unwrap();
        assert_eq!(listing.len(), 1);
        assert_eq!(listing[0].name, "hello.txt");
        assert_eq!(listing[0].size, 8);
    }

    #[tokio::test]
    async fn nested_dir_indexed_recursively() {
        let dir = tempdir().unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub").join("a.txt"), b"a").unwrap();
        let tree = new_tree();
        let cid = index_dir(&tree, dir.path()).await.unwrap();
        let top = tree.list_directory(&cid).await.unwrap();
        assert_eq!(top.len(), 1);
        assert_eq!(top[0].name, "sub");
        let sub_cid = Cid {
            hash: top[0].hash,
            key: top[0].key,
        };
        let sub = tree.list_directory(&sub_cid).await.unwrap();
        assert_eq!(sub.len(), 1);
        assert_eq!(sub[0].name, "a.txt");
    }

    #[tokio::test]
    async fn indexing_is_deterministic() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("a"), b"alpha").unwrap();
        std::fs::write(dir.path().join("b"), b"beta").unwrap();
        std::fs::create_dir(dir.path().join("inner")).unwrap();
        std::fs::write(dir.path().join("inner").join("c"), b"gamma").unwrap();
        let cid_1 = index_dir(&new_tree(), dir.path()).await.unwrap();
        let cid_2 = index_dir(&new_tree(), dir.path()).await.unwrap();
        assert_eq!(cid_1.hash, cid_2.hash);
    }

    #[tokio::test]
    async fn different_contents_produce_different_cids() {
        let dir_a = tempdir().unwrap();
        let dir_b = tempdir().unwrap();
        std::fs::write(dir_a.path().join("a.txt"), b"alpha").unwrap();
        std::fs::write(dir_b.path().join("a.txt"), b"different").unwrap();
        let cid_a = index_dir(&new_tree(), dir_a.path()).await.unwrap();
        let cid_b = index_dir(&new_tree(), dir_b.path()).await.unwrap();
        assert_ne!(cid_a.hash, cid_b.hash);
    }

    #[tokio::test]
    async fn symlinks_are_ignored() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("real.txt"), b"real").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            dir.path().join("real.txt"),
            dir.path().join("link.txt"),
        )
        .unwrap();
        let tree = new_tree();
        let cid = index_dir(&tree, dir.path()).await.unwrap();
        let listing = tree.list_directory(&cid).await.unwrap();
        // On Unix we expect only the real file; on non-Unix the symlink
        // isn't created so we also expect just one entry. Either way:
        assert_eq!(listing.len(), 1);
        assert_eq!(listing[0].name, "real.txt");
    }

    #[tokio::test]
    async fn non_existent_dir_errors() {
        let tree = new_tree();
        let err = index_dir(&tree, Path::new("/this/should/not/exist/abcxyz"))
            .await
            .unwrap_err();
        assert!(matches!(err, IndexError::NotADirectory(_)));
    }
}
