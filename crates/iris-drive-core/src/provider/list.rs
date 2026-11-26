//! Single-pass provider directory listing and timestamp projection.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use anyhow::Result;
use hashtree_core::{Cid, HashTree, HashTreeError, LinkType, Store};

use super::{
    ProviderListEntry, provider_collision_family_path, provider_file_probable_os_placeholder_family,
};
use crate::merge::MODIFIED_AT_META_KEY;
use crate::projection::PrimaryMergedView;

const MIN_PROVIDER_DISPLAY_UNIX_SECS: i64 = 946_684_800;

/// Project per-file modification times onto each file and its ancestor folders.
#[must_use]
pub fn provider_modified_at_index(view: &PrimaryMergedView) -> BTreeMap<String, i64> {
    let mut index = BTreeMap::new();
    for entry in &view.view.files {
        let Some(modified_at) = entry.modified_at else {
            continue;
        };
        remember_provider_modified_at(&mut index, &entry.path, modified_at);
        let mut path = entry.path.as_str();
        while let Some((parent, _name)) = path.rsplit_once('/') {
            remember_provider_modified_at(&mut index, parent, modified_at);
            path = parent;
        }
    }
    index
}

fn remember_provider_modified_at(index: &mut BTreeMap<String, i64>, path: &str, modified_at: i64) {
    if path.is_empty() || modified_at < MIN_PROVIDER_DISPLAY_UNIX_SECS {
        return;
    }
    index
        .entry(path.to_owned())
        .and_modify(|existing| *existing = (*existing).max(modified_at))
        .or_insert(modified_at);
}

fn modified_at_from_meta(meta: Option<&HashMap<String, serde_json::Value>>) -> Option<i64> {
    meta.and_then(|meta| meta.get(MODIFIED_AT_META_KEY))
        .and_then(serde_json::Value::as_i64)
        .filter(|value| *value >= MIN_PROVIDER_DISPLAY_UNIX_SECS)
}

fn entry_from_tree_entry(
    parent: &str,
    child: hashtree_core::TreeEntry,
    modified_at_by_path: Option<&BTreeMap<String, i64>>,
) -> (ProviderListEntry, Option<Cid>) {
    let path = if parent.is_empty() {
        child.name.clone()
    } else {
        format!("{parent}/{}", child.name)
    };
    let cid = Cid {
        hash: child.hash,
        key: child.key,
    };
    let (kind, dir_cid) = match child.link_type {
        LinkType::Dir | LinkType::Fanout => ("directory", Some(cid.clone())),
        LinkType::Blob | LinkType::File => ("file", None),
    };
    let modified_at = modified_at_by_path
        .and_then(|map| map.get(&path).copied())
        .or_else(|| modified_at_from_meta(child.meta.as_ref()));
    (
        ProviderListEntry {
            path,
            parent_path: parent.to_owned(),
            display_name: child.name,
            kind,
            size: child.size,
            version: cid.to_string(),
            modified_at,
        },
        dir_cid,
    )
}

/// Walk a provider root once per directory, returning every visible entry sorted
/// by canonical path with probable OS empty placeholder copies filtered out.
pub async fn list_provider_entries<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    modified_at_by_path: &BTreeMap<String, i64>,
) -> Result<Vec<ProviderListEntry>> {
    let mut entries = Vec::new();
    let mut stack = vec![(String::new(), root.clone())];
    while let Some((parent, dir_cid)) = stack.pop() {
        for child in tree.list_directory_required(&dir_cid).await? {
            let (entry, sub_dir) = entry_from_tree_entry(&parent, child, Some(modified_at_by_path));
            if let Some(sub_cid) = sub_dir {
                stack.push((entry.path.clone(), sub_cid));
            }
            entries.push(entry);
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    filter_probable_os_placeholders(&mut entries);
    Ok(entries)
}

/// List only the direct children of `parent_path` (empty string for root),
/// avoiding a whole-drive walk for single-folder collision and placeholder
/// checks. Returns empty if `parent_path` does not exist yet.
pub async fn list_provider_dir_entries<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    parent_path: &str,
) -> Result<Vec<ProviderListEntry>> {
    let mut current = root.clone();
    for segment in parent_path.split('/').filter(|part| !part.is_empty()) {
        let children = tree.list_directory_required(&current).await?;
        let Some(next) = children.into_iter().find(|entry| entry.name == segment) else {
            return Ok(Vec::new());
        };
        if !matches!(next.link_type, LinkType::Dir | LinkType::Fanout) {
            return Ok(Vec::new());
        }
        current = Cid {
            hash: next.hash,
            key: next.key,
        };
    }

    let children = match tree.list_directory_required(&current).await {
        Ok(children) => children,
        Err(HashTreeError::PathNotFound(_)) => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let mut entries = children
        .into_iter()
        .map(|child| entry_from_tree_entry(parent_path, child, None).0)
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    filter_probable_os_placeholders(&mut entries);
    Ok(entries)
}

fn filter_probable_os_placeholders(entries: &mut Vec<ProviderListEntry>) {
    // Placeholders are rare; skip building the family set unless one exists.
    if !entries.iter().any(|entry| {
        entry.kind == "file"
            && provider_file_probable_os_placeholder_family(&entry.path, entry.size).is_some()
    }) {
        return;
    }
    let non_empty_families = entries
        .iter()
        .filter(|entry| entry.kind == "file" && entry.size > 0)
        .map(|entry| provider_collision_family_path(&entry.path).0)
        .collect::<BTreeSet<_>>();
    entries.retain(|entry| {
        if entry.kind != "file" {
            return true;
        }
        let Some(family) = provider_file_probable_os_placeholder_family(&entry.path, entry.size)
        else {
            return true;
        };
        !non_empty_families.contains(&family)
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use hashtree_core::{DirEntry, Hash, HashTreeConfig, MemoryStore, StoreError};

    use super::*;
    use crate::merge::{MergedEntry, MergedView};

    struct CountingStore {
        inner: MemoryStore,
        gets: AtomicUsize,
    }

    #[async_trait]
    impl Store for CountingStore {
        async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
            self.inner.put(hash, data).await
        }
        async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
            self.gets.fetch_add(1, Ordering::Relaxed);
            self.inner.get(hash).await
        }
        async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
            self.inner.has(hash).await
        }
        async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
            self.inner.delete(hash).await
        }
    }

    #[tokio::test]
    async fn listing_reads_each_directory_once_and_dir_list_skips_siblings() {
        let store = Arc::new(CountingStore {
            inner: MemoryStore::new(),
            gets: AtomicUsize::new(0),
        });
        let tree = HashTree::new(HashTreeConfig::new(store.clone()));

        let mut photo_entries = Vec::new();
        for i in 0..200 {
            let (cid, size) = tree.put(format!("p{i}").as_bytes()).await.unwrap();
            photo_entries.push(
                DirEntry::from_cid(format!("IMG_{i:04}.jpg"), &cid)
                    .with_size(size)
                    .with_link_type(LinkType::Blob),
            );
        }
        let photos_dir = tree.put_directory(photo_entries).await.unwrap();

        let (doc_cid, doc_size) = tree.put(b"doc").await.unwrap();
        let docs_dir = tree
            .put_directory(vec![
                DirEntry::from_cid("readme.txt", &doc_cid)
                    .with_size(doc_size)
                    .with_link_type(LinkType::Blob),
            ])
            .await
            .unwrap();
        let root = tree
            .put_directory(vec![
                DirEntry::from_cid("Photos", &photos_dir).with_link_type(LinkType::Dir),
                DirEntry::from_cid("Docs", &docs_dir).with_link_type(LinkType::Dir),
            ])
            .await
            .unwrap();

        store.gets.store(0, Ordering::Relaxed);
        let all = list_provider_entries(&tree, &root, &BTreeMap::new())
            .await
            .unwrap();
        let full_gets = store.gets.load(Ordering::Relaxed);
        assert_eq!(all.len(), 203);
        // Root + Photos (with at most small fanout) + Docs, never once per file.
        assert!(
            full_gets <= 10,
            "listing 200 files should read each directory once, did {full_gets} store reads"
        );

        store.gets.store(0, Ordering::Relaxed);
        let docs_only = list_provider_dir_entries(&tree, &root, "Docs")
            .await
            .unwrap();
        let dir_gets = store.gets.load(Ordering::Relaxed);
        assert_eq!(docs_only.len(), 1);
        assert_eq!(docs_only[0].path, "Docs/readme.txt");
        assert_eq!(dir_gets, 2, "only root and Docs should be read");

        assert_eq!(
            list_provider_dir_entries(&tree, &root, "Missing/Sub")
                .await
                .unwrap(),
            Vec::<ProviderListEntry>::new()
        );
    }

    #[test]
    fn provider_modified_at_index_ignores_root_published_at_and_epoch_sentinel() {
        let view = PrimaryMergedView {
            view: MergedView {
                files: vec![
                    MergedEntry {
                        path: "legacy.txt".to_string(),
                        source_path: None,
                        hash: [1; 32],
                        size: 1,
                        whole_file_hash: None,
                        modified_at: None,
                        source_app_key_pubkey: "device".to_string(),
                        published_at: 1_800_000_000,
                    },
                    MergedEntry {
                        path: "epoch.txt".to_string(),
                        source_path: None,
                        hash: [2; 32],
                        size: 1,
                        whole_file_hash: None,
                        modified_at: Some(1),
                        source_app_key_pubkey: "device".to_string(),
                        published_at: 1_800_000_000,
                    },
                    MergedEntry {
                        path: "docs/current.txt".to_string(),
                        source_path: None,
                        hash: [3; 32],
                        size: 2,
                        whole_file_hash: None,
                        modified_at: Some(1_700_000_000),
                        source_app_key_pubkey: "device".to_string(),
                        published_at: 1_800_000_000,
                    },
                ],
                ..MergedView::default()
            },
            ..Default::default()
        };

        let index = provider_modified_at_index(&view);

        assert!(!index.contains_key("legacy.txt"));
        assert!(!index.contains_key("epoch.txt"));
        assert_eq!(index.get("docs"), Some(&1_700_000_000));
        assert_eq!(index.get("docs/current.txt"), Some(&1_700_000_000));
    }
}
