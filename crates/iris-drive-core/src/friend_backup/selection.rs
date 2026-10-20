//! Bounded selection of every encrypted physical block in a live snapshot.

use std::collections::{BTreeMap, HashSet};

use hashtree_core::{
    Cid, Hash, HashTree, Link, LinkType, Store, TreeNode, decode_tree_node, decrypt_chk,
    is_tree_node, sha256,
};
use hashtree_fs::FsBlobStore;

use super::storage::{FriendBackupError, MAX_BLOB_BYTES, MAX_MANIFEST_ENTRIES};

type Result<T> = std::result::Result<T, FriendBackupError>;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum DirectoryScope {
    Root,
    Visible,
    MetadataRoot,
    MetadataChild,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Context {
    Directory(DirectoryScope),
    File(u64),
    Blob(u64),
}

struct Selection {
    keys: BTreeMap<Hash, [u8; 32]>,
    scheduled: HashSet<(Hash, Context)>,
    pending: Vec<(Cid, Context)>,
    limit: usize,
}

impl Selection {
    fn push(&mut self, cid: Cid, context: Context) -> Result<()> {
        let key = cid
            .key
            .ok_or_else(|| invalid("selected backup contains unencrypted content"))?;
        if let Some(existing) = self.keys.get(&cid.hash) {
            if existing != &key {
                return Err(invalid("selected block has inconsistent encryption keys"));
            }
        } else {
            if self.keys.len() >= self.limit {
                return Err(invalid(format!("snapshot exceeds {} blobs", self.limit)));
            }
            self.keys.insert(cid.hash, key);
        }
        if !self.scheduled.contains(&(cid.hash, context)) {
            // A shared subtree can appear in normal and metadata locations. Keep
            // that distinction, without allowing arbitrary duplicate traversal.
            if self.scheduled.len() >= self.limit.saturating_mul(4) {
                return Err(invalid("snapshot exceeds its traversal work limit"));
            }
            self.scheduled.insert((cid.hash, context));
            self.pending.push((cid, context));
        }
        Ok(())
    }
}

pub(super) async fn collect_backup_hashes(
    tree: &HashTree<FsBlobStore>,
    root: &Cid,
) -> Result<Vec<Hash>> {
    collect_with_limit(tree, root, MAX_MANIFEST_ENTRIES).await
}

async fn collect_with_limit(
    tree: &HashTree<FsBlobStore>,
    root: &Cid,
    limit: usize,
) -> Result<Vec<Hash>> {
    let mut selection = Selection {
        keys: BTreeMap::new(),
        scheduled: HashSet::new(),
        pending: Vec::new(),
        limit,
    };
    selection.push(root.clone(), Context::Directory(DirectoryScope::Root))?;
    let store = tree.get_store();
    while let Some((cid, context)) = selection.pending.pop() {
        let size = store
            .blob_size(&cid.hash)
            .await?
            .ok_or_else(|| FriendBackupError::Missing(hex::encode(cid.hash)))?;
        if size > MAX_BLOB_BYTES as u64 {
            return Err(invalid("snapshot contains a blob exceeding 16 MiB"));
        }
        let bytes = store
            .get(&cid.hash)
            .await?
            .ok_or_else(|| FriendBackupError::Missing(hex::encode(cid.hash)))?;
        if bytes.len() > MAX_BLOB_BYTES || sha256(&bytes) != cid.hash {
            return Err(FriendBackupError::InvalidBlob(hex::encode(cid.hash)));
        }
        let plaintext = decrypt_chk(&bytes, &selection.keys[&cid.hash])
            .map_err(|error| invalid(format!("selected backup cannot be decrypted: {error}")))?;
        match context {
            Context::Directory(scope) => {
                let node = decode_tree_node(&plaintext)
                    .map_err(|error| invalid(format!("invalid backup directory: {error}")))?;
                if !node.node_type.is_directory_like() {
                    return Err(invalid("selected directory is not a directory node"));
                }
                let internal = node.node_type == LinkType::Fanout || legacy_fanout(&node);
                for link in node.links {
                    if internal {
                        if !link.link_type.is_directory_like() {
                            return Err(invalid("directory fanout has a non-directory child"));
                        }
                        selection.push(link.to_cid(), Context::Directory(scope))?;
                    } else if let Some(next) = named_context(scope, &link)? {
                        selection.push(link.to_cid(), next)?;
                    }
                }
            }
            Context::Blob(size) if size == plaintext.len() as u64 => {}
            Context::Blob(size) | Context::File(size) => {
                if !is_tree_node(&plaintext) {
                    if size != plaintext.len() as u64 {
                        return Err(invalid("file leaf has the wrong size"));
                    }
                    continue;
                }
                let node = decode_tree_node(&plaintext)
                    .map_err(|error| invalid(format!("invalid backup file node: {error}")))?;
                if node.node_type != LinkType::File {
                    return Err(invalid("selected file contains a non-file tree node"));
                }
                for link in node.links {
                    let next = match link.link_type {
                        LinkType::Blob => Context::Blob(link.size),
                        LinkType::File => Context::File(link.size),
                        _ => return Err(invalid("file node has a directory child")),
                    };
                    selection.push(link.to_cid(), next)?;
                }
            }
        }
    }
    Ok(selection.keys.into_keys().collect())
}

fn named_context(scope: DirectoryScope, link: &Link) -> Result<Option<Context>> {
    let name = link
        .name
        .as_deref()
        .ok_or_else(|| invalid("directory entry has no name"))?;
    if scope == DirectoryScope::MetadataRoot && name == "prev" {
        return Ok(None);
    }
    let next_scope = if scope == DirectoryScope::Root && name == crate::merge::META_DIR {
        DirectoryScope::MetadataRoot
    } else if matches!(
        scope,
        DirectoryScope::MetadataRoot | DirectoryScope::MetadataChild
    ) {
        DirectoryScope::MetadataChild
    } else {
        if crate::indexer::should_ignore_name(name) {
            return Ok(None);
        }
        DirectoryScope::Visible
    };
    Ok(Some(match link.link_type {
        LinkType::Dir | LinkType::Fanout => Context::Directory(next_scope),
        LinkType::File => Context::File(link.size),
        LinkType::Blob => Context::Blob(link.size),
    }))
}

// Hashtree's established legacy directory encoding also uses physical chunks.
fn legacy_fanout(node: &TreeNode) -> bool {
    !node.links.is_empty()
        && node.links.iter().all(|link| {
            link.link_type == LinkType::Dir
                && link
                    .name
                    .as_deref()
                    .and_then(|name| name.strip_prefix("_chunk_"))
                    .is_some_and(|suffix| {
                        !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
                    })
        })
}

fn invalid(message: impl Into<String>) -> FriendBackupError {
    FriendBackupError::Invalid(message.into())
}

#[cfg(test)]
#[path = "selection_tests.rs"]
mod tests;
