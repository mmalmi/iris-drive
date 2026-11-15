//! How much of a Drive this device holds: the size of every file in it, as
//! its folders list them, and how much of each file's content is stored
//! here. The same count as Iris Drive web's `src/workers/driveHolding.ts`.
//!
//! Only blocks stored here are read: a folder this device lacks is counted
//! as unread, and a file's content is never fetched. A part counts as held
//! only when it is stored here already. Sizes are the files' own.
//!
//! A folder's hash fixes what is in it, so what was counted for it is kept
//! and used again while it is younger than `max_age`: after an edit only the
//! folders on the changed path are counted again. Only what is held changes
//! under an unchanged hash, as content is fetched or removed.

use std::collections::{HashMap, HashSet};

use futures::StreamExt;
use hashtree_core::{Cid, HashTree, LinkType, Store, TreeEntry, to_hex};

use super::report::DeviceStorageFolder;
use crate::indexer::should_ignore_name;
use crate::merge::META_DIR;

/// Pieces both writers cut files into; a file this size or smaller is one piece.
const CHUNK_SIZE: u64 = hashtree_core::DEFAULT_CHUNK_SIZE as u64;
const CONCURRENCY: usize = 8;
/// A folder is listed when it holds at least this share of the Drive…
pub const FOLDER_SHARE: f64 = 0.02;
/// …down to this many levels, and at most this many folders.
pub const FOLDER_DEPTH: usize = 4;
pub const MAX_FOLDERS: usize = 48;
const MAX_REMEMBERED: usize = 200_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    pub files: u64,
    pub bytes: u64,
    /// Files whose whole content is here.
    pub held_files: u64,
    pub held_bytes: u64,
}

impl Tally {
    fn add(&mut self, other: &Self) {
        self.files += other.files;
        self.bytes += other.bytes;
        self.held_files += other.held_files;
        self.held_bytes += other.held_bytes;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DriveHolding {
    pub total: Tally,
    /// Files at the top of the Drive, in no folder.
    pub loose: Tally,
    /// The folders that make up a real share of the Drive, a folder before
    /// its subfolders, larger ones first.
    pub folders: Vec<DeviceStorageFolder>,
    /// Folders that could not be read: their files are not counted.
    pub unread_folders: u64,
}

#[derive(Debug, Clone)]
struct FolderTally {
    total: Tally,
    loose: Tally,
    /// Folders directly inside, by name and hash.
    subfolders: Vec<(String, [u8; 32])>,
    unread_folders: u64,
    /// Unix milliseconds.
    at: u64,
}

/// What was counted for each folder, by its hash. Kept for as long as the
/// process runs; a profile change starts a new one.
#[derive(Debug, Default)]
pub struct HoldingCache {
    remembered: HashMap<[u8; 32], FolderTally>,
}

/// Measure the Drive at `root`. `max_age_ms` is how old a folder's count may
/// be to be used again; `Some(0)` counts everything again.
pub async fn measure_holding<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    cache: &mut HoldingCache,
    max_age_ms: Option<u64>,
    now_ms: u64,
) -> DriveHolding {
    let mut walk = Walk {
        tree,
        cache,
        max_age_ms,
        now_ms,
    };
    let top = walk.count(root, true).await;
    DriveHolding {
        total: top.total,
        loose: top.loose,
        folders: large_folders(&top, &walk.cache.remembered),
        unread_folders: top.unread_folders,
    }
}

struct Walk<'a, S: Store> {
    tree: &'a HashTree<S>,
    cache: &'a mut HoldingCache,
    max_age_ms: Option<u64>,
    now_ms: u64,
}

impl<S: Store> Walk<'_, S> {
    fn count<'w>(
        &'w mut self,
        cid: &'w Cid,
        top: bool,
    ) -> futures::future::BoxFuture<'w, FolderTally> {
        Box::pin(async move {
            if let Some(known) = self.cache.remembered.get(&cid.hash)
                && self
                    .max_age_ms
                    .is_none_or(|max| self.now_ms.saturating_sub(known.at) <= max)
            {
                return known.clone();
            }
            let mut tally = FolderTally {
                total: Tally::default(),
                loose: Tally::default(),
                subfolders: Vec::new(),
                unread_folders: 0,
                at: self.now_ms,
            };
            let Ok(entries) = self.tree.list_directory_required(cid).await else {
                // Not remembered: it is tried again next time.
                tally.unread_folders = 1;
                return tally;
            };
            let mut files = Vec::new();
            for entry in entries {
                if (top && entry.name == META_DIR) || should_ignore_name(&entry.name) {
                    continue;
                }
                if entry.link_type == LinkType::Dir {
                    let child = Cid {
                        hash: entry.hash,
                        key: entry.key,
                    };
                    let sub = self.count(&child, false).await;
                    tally.total.add(&sub.total);
                    tally.unread_folders += sub.unread_folders;
                    tally.subfolders.push((entry.name, entry.hash));
                } else {
                    files.push(entry);
                }
            }
            let tree = self.tree;
            let mut held = futures::stream::iter(files)
                .map(|entry| async move {
                    let held = held_of(tree, &entry).await;
                    (entry.size, held)
                })
                .buffer_unordered(CONCURRENCY);
            while let Some((size, held)) = held.next().await {
                let file = Tally {
                    files: 1,
                    bytes: size,
                    held_files: u64::from(held >= size),
                    held_bytes: held,
                };
                tally.total.add(&file);
                tally.loose.add(&file);
            }
            if tally.unread_folders == 0 {
                if self.cache.remembered.len() >= MAX_REMEMBERED {
                    self.cache.remembered.clear();
                }
                self.cache.remembered.insert(cid.hash, tally.clone());
            }
            tally
        })
    }
}

/// How much of a file's content is stored here.
async fn held_of<S: Store>(tree: &HashTree<S>, entry: &TreeEntry) -> u64 {
    let cid = Cid {
        hash: entry.hash,
        key: entry.key,
    };
    held_of_part(tree, &cid, entry.size, false).await
}

fn held_of_part<'a, S: Store>(
    tree: &'a HashTree<S>,
    cid: &'a Cid,
    size: u64,
    piece: bool,
) -> futures::future::BoxFuture<'a, u64> {
    Box::pin(async move {
        if !tree.get_store().has(&cid.hash).await.unwrap_or(false) {
            return 0;
        }
        // A file of one piece is that piece; a larger one lists its pieces.
        if piece || size <= CHUNK_SIZE {
            return size;
        }
        let node = match tree.get_node(cid).await {
            Ok(Some(node)) if node.node_type == LinkType::File => node,
            _ => return size,
        };
        let mut held = 0_u64;
        for link in &node.links {
            let part = Cid {
                hash: link.hash,
                key: link.key,
            };
            held += held_of_part(tree, &part, link.size, link.link_type == LinkType::Blob).await;
        }
        held.min(size)
    })
}

/// The folders holding at least `FOLDER_SHARE` of the Drive, a folder before
/// its subfolders.
fn large_folders(
    top: &FolderTally,
    remembered: &HashMap<[u8; 32], FolderTally>,
) -> Vec<DeviceStorageFolder> {
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]
    let least = ((top.total.bytes as f64) * FOLDER_SHARE).max(1.0).ceil() as u64;
    let mut found = Vec::new();
    collect(top, &[], remembered, least, &mut found);
    // The largest, which take in each one's parents, as no subfolder is
    // larger than its folder.
    let mut largest = found.iter().collect::<Vec<_>>();
    largest.sort_by(|a, b| b.bytes.cmp(&a.bytes).then(a.path.len().cmp(&b.path.len())));
    let kept = largest
        .into_iter()
        .take(MAX_FOLDERS)
        .map(|folder| folder.path.clone())
        .collect::<HashSet<_>>();
    let mut children: HashMap<Vec<String>, Vec<&DeviceStorageFolder>> = HashMap::new();
    for folder in found.iter().filter(|folder| kept.contains(&folder.path)) {
        let parent = folder.path[..folder.path.len() - 1].to_vec();
        children.entry(parent).or_default().push(folder);
    }
    let mut ordered = Vec::new();
    place(&[], &mut children, &mut ordered);
    ordered
}

fn collect(
    folder: &FolderTally,
    path: &[String],
    remembered: &HashMap<[u8; 32], FolderTally>,
    least: u64,
    found: &mut Vec<DeviceStorageFolder>,
) {
    if path.len() >= FOLDER_DEPTH {
        return;
    }
    for (name, hash) in &folder.subfolders {
        let Some(sub) = remembered.get(hash) else {
            continue;
        };
        if sub.total.bytes < least {
            continue;
        }
        let mut at = path.to_vec();
        at.push(name.clone());
        found.push(DeviceStorageFolder {
            path: at.clone(),
            bytes: sub.total.bytes,
            held: sub.total.held_bytes,
        });
        collect(sub, &at, remembered, least, found);
    }
}

fn place(
    parent: &[String],
    children: &mut HashMap<Vec<String>, Vec<&DeviceStorageFolder>>,
    ordered: &mut Vec<DeviceStorageFolder>,
) {
    let Some(mut under) = children.remove(parent) else {
        return;
    };
    under.sort_by(|a, b| {
        b.bytes
            .cmp(&a.bytes)
            .then_with(|| a.path.join("\0").cmp(&b.path.join("\0")))
    });
    for folder in under {
        ordered.push(folder.clone());
        place(&folder.path, children, ordered);
    }
}

/// The hex hash of a measured root.
#[must_use]
pub fn root_hex(root: &Cid) -> String {
    to_hex(&root.hash)
}
