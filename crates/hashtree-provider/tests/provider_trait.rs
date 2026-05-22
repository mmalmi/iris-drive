//! Smoke test that the `ProviderFs` trait is shape-correct and usable
//! through dynamic dispatch. The implementation is a trivial in-memory
//! map; the real `HashTreeProviderFs` lives elsewhere.

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;
use hashtree_provider::{
    DirItem, Item, ItemKind, PathChange, ProviderError, ProviderFs, SyncAnchor,
};

#[derive(Default)]
struct MemFsInner {
    files: HashMap<String, Vec<u8>>,
    dirs: HashMap<String, Vec<(String, ItemKind)>>,
    revision: u64,
}

struct MemFs {
    inner: Mutex<MemFsInner>,
}

impl MemFs {
    fn new() -> Self {
        let mut inner = MemFsInner::default();
        inner.dirs.insert(String::new(), Vec::new());
        Self {
            inner: Mutex::new(inner),
        }
    }
}

fn child_path(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", parent, name)
    }
}

#[async_trait]
impl ProviderFs for MemFs {
    type ItemId = String;

    async fn root(&self) -> Self::ItemId {
        String::new()
    }

    async fn lookup(
        &self,
        parent: &Self::ItemId,
        name: &str,
    ) -> Result<Item<Self::ItemId>, ProviderError> {
        let inner = self.inner.lock().unwrap();
        let entries = inner.dirs.get(parent).ok_or(ProviderError::NotFound)?;
        let (_, kind) = entries
            .iter()
            .find(|(n, _)| n == name)
            .ok_or(ProviderError::NotFound)?;
        let path = child_path(parent, name);
        let size = match kind {
            ItemKind::File => inner.files.get(&path).map(|b| b.len() as u64).unwrap_or(0),
            ItemKind::Directory => 0,
        };
        Ok(Item {
            id: path,
            name: name.to_string(),
            kind: *kind,
            size,
        })
    }

    async fn item(&self, id: &Self::ItemId) -> Result<Item<Self::ItemId>, ProviderError> {
        let inner = self.inner.lock().unwrap();
        if id.is_empty() {
            return Ok(Item {
                id: id.clone(),
                name: String::new(),
                kind: ItemKind::Directory,
                size: 0,
            });
        }
        if let Some(bytes) = inner.files.get(id) {
            return Ok(Item {
                id: id.clone(),
                name: id.rsplit('/').next().unwrap_or(id).to_string(),
                kind: ItemKind::File,
                size: bytes.len() as u64,
            });
        }
        if inner.dirs.contains_key(id) {
            return Ok(Item {
                id: id.clone(),
                name: id.rsplit('/').next().unwrap_or(id).to_string(),
                kind: ItemKind::Directory,
                size: 0,
            });
        }
        Err(ProviderError::NotFound)
    }

    async fn read(
        &self,
        id: &Self::ItemId,
        offset: u64,
        size: u64,
    ) -> Result<Vec<u8>, ProviderError> {
        let inner = self.inner.lock().unwrap();
        let bytes = inner.files.get(id).ok_or(ProviderError::NotFound)?;
        let start = offset.min(bytes.len() as u64) as usize;
        let end = (offset + size).min(bytes.len() as u64) as usize;
        Ok(bytes[start..end].to_vec())
    }

    async fn read_dir(
        &self,
        id: &Self::ItemId,
    ) -> Result<Vec<DirItem<Self::ItemId>>, ProviderError> {
        let inner = self.inner.lock().unwrap();
        let entries = inner.dirs.get(id).ok_or(ProviderError::NotFound)?;
        Ok(entries
            .iter()
            .map(|(name, kind)| DirItem {
                id: child_path(id, name),
                name: name.clone(),
                kind: *kind,
            })
            .collect())
    }

    async fn create_file(
        &self,
        parent: &Self::ItemId,
        name: &str,
    ) -> Result<Item<Self::ItemId>, ProviderError> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.dirs.contains_key(parent) {
            return Err(ProviderError::NotFound);
        }
        if inner
            .dirs
            .get(parent)
            .unwrap()
            .iter()
            .any(|(n, _)| n == name)
        {
            return Err(ProviderError::AlreadyExists);
        }
        let path = child_path(parent, name);
        inner.files.insert(path.clone(), Vec::new());
        inner
            .dirs
            .get_mut(parent)
            .unwrap()
            .push((name.to_string(), ItemKind::File));
        inner.revision += 1;
        Ok(Item {
            id: path,
            name: name.to_string(),
            kind: ItemKind::File,
            size: 0,
        })
    }

    async fn create_dir(
        &self,
        parent: &Self::ItemId,
        name: &str,
    ) -> Result<Item<Self::ItemId>, ProviderError> {
        let mut inner = self.inner.lock().unwrap();
        if !inner.dirs.contains_key(parent) {
            return Err(ProviderError::NotFound);
        }
        if inner
            .dirs
            .get(parent)
            .unwrap()
            .iter()
            .any(|(n, _)| n == name)
        {
            return Err(ProviderError::AlreadyExists);
        }
        let path = child_path(parent, name);
        inner.dirs.insert(path.clone(), Vec::new());
        inner
            .dirs
            .get_mut(parent)
            .unwrap()
            .push((name.to_string(), ItemKind::Directory));
        inner.revision += 1;
        Ok(Item {
            id: path,
            name: name.to_string(),
            kind: ItemKind::Directory,
            size: 0,
        })
    }

    async fn write(
        &self,
        id: &Self::ItemId,
        offset: u64,
        data: &[u8],
    ) -> Result<u32, ProviderError> {
        let mut inner = self.inner.lock().unwrap();
        let bytes = inner.files.get_mut(id).ok_or(ProviderError::NotFound)?;
        let off = offset as usize;
        if bytes.len() < off + data.len() {
            bytes.resize(off + data.len(), 0);
        }
        bytes[off..off + data.len()].copy_from_slice(data);
        inner.revision += 1;
        Ok(data.len() as u32)
    }

    async fn truncate(&self, id: &Self::ItemId, size: u64) -> Result<(), ProviderError> {
        let mut inner = self.inner.lock().unwrap();
        let bytes = inner.files.get_mut(id).ok_or(ProviderError::NotFound)?;
        bytes.resize(size as usize, 0);
        inner.revision += 1;
        Ok(())
    }

    async fn remove(&self, parent: &Self::ItemId, name: &str) -> Result<(), ProviderError> {
        let mut inner = self.inner.lock().unwrap();
        let dir = inner.dirs.get_mut(parent).ok_or(ProviderError::NotFound)?;
        let pos = dir
            .iter()
            .position(|(n, _)| n == name)
            .ok_or(ProviderError::NotFound)?;
        let (_, kind) = dir.remove(pos);
        let path = child_path(parent, name);
        match kind {
            ItemKind::File => {
                inner.files.remove(&path);
            }
            ItemKind::Directory => {
                let children = inner.dirs.get(&path).map(|v| v.len()).unwrap_or(0);
                if children > 0 {
                    inner
                        .dirs
                        .get_mut(parent)
                        .unwrap()
                        .push((name.to_string(), kind));
                    return Err(ProviderError::NotEmpty);
                }
                inner.dirs.remove(&path);
            }
        }
        inner.revision += 1;
        Ok(())
    }

    async fn rename(
        &self,
        old_parent: &Self::ItemId,
        old_name: &str,
        new_parent: &Self::ItemId,
        new_name: &str,
    ) -> Result<(), ProviderError> {
        // Trivial impl for the smoke test — same-dir only.
        if old_parent != new_parent {
            return Err(ProviderError::Backend(
                "cross-dir rename unimplemented".into(),
            ));
        }
        let mut inner = self.inner.lock().unwrap();
        let dir = inner
            .dirs
            .get_mut(old_parent)
            .ok_or(ProviderError::NotFound)?;
        let pos = dir
            .iter()
            .position(|(n, _)| n == old_name)
            .ok_or(ProviderError::NotFound)?;
        dir[pos].0 = new_name.to_string();
        inner.revision += 1;
        Ok(())
    }

    async fn anchor(&self) -> SyncAnchor {
        let inner = self.inner.lock().unwrap();
        SyncAnchor(format!("memfs:{}", inner.revision))
    }

    async fn changes_since(
        &self,
        _anchor: Option<&SyncAnchor>,
    ) -> Result<Vec<PathChange>, ProviderError> {
        // Smoke test only — no real change tracking in MemFs.
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn round_trip_create_read_remove() {
    let fs = MemFs::new();
    let root = fs.root().await;
    let file = fs.create_file(&root, "hello.txt").await.unwrap();
    fs.write(&file.id, 0, b"hi there").await.unwrap();
    let bytes = fs.read(&file.id, 0, 8).await.unwrap();
    assert_eq!(bytes, b"hi there");
    let listing = fs.read_dir(&root).await.unwrap();
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].name, "hello.txt");
    fs.remove(&root, "hello.txt").await.unwrap();
    assert!(fs.read_dir(&root).await.unwrap().is_empty());
}

#[tokio::test]
async fn anchor_changes_when_state_changes() {
    let fs = MemFs::new();
    let a = fs.anchor().await;
    let root = fs.root().await;
    let _ = fs.create_file(&root, "x").await.unwrap();
    let b = fs.anchor().await;
    assert_ne!(a, b);
}

#[tokio::test]
async fn cannot_remove_non_empty_dir() {
    let fs = MemFs::new();
    let root = fs.root().await;
    let dir = fs.create_dir(&root, "d").await.unwrap();
    fs.create_file(&dir.id, "x").await.unwrap();
    match fs.remove(&root, "d").await {
        Err(ProviderError::NotEmpty) => {}
        other => panic!("expected NotEmpty, got {:?}", other),
    }
}

/// The trait should be object-safe enough to use through a generic.
async fn create_one<F: ProviderFs>(fs: &F, name: &str) -> Item<F::ItemId> {
    let root = fs.root().await;
    fs.create_file(&root, name).await.unwrap()
}

#[tokio::test]
async fn trait_is_usable_through_generic_function() {
    let fs = MemFs::new();
    let item = create_one(&fs, "via-generic.txt").await;
    assert_eq!(item.name, "via-generic.txt");
}
