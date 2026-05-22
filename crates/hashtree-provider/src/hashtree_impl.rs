//! Concrete `ProviderFs` over `hashtree_core::HashTree<S>`.
//!
//! This is the substrate OS adapters consume. Items are identified by
//! their forward-slash path within the tree (`""` is the root); inode
//! bookkeeping is the adapter's problem if it needs it.
//!
//! The current root CID is held in an `RwLock`. Mutations serialize on a
//! tokio `Mutex` so concurrent writers can't race on the
//! read-old-root → compute-new-root → swap sequence.

use std::sync::Arc;

use async_trait::async_trait;
use hashtree_core::{Cid, HashTree, HashTreeError, LinkType, Store};
use tokio::sync::{Mutex, RwLock};

use crate::diff::{PathChange, path_diff};
use crate::error::ProviderError;
use crate::provider::{DirItem, Item, ItemKind, ProviderFs, SyncAnchor};

/// Hook called after every successful mutation. Lets a daemon publish
/// the new root over Nostr / observe revisions / etc. Errors are
/// surfaced to the mutation caller; pass `None` to disable.
pub trait RootObserver: Send + Sync {
    fn on_new_root(&self, new_root: &Cid) -> Result<(), ProviderError>;
}

pub struct HashTreeProviderFs<S: Store> {
    tree: Arc<HashTree<S>>,
    root: RwLock<Cid>,
    modify_lock: Mutex<()>,
    observer: Option<Arc<dyn RootObserver>>,
}

impl<S: Store> HashTreeProviderFs<S> {
    /// Open over an existing root. The root must point at a directory.
    pub async fn open(tree: Arc<HashTree<S>>, root: Cid) -> Result<Self, ProviderError> {
        Self::open_with_observer(tree, root, None).await
    }

    pub async fn open_with_observer(
        tree: Arc<HashTree<S>>,
        root: Cid,
        observer: Option<Arc<dyn RootObserver>>,
    ) -> Result<Self, ProviderError> {
        let is_dir = tree
            .get_directory_node(&root)
            .await
            .map_err(map_err)?
            .is_some();
        if !is_dir {
            return Err(ProviderError::InvalidRoot(
                "root CID does not point at a directory".into(),
            ));
        }
        Ok(Self {
            tree,
            root: RwLock::new(root),
            modify_lock: Mutex::new(()),
            observer,
        })
    }

    /// Build a fresh provider rooted at an empty directory.
    pub async fn fresh(tree: Arc<HashTree<S>>) -> Result<Self, ProviderError> {
        let root = tree.put_directory(Vec::new()).await.map_err(map_err)?;
        Self::open(tree, root).await
    }

    /// Current root CID.
    pub async fn current_root(&self) -> Cid {
        self.root.read().await.clone()
    }

    fn ensure_valid_name(name: &str) -> Result<(), ProviderError> {
        if name.is_empty() || name.contains('/') {
            Err(ProviderError::InvalidName)
        } else {
            Ok(())
        }
    }

    async fn apply_new_root(&self, new_root: Cid) -> Result<(), ProviderError> {
        if let Some(observer) = &self.observer {
            observer.on_new_root(&new_root)?;
        }
        *self.root.write().await = new_root;
        Ok(())
    }

    /// Split an item id (path) into (parent_segments, name). Root id ""
    /// has no parent and returns an error.
    fn split_path(id: &str) -> Result<(Vec<&str>, &str), ProviderError> {
        if id.is_empty() {
            return Err(ProviderError::InvalidName);
        }
        let mut parts: Vec<&str> = id.split('/').filter(|s| !s.is_empty()).collect();
        let name = parts.pop().ok_or(ProviderError::InvalidName)?;
        Ok((parts, name))
    }

    fn segments(id: &str) -> Vec<&str> {
        id.split('/').filter(|s| !s.is_empty()).collect()
    }

    fn join(parent: &str, name: &str) -> String {
        if parent.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", parent, name)
        }
    }

    async fn resolve(&self, id: &str) -> Result<ResolvedEntry, ProviderError> {
        if id.is_empty() {
            let root = self.current_root().await;
            return Ok(ResolvedEntry {
                cid: root,
                link_type: LinkType::Dir,
                size: 0,
            });
        }
        let (parent_segs, name) = Self::split_path(id)?;
        let parent_cid = self.descend(&parent_segs).await?;
        let entries = self
            .tree
            .list_directory(&parent_cid)
            .await
            .map_err(map_err)?;
        let entry = entries
            .into_iter()
            .find(|e| e.name == name)
            .ok_or(ProviderError::NotFound)?;
        Ok(ResolvedEntry {
            cid: Cid {
                hash: entry.hash,
                key: entry.key,
            },
            link_type: entry.link_type,
            size: entry.size,
        })
    }

    async fn descend(&self, segments: &[&str]) -> Result<Cid, ProviderError> {
        let mut current = self.current_root().await;
        for seg in segments {
            let listing = self.tree.list_directory(&current).await.map_err(map_err)?;
            let entry = listing
                .into_iter()
                .find(|e| e.name == *seg)
                .ok_or(ProviderError::NotFound)?;
            if entry.link_type != LinkType::Dir {
                return Err(ProviderError::NotDir);
            }
            current = Cid {
                hash: entry.hash,
                key: entry.key,
            };
        }
        Ok(current)
    }
}

struct ResolvedEntry {
    cid: Cid,
    link_type: LinkType,
    size: u64,
}

fn map_err(e: HashTreeError) -> ProviderError {
    ProviderError::Backend(e.to_string())
}

fn link_type_for_size(size: u64) -> LinkType {
    if size > hashtree_core::DEFAULT_CHUNK_SIZE as u64 {
        LinkType::File
    } else {
        LinkType::Blob
    }
}

fn kind_from_link(lt: LinkType) -> ItemKind {
    match lt {
        LinkType::Dir => ItemKind::Directory,
        _ => ItemKind::File,
    }
}

#[async_trait]
impl<S: Store + 'static> ProviderFs for HashTreeProviderFs<S> {
    type ItemId = String;

    async fn root(&self) -> Self::ItemId {
        String::new()
    }

    async fn lookup(
        &self,
        parent: &Self::ItemId,
        name: &str,
    ) -> Result<Item<Self::ItemId>, ProviderError> {
        Self::ensure_valid_name(name)?;
        let id = Self::join(parent, name);
        let resolved = self.resolve(&id).await?;
        Ok(Item {
            id,
            name: name.to_string(),
            kind: kind_from_link(resolved.link_type),
            size: resolved.size,
        })
    }

    async fn item(&self, id: &Self::ItemId) -> Result<Item<Self::ItemId>, ProviderError> {
        let resolved = self.resolve(id).await?;
        let name = id.rsplit('/').next().unwrap_or("").to_string();
        Ok(Item {
            id: id.clone(),
            name,
            kind: kind_from_link(resolved.link_type),
            size: resolved.size,
        })
    }

    async fn read(
        &self,
        id: &Self::ItemId,
        offset: u64,
        size: u64,
    ) -> Result<Vec<u8>, ProviderError> {
        let resolved = self.resolve(id).await?;
        if resolved.link_type == LinkType::Dir {
            return Err(ProviderError::IsDir);
        }
        if offset >= resolved.size || size == 0 {
            return Ok(Vec::new());
        }
        let end = (offset + size).min(resolved.size);
        if resolved.cid.key.is_some() {
            // Encrypted single-chunk blob — fetch + slice.
            let data = self
                .tree
                .get(&resolved.cid, None)
                .await
                .map_err(map_err)?
                .ok_or(ProviderError::NotFound)?;
            let start = offset as usize;
            let stop = (end as usize).min(data.len());
            return Ok(data[start..stop].to_vec());
        }
        let data = self
            .tree
            .read_file_range(&resolved.cid.hash, offset, Some(end))
            .await
            .map_err(map_err)?
            .ok_or(ProviderError::NotFound)?;
        Ok(data)
    }

    async fn read_dir(
        &self,
        id: &Self::ItemId,
    ) -> Result<Vec<DirItem<Self::ItemId>>, ProviderError> {
        let resolved = self.resolve(id).await?;
        if resolved.link_type != LinkType::Dir {
            return Err(ProviderError::NotDir);
        }
        let listing = self
            .tree
            .list_directory(&resolved.cid)
            .await
            .map_err(map_err)?;
        Ok(listing
            .into_iter()
            .map(|e| DirItem {
                id: Self::join(id, &e.name),
                name: e.name,
                kind: kind_from_link(e.link_type),
            })
            .collect())
    }

    async fn create_file(
        &self,
        parent: &Self::ItemId,
        name: &str,
    ) -> Result<Item<Self::ItemId>, ProviderError> {
        Self::ensure_valid_name(name)?;
        let _guard = self.modify_lock.lock().await;

        let id = Self::join(parent, name);
        if self.resolve(&id).await.is_ok() {
            return Err(ProviderError::AlreadyExists);
        }

        let (cid, size) = self.tree.put(&[]).await.map_err(map_err)?;
        let parent_segs = Self::segments(parent);
        let new_root = self
            .tree
            .set_entry(
                &self.current_root().await,
                &parent_segs,
                name,
                &cid,
                size,
                link_type_for_size(size),
            )
            .await
            .map_err(map_err)?;
        self.apply_new_root(new_root).await?;
        Ok(Item {
            id,
            name: name.to_string(),
            kind: ItemKind::File,
            size,
        })
    }

    async fn create_dir(
        &self,
        parent: &Self::ItemId,
        name: &str,
    ) -> Result<Item<Self::ItemId>, ProviderError> {
        Self::ensure_valid_name(name)?;
        let _guard = self.modify_lock.lock().await;

        let id = Self::join(parent, name);
        if self.resolve(&id).await.is_ok() {
            return Err(ProviderError::AlreadyExists);
        }

        let dir_cid = self.tree.put_directory(Vec::new()).await.map_err(map_err)?;
        let parent_segs = Self::segments(parent);
        let new_root = self
            .tree
            .set_entry(
                &self.current_root().await,
                &parent_segs,
                name,
                &dir_cid,
                0,
                LinkType::Dir,
            )
            .await
            .map_err(map_err)?;
        self.apply_new_root(new_root).await?;
        Ok(Item {
            id,
            name: name.to_string(),
            kind: ItemKind::Directory,
            size: 0,
        })
    }

    async fn write(
        &self,
        id: &Self::ItemId,
        offset: u64,
        data: &[u8],
    ) -> Result<u32, ProviderError> {
        let _guard = self.modify_lock.lock().await;
        let resolved = self.resolve(id).await?;
        if resolved.link_type == LinkType::Dir {
            return Err(ProviderError::IsDir);
        }
        // Read existing, apply, put.
        let existing = self.read_full(&resolved).await?;
        let new_bytes = apply_write(existing, offset, data);
        self.replace_file_bytes(id, &new_bytes).await?;
        Ok(data.len() as u32)
    }

    async fn truncate(&self, id: &Self::ItemId, size: u64) -> Result<(), ProviderError> {
        let _guard = self.modify_lock.lock().await;
        let resolved = self.resolve(id).await?;
        if resolved.link_type == LinkType::Dir {
            return Err(ProviderError::IsDir);
        }
        let existing = self.read_full(&resolved).await?;
        let new_bytes = apply_truncate(existing, size);
        self.replace_file_bytes(id, &new_bytes).await?;
        Ok(())
    }

    async fn remove(&self, parent: &Self::ItemId, name: &str) -> Result<(), ProviderError> {
        Self::ensure_valid_name(name)?;
        let _guard = self.modify_lock.lock().await;

        let id = Self::join(parent, name);
        let resolved = self.resolve(&id).await?;
        if resolved.link_type == LinkType::Dir {
            let listing = self
                .tree
                .list_directory(&resolved.cid)
                .await
                .map_err(map_err)?;
            if !listing.is_empty() {
                return Err(ProviderError::NotEmpty);
            }
        }
        let parent_segs = Self::segments(parent);
        let new_root = self
            .tree
            .remove_entry(&self.current_root().await, &parent_segs, name)
            .await
            .map_err(map_err)?;
        self.apply_new_root(new_root).await?;
        Ok(())
    }

    async fn rename(
        &self,
        old_parent: &Self::ItemId,
        old_name: &str,
        new_parent: &Self::ItemId,
        new_name: &str,
    ) -> Result<(), ProviderError> {
        Self::ensure_valid_name(old_name)?;
        Self::ensure_valid_name(new_name)?;
        if old_parent == new_parent && old_name == new_name {
            return Ok(());
        }
        let _guard = self.modify_lock.lock().await;

        let old_id = Self::join(old_parent, old_name);
        let resolved = self.resolve(&old_id).await?;

        let new_id = Self::join(new_parent, new_name);
        if self.resolve(&new_id).await.is_ok() {
            return Err(ProviderError::AlreadyExists);
        }

        let new_parent_segs = Self::segments(new_parent);
        let mut new_root = self
            .tree
            .set_entry(
                &self.current_root().await,
                &new_parent_segs,
                new_name,
                &resolved.cid,
                resolved.size,
                resolved.link_type,
            )
            .await
            .map_err(map_err)?;
        let old_parent_segs = Self::segments(old_parent);
        new_root = self
            .tree
            .remove_entry(&new_root, &old_parent_segs, old_name)
            .await
            .map_err(map_err)?;
        self.apply_new_root(new_root).await?;
        Ok(())
    }

    async fn anchor(&self) -> SyncAnchor {
        SyncAnchor::from_cid(&self.current_root().await)
    }

    async fn changes_since(
        &self,
        anchor: Option<&SyncAnchor>,
    ) -> Result<Vec<PathChange>, ProviderError> {
        let old = match anchor {
            Some(a) => Some(
                Cid::parse(a.as_str())
                    .map_err(|e| ProviderError::Backend(format!("bad anchor: {}", e)))?,
            ),
            None => None,
        };
        let new = self.current_root().await;
        path_diff(&self.tree, old.as_ref(), &new)
            .await
            .map_err(map_err)
    }
}

impl<S: Store> HashTreeProviderFs<S> {
    async fn read_full(&self, resolved: &ResolvedEntry) -> Result<Vec<u8>, ProviderError> {
        if resolved.size == 0 {
            return Ok(Vec::new());
        }
        if resolved.cid.key.is_some() {
            return Ok(self
                .tree
                .get(&resolved.cid, None)
                .await
                .map_err(map_err)?
                .unwrap_or_default());
        }
        Ok(self
            .tree
            .read_file_range(&resolved.cid.hash, 0, None)
            .await
            .map_err(map_err)?
            .unwrap_or_default())
    }

    async fn replace_file_bytes(&self, id: &str, bytes: &[u8]) -> Result<(), ProviderError> {
        let (parent_segs, name) = Self::split_path(id)?;
        let (cid, size) = self.tree.put(bytes).await.map_err(map_err)?;
        let new_root = self
            .tree
            .set_entry(
                &self.current_root().await,
                &parent_segs,
                name,
                &cid,
                size,
                link_type_for_size(size),
            )
            .await
            .map_err(map_err)?;
        self.apply_new_root(new_root).await
    }
}

fn apply_write(mut existing: Vec<u8>, offset: u64, data: &[u8]) -> Vec<u8> {
    let off = offset as usize;
    if existing.len() < off + data.len() {
        existing.resize(off + data.len(), 0);
    }
    existing[off..off + data.len()].copy_from_slice(data);
    existing
}

fn apply_truncate(mut existing: Vec<u8>, size: u64) -> Vec<u8> {
    existing.resize(size as usize, 0);
    existing
}
