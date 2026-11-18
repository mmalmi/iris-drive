use std::collections::BTreeMap;

use anyhow::{Context, Result, ensure};
use hashtree_core::{Cid, DirEntry, HashTree, LinkType, Store, TreeEntry};
use serde::de::DeserializeOwned;

use super::{CONTENT_DIR, ITEM_FILE, REMOVED_DIR, TrashItem, TrashRemoval, valid_id};
use crate::NostrIdentityId;
use crate::merge::META_DIR;

const MAX_METADATA_BYTES: u64 = 1024 * 1024;

pub(super) struct StoredItem {
    pub item: TrashItem,
    pub content: TreeEntry,
    slot: DirEntry,
}

#[derive(Default)]
pub(super) struct TrashTree {
    pub items: BTreeMap<String, StoredItem>,
    removed: BTreeMap<String, DirEntry>,
}

impl TrashTree {
    pub async fn read<S: Store>(
        tree: &HashTree<S>,
        account: NostrIdentityId,
        heads: &[Cid],
    ) -> Result<Self> {
        let mut result = Self::default();
        let mut directories = Vec::with_capacity(heads.len());
        for head in heads {
            ensure!(head.key.is_some(), "Trash root is unencrypted");
            let entries = tree.list_directory_required(head).await?;
            if let Some(removed) = entries.iter().find(|entry| entry.name == REMOVED_DIR) {
                for entry in tree.list_directory_required(&entry_cid(removed)).await? {
                    let marker: TrashRemoval = read_json(tree, &entry).await?;
                    marker.validate(&entry.name)?;
                    result.removed.insert(marker.id, dir_entry(&entry));
                }
            }
            directories.push(entries);
        }
        for entries in directories {
            for slot in entries {
                if matches!(slot.name.as_str(), META_DIR | REMOVED_DIR)
                    || result.removed.contains_key(&slot.name)
                {
                    continue;
                }
                ensure!(valid_id(&slot.name), "Invalid Trash item directory");
                if let Some(previous) = result.items.get(&slot.name)
                    && previous.slot.hash == slot.hash
                    && previous.slot.key == slot.key
                {
                    continue;
                }
                let contents = tree.list_directory_required(&entry_cid(&slot)).await?;
                let (note, content_dir) = (
                    named_entry(&contents, ITEM_FILE)?,
                    named_entry(&contents, CONTENT_DIR)?,
                );
                let item: TrashItem = read_json(tree, note).await?;
                item.validate(account, &slot.name)?;
                let name = item.path.last().context("Missing Trash filename")?;
                let mut content = tree
                    .list_directory_required(&entry_cid(content_dir))
                    .await?;
                ensure!(
                    content.len() == 1 && content[0].name == *name,
                    "Invalid Trash payload"
                );
                let content = content.pop().context("Missing Trash payload")?;
                if let Some(previous) = result.items.get(&item.id) {
                    ensure!(
                        previous.item == item && same_entry(&previous.content, &content),
                        "Conflicting payloads for the same Trash item"
                    );
                } else {
                    result.items.insert(
                        item.id.clone(),
                        StoredItem {
                            item,
                            content,
                            slot: dir_entry(&slot),
                        },
                    );
                }
            }
        }
        Ok(result)
    }

    pub async fn insert<S: Store>(
        &mut self,
        tree: &HashTree<S>,
        item: TrashItem,
        content: TreeEntry,
    ) -> Result<()> {
        item.validate(item.account, &item.id)?;
        ensure!(
            item.path.last() == Some(&content.name),
            "Trash filename differs from payload"
        );
        ensure!(
            !self.removed.contains_key(&item.id) && !self.items.contains_key(&item.id),
            "Trash item ID already exists"
        );
        let content_root = tree.put_directory(vec![dir_entry(&content)]).await?;
        let slot = tree
            .put_directory(vec![
                write_json(tree, ITEM_FILE, &item).await?,
                directory(CONTENT_DIR, &content_root),
            ])
            .await?;
        self.items.insert(
            item.id.clone(),
            StoredItem {
                slot: directory(&item.id, &slot),
                item,
                content,
            },
        );
        Ok(())
    }

    pub async fn mark_removed<S: Store>(
        &mut self,
        tree: &HashTree<S>,
        marker: &TrashRemoval,
    ) -> Result<()> {
        marker.validate(&marker.id)?;
        if !self.removed.contains_key(&marker.id) {
            self.removed.insert(
                marker.id.clone(),
                write_json(tree, &marker.id, marker).await?,
            );
        }
        self.items.remove(&marker.id);
        Ok(())
    }

    pub async fn expire<S: Store>(&mut self, tree: &HashTree<S>, now: i64) -> Result<()> {
        let expired = self
            .items
            .values()
            .filter(|stored| stored.item.expired(now))
            .map(|stored| stored.item.id.clone())
            .collect::<Vec<_>>();
        for id in expired {
            self.mark_removed(
                tree,
                &TrashRemoval {
                    v: 1,
                    id,
                    removed_at: now,
                },
            )
            .await?;
        }
        Ok(())
    }

    pub fn was_removed(&self, id: &str) -> bool {
        self.removed.contains_key(id)
    }

    pub async fn write<S: Store>(&self, tree: &HashTree<S>) -> Result<Cid> {
        let mut entries = self
            .items
            .values()
            .map(|item| item.slot.clone())
            .collect::<Vec<_>>();
        if !self.removed.is_empty() {
            let removed = tree
                .put_directory(self.removed.values().cloned().collect())
                .await?;
            entries.push(directory(REMOVED_DIR, &removed));
        }
        let root = tree.put_directory(entries).await?;
        ensure!(root.key.is_some(), "Trash root is unencrypted");
        Ok(root)
    }
}

fn named_entry<'a>(entries: &'a [TreeEntry], name: &str) -> Result<&'a TreeEntry> {
    entries
        .iter()
        .find(|entry| entry.name == name)
        .with_context(|| format!("Missing Trash entry {name}"))
}

async fn read_json<S: Store, T: DeserializeOwned>(
    tree: &HashTree<S>,
    entry: &TreeEntry,
) -> Result<T> {
    ensure!(
        !entry.link_type.is_directory_like(),
        "Trash metadata must be a file"
    );
    let bytes = tree
        .get(&entry_cid(entry), Some(MAX_METADATA_BYTES))
        .await?
        .context("Trash metadata is unavailable")?;
    Ok(serde_json::from_slice(&bytes)?)
}

async fn write_json<S: Store>(
    tree: &HashTree<S>,
    name: &str,
    value: &impl serde::Serialize,
) -> Result<DirEntry> {
    let (cid, size) = tree.put(&serde_json::to_vec(value)?).await?;
    Ok(DirEntry::from_cid(name, &cid).with_size(size))
}

fn directory(name: &str, cid: &Cid) -> DirEntry {
    DirEntry::from_cid(name, cid).with_link_type(LinkType::Dir)
}

fn dir_entry(entry: &TreeEntry) -> DirEntry {
    DirEntry {
        name: entry.name.clone(),
        hash: entry.hash,
        key: entry.key,
        size: entry.size,
        link_type: entry.link_type,
        meta: entry.meta.clone(),
    }
}

pub(super) fn same_entry(left: &TreeEntry, right: &TreeEntry) -> bool {
    left.hash == right.hash
        && left.key == right.key
        && left.name == right.name
        && left.size == right.size
        && left.link_type == right.link_type
        && left.meta == right.meta
}

pub(super) fn entry_cid(entry: &TreeEntry) -> Cid {
    Cid {
        hash: entry.hash,
        key: entry.key,
    }
}
