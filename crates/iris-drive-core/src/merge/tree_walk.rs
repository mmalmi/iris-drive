//! Walk one app key's stored root into files, tombstones and, on request,
//! the user directories and directory listings seen on the way.

use std::collections::HashMap;

use hashtree_core::{Cid, HashTree, HashTreeError, LinkType, Store, TreeEntry, from_hex, to_hex};

use super::{
    AppKeyFileEntry, AppKeyTombstone, CONFLICTS_PREFIX, META_DIR, MODIFIED_AT_META_KEY,
    PATH_KIND_REPLACEMENTS_PATH, PREV_LINK_PATH, ROOT_META_PATH, TOMBSTONE_PREFIX,
    WHOLE_FILE_HASH_META_KEY, original_path_from_tombstone,
};
use crate::indexer::should_ignore_name;

type LinkMeta = HashMap<String, serde_json::Value>;

/// The user-visible directory structure of one root, gathered by the same
/// walk that collects its files.
#[derive(Debug, Default)]
pub(crate) struct UserTreeIndex {
    /// Every user directory in walk order with the metadata on its link.
    pub(crate) dirs: Vec<(String, LinkMeta)>,
    /// Entries of each directory a path lookup can reach, by directory path
    /// (`""` for the root) and then by name. A name maps to the first entry
    /// carrying it, which is the one a lookup returns.
    pub(crate) listings: HashMap<String, HashMap<String, TreeEntry>>,
}

/// Walk an htree directory root and partition its contents into
/// regular files and tombstones. Tombstone leaves (under `.tombstones/`)
/// are decoded by parsing their content as a unix-seconds integer; any
/// leaf whose content can't be parsed is silently skipped.
pub async fn walk_app_key_tree<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
) -> Result<(Vec<AppKeyFileEntry>, Vec<AppKeyTombstone>), HashTreeError> {
    let mut walk = Walk::new(tree, false);
    walk.dir(root, "", Some(true)).await?;
    Ok((walk.files, walk.tombstones))
}

/// [`walk_app_key_tree`] that also returns the directory structure, so a
/// caller needing both does not list the tree again.
pub(crate) async fn walk_app_key_tree_indexed<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
) -> Result<(Vec<AppKeyFileEntry>, Vec<AppKeyTombstone>, UserTreeIndex), HashTreeError> {
    let mut walk = Walk::new(tree, true);
    walk.dir(root, "", Some(true)).await?;
    Ok((walk.files, walk.tombstones, walk.index.unwrap_or_default()))
}

struct Walk<'a, S: Store> {
    tree: &'a HashTree<S>,
    files: Vec<AppKeyFileEntry>,
    tombstones: Vec<AppKeyTombstone>,
    index: Option<UserTreeIndex>,
}

impl<'a, S: Store> Walk<'a, S> {
    fn new(tree: &'a HashTree<S>, indexed: bool) -> Self {
        Self {
            tree,
            files: Vec::new(),
            tombstones: Vec::new(),
            index: indexed.then(UserTreeIndex::default),
        }
    }

    /// Walk inside `.hashtree/` collecting tombstones and ignoring other
    /// structural metadata.
    /// Lifted out so the main walker doesn't need to know `META_DIR`
    /// internals.
    fn meta_dir<'w>(
        &'w mut self,
        dir_cid: &'w Cid,
        prefix: &'w str,
    ) -> futures::future::BoxFuture<'w, Result<(), HashTreeError>> {
        Box::pin(async move {
            let entries = self.tree.list_directory_required(dir_cid).await?;
            for entry in entries {
                let path = format!("{prefix}/{}", entry.name);
                // Structural metadata is not user-visible content. Only
                // the tombstone subtree is intentionally traversed.
                if path == PREV_LINK_PATH
                    || path == ROOT_META_PATH
                    || path == CONFLICTS_PREFIX
                    || path == PATH_KIND_REPLACEMENTS_PATH
                {
                    continue;
                }
                let child_cid = Cid {
                    hash: entry.hash,
                    key: entry.key,
                };
                if entry.link_type == LinkType::Dir && path == TOMBSTONE_PREFIX {
                    // The tombstones subtree mirrors original paths; recurse
                    // and let the tombstone-leaf check below pick them up.
                    self.dir(&child_cid, &path, None).await?;
                }
            }
            Ok(())
        })
    }

    /// `user_dir` is `None` inside `.hashtree/`. Otherwise it says whether a
    /// path lookup from the root reaches this directory: every ancestor link
    /// is the first entry carrying its name and that name is one path
    /// segment.
    fn dir<'w>(
        &'w mut self,
        dir_cid: &'w Cid,
        prefix: &'w str,
        user_dir: Option<bool>,
    ) -> futures::future::BoxFuture<'w, Result<(), HashTreeError>> {
        Box::pin(async move {
            let entries = self.tree.list_directory_required(dir_cid).await?;
            let indexed = if self.index.is_some() { user_dir } else { None };
            let first_by_name: HashMap<&str, usize> = if indexed == Some(true) {
                let mut first = HashMap::with_capacity(entries.len());
                for (position, entry) in entries.iter().enumerate() {
                    first.entry(entry.name.as_str()).or_insert(position);
                }
                first
            } else {
                HashMap::new()
            };
            for (position, entry) in entries.iter().enumerate() {
                let path = if prefix.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{prefix}/{}", entry.name)
                };
                let child_cid = Cid {
                    hash: entry.hash,
                    key: entry.key,
                };
                if entry.name == META_DIR && prefix.is_empty() {
                    // `.hashtree/` subtree carries htree-format metadata
                    // (revision back-link, tombstones, ...). Recurse so the
                    // tombstone walker can pick up entries under
                    // `.hashtree/tombstones/`, but skip the `prev` link.
                    self.meta_dir(&child_cid, META_DIR).await?;
                    continue;
                }
                if should_ignore_name(&entry.name) {
                    continue;
                }
                if entry.link_type == LinkType::Dir {
                    let child_user_dir = indexed.map(|reachable| {
                        reachable
                            && !entry.name.is_empty()
                            && !entry.name.contains('/')
                            && first_by_name.get(entry.name.as_str()) == Some(&position)
                    });
                    if indexed.is_some()
                        && let Some(index) = self.index.as_mut()
                    {
                        index
                            .dirs
                            .push((path.clone(), entry.meta.clone().unwrap_or_default()));
                    }
                    self.dir(&child_cid, &path, child_user_dir).await?;
                } else if let Some(orig_path) = original_path_from_tombstone(&path) {
                    let raw = self
                        .tree
                        .get(&child_cid, None)
                        .await?
                        .ok_or_else(|| HashTreeError::MissingChunk(to_hex(&entry.hash)))?;
                    let ts_str = String::from_utf8_lossy(&raw);
                    if let Ok(tombstoned_at) = ts_str.trim().parse::<i64>() {
                        self.tombstones.push(AppKeyTombstone {
                            path: orig_path.to_string(),
                            tombstoned_at,
                        });
                    } else {
                        tracing::warn!("malformed tombstone at {path}: {ts_str:?}");
                    }
                } else {
                    self.files.push(AppKeyFileEntry {
                        path,
                        hash: entry.hash,
                        size: entry.size,
                        whole_file_hash: whole_file_hash_from_meta(entry.meta.as_ref()),
                        modified_at: modified_at_from_meta(entry.meta.as_ref()),
                    });
                }
            }
            if indexed == Some(true)
                && let Some(index) = self.index.as_mut()
            {
                drop(first_by_name);
                let mut by_name = HashMap::with_capacity(entries.len());
                for entry in entries {
                    by_name.entry(entry.name.clone()).or_insert(entry);
                }
                index.listings.insert(prefix.to_string(), by_name);
            }
            Ok(())
        })
    }
}

fn whole_file_hash_from_meta(meta: Option<&LinkMeta>) -> Option<[u8; 32]> {
    meta.and_then(|m| m.get(WHOLE_FILE_HASH_META_KEY))
        .and_then(serde_json::Value::as_str)
        .and_then(|s| from_hex(s).ok())
}

fn modified_at_from_meta(meta: Option<&LinkMeta>) -> Option<i64> {
    meta.and_then(|m| m.get(MODIFIED_AT_META_KEY))
        .and_then(serde_json::Value::as_i64)
        .filter(|value| *value > 0)
}
