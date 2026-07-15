//! Path-level diff between two hashtree directory roots.
//!
//! The existing [`hashtree_core::diff`] module emits the set of new hashes
//! to upload; this module emits the user-facing `(path, change)` events
//! that NSFileProvider / SAF adapters need.

use hashtree_core::{Cid, HashTree, HashTreeError, LinkType, Store};

/// A snapshot of the on-tree metadata for an entry. The optional `key` is
/// present for encrypted-content nodes; consumers can ignore it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryInfo {
    pub hash: [u8; 32],
    pub size: u64,
    pub link_type: LinkType,
    pub key: Option<[u8; 32]>,
}

/// A path-level change between two tree revisions. Paths use forward
/// slashes; the root directory itself is not emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathChange {
    Added {
        path: String,
        entry: EntryInfo,
    },
    Modified {
        path: String,
        old: EntryInfo,
        new: EntryInfo,
    },
    Removed {
        path: String,
        entry: EntryInfo,
    },
}

impl PathChange {
    pub fn path(&self) -> &str {
        match self {
            PathChange::Added { path, .. }
            | PathChange::Modified { path, .. }
            | PathChange::Removed { path, .. } => path,
        }
    }
}

/// Compute path-level changes from `old_root` to `new_root`.
///
/// Both CIDs must point at directory nodes. If `old_root` is `None`, every
/// entry under `new_root` is reported as `Added`.
///
/// Subtree pruning: when two directory CIDs match (including encryption
/// key), the entire subtree is skipped. This makes incremental diffs
/// cheap even on large trees.
///
/// Output ordering: deterministic, lexicographic by path.
pub async fn path_diff<S: Store>(
    tree: &HashTree<S>,
    old_root: Option<&Cid>,
    new_root: &Cid,
) -> Result<Vec<PathChange>, HashTreeError> {
    let mut out = Vec::new();
    diff_dirs(tree, "", old_root, Some(new_root), &mut out).await?;
    out.sort_by(|a, b| a.path().cmp(b.path()));
    Ok(out)
}

fn entry_info_from(name_size_link: (&[u8; 32], u64, LinkType, Option<[u8; 32]>)) -> EntryInfo {
    let (hash, size, link_type, key) = name_size_link;
    EntryInfo {
        hash: *hash,
        size,
        link_type,
        key,
    }
}

fn cids_equivalent(a: &Cid, b: &Cid) -> bool {
    a.hash == b.hash && a.key == b.key
}

fn join_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", prefix, name)
    }
}

async fn list_dir_sorted<S: Store>(
    tree: &HashTree<S>,
    cid: &Cid,
) -> Result<Vec<hashtree_core::TreeEntry>, HashTreeError> {
    let mut entries = tree.list_directory(cid).await?;
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(entries)
}

fn entry_kind_is_dir(e: &hashtree_core::TreeEntry) -> bool {
    e.link_type == LinkType::Dir
}

fn cid_from_entry(e: &hashtree_core::TreeEntry) -> Cid {
    Cid {
        hash: e.hash,
        key: e.key,
    }
}

fn entry_info_of(e: &hashtree_core::TreeEntry) -> EntryInfo {
    entry_info_from((&e.hash, e.size, e.link_type, e.key))
}

/// Box the recursive call so the async future has a known size.
fn diff_dirs<'a, S: Store>(
    tree: &'a HashTree<S>,
    prefix: &'a str,
    old: Option<&'a Cid>,
    new: Option<&'a Cid>,
    out: &'a mut Vec<PathChange>,
) -> futures::future::BoxFuture<'a, Result<(), HashTreeError>> {
    Box::pin(async move {
        let old_entries = match old {
            Some(cid) => list_dir_sorted(tree, cid).await?,
            None => Vec::new(),
        };
        let new_entries = match new {
            Some(cid) => list_dir_sorted(tree, cid).await?,
            None => Vec::new(),
        };

        let mut i = 0;
        let mut j = 0;
        while i < old_entries.len() || j < new_entries.len() {
            let take_old = match (old_entries.get(i), new_entries.get(j)) {
                (Some(o), Some(n)) => o.name <= n.name,
                (Some(_), None) => true,
                (None, Some(_)) => false,
                (None, None) => break,
            };
            let take_new = match (old_entries.get(i), new_entries.get(j)) {
                (Some(o), Some(n)) => n.name <= o.name,
                (Some(_), None) => false,
                (None, Some(_)) => true,
                (None, None) => break,
            };

            if take_old && take_new {
                let o = &old_entries[i];
                let n = &new_entries[j];
                let o_cid = cid_from_entry(o);
                let n_cid = cid_from_entry(n);
                let path = join_path(prefix, &n.name);
                let same = cids_equivalent(&o_cid, &n_cid) && o.link_type == n.link_type;
                if !same {
                    let o_is_dir = entry_kind_is_dir(o);
                    let n_is_dir = entry_kind_is_dir(n);
                    if o_is_dir && n_is_dir {
                        diff_dirs(tree, &path, Some(&o_cid), Some(&n_cid), out).await?;
                    } else if !o_is_dir && !n_is_dir {
                        out.push(PathChange::Modified {
                            path,
                            old: entry_info_of(o),
                            new: entry_info_of(n),
                        });
                    } else {
                        // type changed (file -> dir or vice versa). Report
                        // as remove + add so downstream consumers don't
                        // mistakenly treat it as in-place modification.
                        if o_is_dir {
                            emit_removed_recursive(tree, &path, &o_cid, out).await?;
                        } else {
                            out.push(PathChange::Removed {
                                path: path.clone(),
                                entry: entry_info_of(o),
                            });
                        }
                        if n_is_dir {
                            emit_added_recursive(tree, &path, &n_cid, out).await?;
                        } else {
                            out.push(PathChange::Added {
                                path,
                                entry: entry_info_of(n),
                            });
                        }
                    }
                }
                i += 1;
                j += 1;
            } else if take_old {
                let o = &old_entries[i];
                let path = join_path(prefix, &o.name);
                let o_cid = cid_from_entry(o);
                if entry_kind_is_dir(o) {
                    emit_removed_recursive(tree, &path, &o_cid, out).await?;
                } else {
                    out.push(PathChange::Removed {
                        path,
                        entry: entry_info_of(o),
                    });
                }
                i += 1;
            } else {
                let n = &new_entries[j];
                let path = join_path(prefix, &n.name);
                let n_cid = cid_from_entry(n);
                if entry_kind_is_dir(n) {
                    emit_added_recursive(tree, &path, &n_cid, out).await?;
                } else {
                    out.push(PathChange::Added {
                        path,
                        entry: entry_info_of(n),
                    });
                }
                j += 1;
            }
        }
        Ok(())
    })
}

fn emit_added_recursive<'a, S: Store>(
    tree: &'a HashTree<S>,
    prefix: &'a str,
    cid: &'a Cid,
    out: &'a mut Vec<PathChange>,
) -> futures::future::BoxFuture<'a, Result<(), HashTreeError>> {
    Box::pin(async move {
        // Emit the directory itself as Added so consumers can mkdir.
        out.push(PathChange::Added {
            path: prefix.to_string(),
            entry: EntryInfo {
                hash: cid.hash,
                size: 0,
                link_type: LinkType::Dir,
                key: cid.key,
            },
        });
        for e in list_dir_sorted(tree, cid).await? {
            let path = join_path(prefix, &e.name);
            let child_cid = cid_from_entry(&e);
            if entry_kind_is_dir(&e) {
                emit_added_recursive(tree, &path, &child_cid, out).await?;
            } else {
                out.push(PathChange::Added {
                    path,
                    entry: entry_info_of(&e),
                });
            }
        }
        Ok(())
    })
}

fn emit_removed_recursive<'a, S: Store>(
    tree: &'a HashTree<S>,
    prefix: &'a str,
    cid: &'a Cid,
    out: &'a mut Vec<PathChange>,
) -> futures::future::BoxFuture<'a, Result<(), HashTreeError>> {
    Box::pin(async move {
        for e in list_dir_sorted(tree, cid).await? {
            let path = join_path(prefix, &e.name);
            let child_cid = cid_from_entry(&e);
            if entry_kind_is_dir(&e) {
                emit_removed_recursive(tree, &path, &child_cid, out).await?;
            } else {
                out.push(PathChange::Removed {
                    path,
                    entry: entry_info_of(&e),
                });
            }
        }
        out.push(PathChange::Removed {
            path: prefix.to_string(),
            entry: EntryInfo {
                hash: cid.hash,
                size: 0,
                link_type: LinkType::Dir,
                key: cid.key,
            },
        });
        Ok(())
    })
}
