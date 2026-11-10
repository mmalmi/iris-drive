//! Assemble the merged visible root in memory before storing it.

use hashtree_core::{Cid, HashTreeError, TreeEntry};

use super::{DirectoryMeta, ProjectionError, split_visible_path, visible_entry_meta};
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
