use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, ensure};
use hashtree_core::{Cid, DirEntry, HashTree, LinkType, Store, TreeEntry};
use nostr_sdk::Event;
use serde::{Deserialize, Serialize};

use super::{FileReturn, OWNER_META, active_profile, entry_cid, read_owner, read_request, wire};
use crate::{SharedFolder, merge::META_DIR, shared_folder_app_key_can_write_roots};

pub(super) const DIR: &str = "file-returns-v1";
const MAX_MARKER_BYTES: u64 = 128 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct RemovalMarker {
    pub v: u8,
    pub claim: Event,
    pub request: Event,
    pub receipt: Event,
}

impl RemovalMarker {
    pub(super) fn validate(&self, folder: &SharedFolder) -> Result<FileReturn> {
        let claim = read_owner(&self.claim)?;
        let request = read_request(&self.request)?;
        wire::verify_receipt(&self.receipt, &self.request)?;
        ensure!(
            self.v == 1
                && claim.share == folder.share_id
                && request.share == claim.share
                && request.claim == self.claim.id.to_hex()
                && request.item == claim.item
                && claim.owner == request.owner,
            "Return marker belongs to another file"
        );
        let editor = self.request.pubkey.to_hex();
        ensure!(
            active_profile(folder, &editor).is_some()
                && shared_folder_app_key_can_write_roots(folder, &editor)
                && active_profile(folder, &self.claim.pubkey.to_hex()) == Some(claim.owner)
                && active_profile(folder, &self.receipt.pubkey.to_hex()) == Some(claim.owner),
            "Return marker signers require verified share membership"
        );
        Ok(request)
    }
}

struct Predicate {
    claim: Event,
    request: FileReturn,
}

#[derive(Default)]
pub(crate) struct Removals(BTreeMap<String, Vec<Predicate>>);

impl Removals {
    pub(crate) async fn read<S: Store>(
        tree: &HashTree<S>,
        folder: &SharedFolder,
        roots: Vec<Cid>,
    ) -> Result<Self, hashtree_core::HashTreeError> {
        let mut index = BTreeMap::<String, Vec<Predicate>>::new();
        let dirs = marker_directories(tree, roots).await?;
        for dir in dirs {
            for entry in tree.list_directory_required(&dir).await? {
                if let Some((marker, request)) = read_marker(tree, folder, &entry).await? {
                    index
                        .entry(request.path.join("/"))
                        .or_default()
                        .push(Predicate {
                            claim: marker.claim,
                            request,
                        });
                }
            }
        }
        Ok(Self(index))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub(crate) fn matches(&self, path: &str, entry: &TreeEntry, writer: &str, seq: u64) -> bool {
        let (Some(predicates), Ok(Some(claim)), Ok(revision)) = (
            self.0.get(path),
            wire::signed_meta_at(entry, OWNER_META),
            wire::entry_revision(entry),
        ) else {
            return false;
        };
        predicates.iter().any(|predicate| {
            predicate.claim == claim
                && (predicate.request.revision == revision
                    || (seq > 0
                        && predicate
                            .request
                            .seen
                            .get(writer)
                            .is_some_and(|seen| seq <= *seen)))
        })
    }
}

async fn marker_directories<S: Store>(
    tree: &HashTree<S>,
    roots: Vec<Cid>,
) -> Result<Vec<Cid>, hashtree_core::HashTreeError> {
    let mut dirs = Vec::new();
    let mut seen = BTreeSet::new();
    for root in roots {
        if let Some(dir) = tree.resolve(&root, &format!("{META_DIR}/{DIR}")).await?
            && dir.key.is_some()
            && seen.insert(dir.to_string())
        {
            dirs.push(dir);
        }
    }
    Ok(dirs)
}

async fn read_marker<S: Store>(
    tree: &HashTree<S>,
    folder: &SharedFolder,
    entry: &TreeEntry,
) -> Result<Option<(RemovalMarker, FileReturn)>, hashtree_core::HashTreeError> {
    if entry.link_type.is_directory_like() || entry.key.is_none() || entry.size > MAX_MARKER_BYTES {
        return Ok(None);
    }
    let Some(bytes) = tree.get(&entry_cid(entry), Some(MAX_MARKER_BYTES)).await? else {
        return Ok(None);
    };
    let Ok(marker) = serde_json::from_slice::<RemovalMarker>(&bytes) else {
        return Ok(None);
    };
    if entry.name != format!("{}.json", marker.request.id.to_hex()) {
        return Ok(None);
    }
    let Ok(request) = marker.validate(folder) else {
        return Ok(None);
    };
    Ok(Some((marker, request)))
}

pub(crate) async fn attach<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    folder: &SharedFolder,
    marker: Option<&RemovalMarker>,
) -> Result<Cid> {
    let roots = crate::shared_folder_authorized_writer_pubkeys(folder)
        .into_iter()
        .filter_map(|key| folder.app_key_roots.get(&key))
        .map(|root| Cid::parse(&root.root_cid))
        .collect::<Result<Vec<_>, _>>()?;
    let dirs = marker_directories(tree, roots).await?;
    let mut root = root.clone();
    match dirs.as_slice() {
        [] => {}
        [dir] => {
            root = tree
                .set_entry(&root, &[META_DIR], DIR, dir, 0, LinkType::Dir)
                .await?;
        }
        _ => {
            // Carry the union forward without replacing a valid record with
            // an unverified leaf using the same name.
            let mut entries = BTreeMap::new();
            for dir in dirs {
                for entry in tree.list_directory_required(&dir).await? {
                    if !entries.contains_key(&entry.name)
                        && read_marker(tree, folder, &entry).await?.is_some()
                    {
                        let link = DirEntry::from_cid(&entry.name, &entry_cid(&entry))
                            .with_size(entry.size)
                            .with_link_type(entry.link_type);
                        entries.insert(entry.name, link);
                    }
                }
            }
            let dir = tree.put_directory(entries.into_values().collect()).await?;
            root = tree
                .set_entry(&root, &[META_DIR], DIR, &dir, 0, LinkType::Dir)
                .await?;
        }
    }
    if let Some(marker) = marker {
        marker.validate(folder)?;
        let (cid, size) = tree.put(&serde_json::to_vec(marker)?).await?;
        ensure!(
            cid.key.is_some() && size <= MAX_MARKER_BYTES,
            "Shared return proof must be an encrypted small file"
        );
        let name = format!("{}.json", marker.request.id.to_hex());
        if tree
            .resolve(&root, &format!("{META_DIR}/{DIR}"))
            .await?
            .is_some()
        {
            root = tree
                .set_entry(&root, &[META_DIR, DIR], &name, &cid, size, LinkType::Blob)
                .await?;
        } else {
            let dir = tree
                .put_directory(vec![DirEntry::from_cid(&name, &cid).with_size(size)])
                .await?;
            root = tree
                .set_entry(&root, &[META_DIR], DIR, &dir, 0, LinkType::Dir)
                .await?;
        }
    }
    Ok(root)
}
