//! An in-memory directory tree that is written to a hashtree bottom-up.
//!
//! Editing a stored tree one entry at a time rewrites every ancestor
//! directory per edit, which is quadratic in directory size and leaves each
//! intermediate node behind in the store. Callers that already know the whole
//! result assemble it here and store every directory exactly once.

use std::collections::{BTreeMap, HashMap};

use hashtree_core::{Cid, DirEntry, HashTree, HashTreeError, LinkType, Store, TreeEntry};

pub(crate) type LinkMeta = HashMap<String, serde_json::Value>;

/// One directory under construction. A name lives in `dirs` or `leaves`,
/// never both; callers decide which side gives way on a collision.
#[derive(Default)]
pub(crate) struct DirBuilder {
    pub(crate) dirs: BTreeMap<String, DirBuilder>,
    pub(crate) leaves: BTreeMap<String, LeafLink>,
    /// Metadata on the link that points at this directory from its parent.
    pub(crate) meta: Option<LinkMeta>,
}

/// A link to content that already exists in the store.
pub(crate) struct LeafLink {
    pub(crate) cid: Cid,
    pub(crate) size: u64,
    pub(crate) link_type: LinkType,
    pub(crate) meta: Option<LinkMeta>,
}

impl LeafLink {
    pub(crate) fn from_entry(entry: &TreeEntry) -> Self {
        Self {
            cid: Cid {
                hash: entry.hash,
                key: entry.key,
            },
            size: entry.size,
            link_type: entry.link_type,
            meta: entry.meta.clone(),
        }
    }
}

impl DirBuilder {
    pub(crate) fn is_empty(&self) -> bool {
        self.dirs.is_empty() && self.leaves.is_empty()
    }

    /// Store this directory and everything below it, children first.
    pub(crate) fn put<S: Store>(
        self,
        tree: &HashTree<S>,
    ) -> futures::future::BoxFuture<'_, Result<Cid, HashTreeError>> {
        Box::pin(async move {
            let mut entries = Vec::with_capacity(self.dirs.len() + self.leaves.len());
            for (name, leaf) in self.leaves {
                entries.push(DirEntry {
                    name,
                    hash: leaf.cid.hash,
                    size: leaf.size,
                    key: leaf.cid.key,
                    link_type: leaf.link_type,
                    meta: leaf.meta,
                });
            }
            for (name, mut dir) in self.dirs {
                let meta = dir.meta.take();
                let cid = dir.put(tree).await?;
                entries.push(DirEntry {
                    name,
                    hash: cid.hash,
                    size: 0,
                    key: cid.key,
                    link_type: LinkType::Dir,
                    meta,
                });
            }
            tree.put_directory(entries).await
        })
    }
}
