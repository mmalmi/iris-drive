//! Backend-neutral filesystem trait.

use async_trait::async_trait;
use hashtree_core::Cid;

use crate::diff::PathChange;
use crate::error::ProviderError;

/// Opaque sync anchor identifying a particular tree revision. For hashtree
/// backends this wraps a root CID; other backends may use any monotonic
/// identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncAnchor(pub String);

impl SyncAnchor {
    pub fn from_cid(cid: &Cid) -> Self {
        Self(cid.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemKind {
    File,
    Directory,
}

/// A filesystem entry projected for an OS adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item<Id> {
    pub id: Id,
    pub name: String,
    pub kind: ItemKind,
    pub size: u64,
}

/// A child entry in a directory listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirItem<Id> {
    pub id: Id,
    pub name: String,
    pub kind: ItemKind,
}

/// The contract every OS adapter (FUSE, WinFsp, NSFileProvider, SAF) calls
/// into. Implementations own the concrete backing store; the trait is
/// generic over the identifier the implementation chooses to use.
///
/// All methods are async. Implementations should avoid blocking the
/// runtime even for "cheap" lookups, because adapters like NSFileProvider
/// expect prompt completion of fetch/enumerate callbacks.
#[async_trait]
pub trait ProviderFs: Send + Sync {
    /// Identifier type the implementation uses to refer to items. FUSE
    /// adapters typically pick `u64` inodes; NSFileProvider adapters pick
    /// opaque strings.
    type ItemId: Clone + Eq + std::hash::Hash + Send + Sync + std::fmt::Debug;

    /// Identifier of the root directory.
    async fn root(&self) -> Self::ItemId;

    /// Resolve a child by name within a parent.
    async fn lookup(
        &self,
        parent: &Self::ItemId,
        name: &str,
    ) -> Result<Item<Self::ItemId>, ProviderError>;

    /// Fetch attributes for an existing item.
    async fn item(&self, id: &Self::ItemId) -> Result<Item<Self::ItemId>, ProviderError>;

    /// Read bytes from a file.
    async fn read(
        &self,
        id: &Self::ItemId,
        offset: u64,
        size: u64,
    ) -> Result<Vec<u8>, ProviderError>;

    /// Enumerate a directory's children.
    async fn read_dir(
        &self,
        id: &Self::ItemId,
    ) -> Result<Vec<DirItem<Self::ItemId>>, ProviderError>;

    /// Create an empty file under `parent`.
    async fn create_file(
        &self,
        parent: &Self::ItemId,
        name: &str,
    ) -> Result<Item<Self::ItemId>, ProviderError>;

    /// Create a directory under `parent`.
    async fn create_dir(
        &self,
        parent: &Self::ItemId,
        name: &str,
    ) -> Result<Item<Self::ItemId>, ProviderError>;

    /// Write bytes into an existing file. Returns bytes actually written.
    async fn write(
        &self,
        id: &Self::ItemId,
        offset: u64,
        data: &[u8],
    ) -> Result<u32, ProviderError>;

    /// Truncate a file to the given size (grow or shrink).
    async fn truncate(&self, id: &Self::ItemId, size: u64) -> Result<(), ProviderError>;

    /// Remove a child by name from `parent`. Works for files and (empty)
    /// directories.
    async fn remove(&self, parent: &Self::ItemId, name: &str) -> Result<(), ProviderError>;

    /// Rename / move a child.
    async fn rename(
        &self,
        old_parent: &Self::ItemId,
        old_name: &str,
        new_parent: &Self::ItemId,
        new_name: &str,
    ) -> Result<(), ProviderError>;

    /// Current sync anchor identifying the latest revision.
    async fn anchor(&self) -> SyncAnchor;

    /// Path-level changes since the given anchor. `None` enumerates the
    /// full current state as `Added` events — used for first-time sync.
    async fn changes_since(
        &self,
        anchor: Option<&SyncAnchor>,
    ) -> Result<Vec<PathChange>, ProviderError>;
}
