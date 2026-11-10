//! The source roots behind a merged view, each walked once.

use std::collections::BTreeMap;
use std::sync::Arc;

use hashtree_core::{Cid, HashTree, HashTreeError, Store, TreeEntry};

use super::path_kind::{MergedDirectory, insert_directory_candidate};
use super::{ProjectionError, merge_root_for_device, merged_entry_source_path, split_visible_path};
use crate::config::{AppKeyRootRef, Drive};
use crate::merge::{
    AppKeyFileEntry, AppKeyTombstone, MergedEntry, UserTreeIndex, walk_app_key_tree_indexed,
};

/// Files and tombstones of one source root, in merge order.
pub(super) struct SourceContents {
    pub(super) app_key_pubkey: String,
    pub(super) cid: Cid,
    pub(super) files: Vec<AppKeyFileEntry>,
    pub(super) tombstones: Vec<AppKeyTombstone>,
}

/// The resolved source root of every merged app key with its directories and
/// directory listings.
pub(super) struct DriveSources {
    /// What the roots were resolved from: each merge app key and the root the
    /// drive recorded for it.
    inputs: Vec<(String, Option<AppKeyRootRef>)>,
    pub(super) roots: BTreeMap<String, AppKeyRootRef>,
    trees: BTreeMap<String, UserTreeIndex>,
}

impl DriveSources {
    pub(super) async fn load<S: Store>(
        tree: &HashTree<S>,
        drive: &Drive,
        merge_app_keys: &[String],
    ) -> Result<(Self, Vec<SourceContents>), ProjectionError> {
        let mut sources = Self {
            inputs: source_inputs(drive, merge_app_keys),
            roots: BTreeMap::new(),
            trees: BTreeMap::new(),
        };
        let mut contents = Vec::new();
        for app_key_pubkey in merge_app_keys {
            let Some(root) = drive.app_key_roots.get(app_key_pubkey) else {
                continue;
            };
            let Some(root) =
                merge_root_for_device(tree, app_key_pubkey, root, merge_app_keys).await?
            else {
                continue;
            };
            let cid = Cid::parse(&root.root_cid).map_err(|source| ProjectionError::RootCid {
                app_key_pubkey: app_key_pubkey.clone(),
                root_cid: root.root_cid.clone(),
                source,
            })?;
            let (files, tombstones, index) = walk_app_key_tree_indexed(tree, &cid).await?;
            // A key listed twice keeps its first walk, as the merge does.
            if !sources.roots.contains_key(app_key_pubkey) {
                sources.roots.insert(app_key_pubkey.clone(), root);
                sources.trees.insert(app_key_pubkey.clone(), index);
            }
            contents.push(SourceContents {
                app_key_pubkey: app_key_pubkey.clone(),
                cid,
                files,
                tombstones,
            });
        }
        Ok((sources, contents))
    }

    fn resolved_from(&self, drive: &Drive, merge_app_keys: &[String]) -> bool {
        self.inputs.len() == merge_app_keys.len()
            && self
                .inputs
                .iter()
                .zip(merge_app_keys)
                .all(|((app_key_pubkey, root), wanted)| {
                    app_key_pubkey == wanted && root.as_ref() == drive.app_key_roots.get(wanted)
                })
    }

    /// Every user directory across the source roots, one candidate per path.
    pub(super) fn user_directories(
        &self,
        merge_app_keys: &[String],
    ) -> BTreeMap<String, MergedDirectory> {
        let mut dirs = BTreeMap::new();
        for app_key_pubkey in merge_app_keys {
            let (Some(root), Some(index)) = (
                self.roots.get(app_key_pubkey),
                self.trees.get(app_key_pubkey),
            ) else {
                continue;
            };
            for (path, meta) in &index.dirs {
                insert_directory_candidate(
                    &mut dirs,
                    MergedDirectory {
                        path: path.clone(),
                        meta: meta.clone(),
                        source_app_key_pubkey: app_key_pubkey.clone(),
                        source_root: root.clone(),
                    },
                );
            }
        }
        dirs
    }

    /// The source-tree entry behind a merged file.
    pub(super) fn entry_for(&self, entry: &MergedEntry) -> Result<&TreeEntry, ProjectionError> {
        let index = self
            .trees
            .get(&entry.source_app_key_pubkey)
            .ok_or_else(|| HashTreeError::PathNotFound(entry.source_app_key_pubkey.clone()))?;
        let (parent, name) = split_visible_path(merged_entry_source_path(entry))?;
        let parent = parent.join("/");
        let Some(listing) = index.listings.get(&parent) else {
            return Err(HashTreeError::PathNotFound(parent).into());
        };
        listing
            .get(name)
            .ok_or_else(|| HashTreeError::EntryNotFound(name.to_string()).into())
    }
}

fn source_inputs(drive: &Drive, merge_app_keys: &[String]) -> Vec<(String, Option<AppKeyRootRef>)> {
    merge_app_keys
        .iter()
        .map(|app_key_pubkey| {
            (
                app_key_pubkey.clone(),
                drive.app_key_roots.get(app_key_pubkey).cloned(),
            )
        })
        .collect()
}

/// Source roots walked while building a merged view, kept so materializing
/// the provider root from that view does not walk them again. It is a cache:
/// it never affects equality and a view built without it works the same.
#[derive(Clone, Default)]
pub struct SourceCache(Option<Arc<DriveSources>>);

impl SourceCache {
    pub(super) fn new(sources: DriveSources) -> Self {
        Self(Some(Arc::new(sources)))
    }

    /// The cached sources, if they were resolved from this drive's current
    /// roots for these app keys.
    pub(super) fn for_drive(
        &self,
        drive: &Drive,
        merge_app_keys: &[String],
    ) -> Option<Arc<DriveSources>> {
        self.0
            .as_ref()
            .filter(|sources| sources.resolved_from(drive, merge_app_keys))
            .cloned()
    }
}

impl PartialEq for SourceCache {
    fn eq(&self, _other: &Self) -> bool {
        true
    }
}

impl std::fmt::Debug for SourceCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.0.is_some() {
            "SourceCache(walked)"
        } else {
            "SourceCache(empty)"
        })
    }
}
