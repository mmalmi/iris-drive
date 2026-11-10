//! Assemble the merged visible root in memory before storing it.

use std::collections::{BTreeMap, HashMap};

use hashtree_core::{Cid, HashTree, HashTreeError, Store, TreeEntry};

use super::{
    DirectoryMeta, ProjectionError, merged_entry_source_path, split_visible_path,
    visible_entry_meta,
};
use crate::config::AppKeyRootRef;
use crate::dir_builder::{DirBuilder, LeafLink};
use crate::merge::MergedEntry;

/// The directory at `segments`, created along with any missing ancestors. A
/// file already holding one of those names is not a directory to descend into.
pub(super) fn ensure_dir<'d>(
    mut dir: &'d mut DirBuilder,
    segments: &[&str],
) -> Result<&'d mut DirBuilder, ProjectionError> {
    for (depth, segment) in segments.iter().enumerate() {
        if dir.leaves.contains_key(*segment) {
            return Err(HashTreeError::PathNotFound(segments[..=depth].join("/")).into());
        }
        dir = dir.dirs.entry((*segment).to_string()).or_default();
    }
    Ok(dir)
}

/// Link a merged file into `dir`, replacing whatever held that name.
pub(super) fn set_file(dir: &mut DirBuilder, name: &str, source: &TreeEntry, merged: &MergedEntry) {
    dir.dirs.remove(name);
    dir.leaves.insert(
        name.to_string(),
        LeafLink {
            cid: Cid {
                hash: source.hash,
                key: source.key,
            },
            size: source.size,
            link_type: source.link_type,
            meta: visible_entry_meta(source, merged),
        },
    );
}

/// Attach link metadata to an existing directory. Empty metadata leaves the
/// tree untouched, even when the directory is no longer present.
pub(super) fn set_dir_meta(
    root: &mut DirBuilder,
    path: &str,
    meta: DirectoryMeta,
) -> Result<(), ProjectionError> {
    if meta.is_empty() {
        return Ok(());
    }
    let (parent, name) = split_visible_path(path)?;
    let mut dir = root;
    for (depth, segment) in parent.iter().enumerate() {
        // A file in the parent position lists as an empty directory.
        if depth + 1 == parent.len() && dir.leaves.contains_key(*segment) {
            return Err(HashTreeError::EntryNotFound(name.to_string()).into());
        }
        dir = dir
            .dirs
            .get_mut(*segment)
            .ok_or_else(|| HashTreeError::PathNotFound(parent.join("/")))?;
    }
    if dir.leaves.contains_key(name) {
        return Err(HashTreeError::PathNotFound(path.to_string()).into());
    }
    dir.dirs
        .get_mut(name)
        .ok_or_else(|| HashTreeError::EntryNotFound(name.to_string()))?
        .meta = Some(meta);
    Ok(())
}

/// Looks up the source-tree entry behind each merged file, listing every
/// source directory at most once.
pub(super) struct SourceEntries<'a, S: Store> {
    tree: &'a HashTree<S>,
    roots: &'a BTreeMap<String, AppKeyRootRef>,
    listings: HashMap<(&'a str, String), HashMap<String, TreeEntry>>,
}

impl<'a, S: Store> SourceEntries<'a, S> {
    pub(super) fn new(tree: &'a HashTree<S>, roots: &'a BTreeMap<String, AppKeyRootRef>) -> Self {
        Self {
            tree,
            roots,
            listings: HashMap::new(),
        }
    }

    pub(super) async fn entry_for(
        &mut self,
        entry: &MergedEntry,
    ) -> Result<&TreeEntry, ProjectionError> {
        let (app_key_pubkey, root) = self
            .roots
            .get_key_value(&entry.source_app_key_pubkey)
            .ok_or_else(|| HashTreeError::PathNotFound(entry.source_app_key_pubkey.clone()))?;
        let (parent, name) = split_visible_path(merged_entry_source_path(entry))?;
        let key = (app_key_pubkey.as_str(), parent.join("/"));
        if !self.listings.contains_key(&key) {
            let root_cid =
                Cid::parse(&root.root_cid).map_err(|source| ProjectionError::RootCid {
                    app_key_pubkey: app_key_pubkey.clone(),
                    root_cid: root.root_cid.clone(),
                    source,
                })?;
            let parent_cid = if parent.is_empty() {
                root_cid
            } else {
                self.tree
                    .resolve(&root_cid, &key.1)
                    .await?
                    .ok_or_else(|| HashTreeError::PathNotFound(key.1.clone()))?
            };
            let mut by_name = HashMap::new();
            for listed in self.tree.list_directory(&parent_cid).await? {
                // A lookup takes the first entry carrying a name.
                by_name.entry(listed.name.clone()).or_insert(listed);
            }
            self.listings.insert(key.clone(), by_name);
        }
        self.listings
            .get(&key)
            .and_then(|by_name| by_name.get(name))
            .ok_or_else(|| HashTreeError::EntryNotFound(name.to_string()).into())
    }
}
