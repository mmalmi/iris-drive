#[allow(clippy::wildcard_imports)]
use super::*;
use hashtree_core::{HashTree, HashTreeConfig, MemoryStore};
use hashtree_provider::{DirItem, HashTreeProviderFs, Item, SyncAnchor};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

async fn fresh_provider() -> HashTreeProviderFs<MemoryStore> {
    let tree = Arc::new(HashTree::new(
        HashTreeConfig::new(Arc::new(MemoryStore::new())).public(),
    ));
    HashTreeProviderFs::fresh(tree).await.unwrap()
}

async fn write_file<P: ProviderFs<ItemId = String>>(fs: &P, path: &str, bytes: &[u8]) {
    write_full(fs, path, bytes).await.unwrap();
}

async fn read_file<P: ProviderFs<ItemId = String>>(fs: &P, path: &str) -> Vec<u8> {
    read_full(fs, path).await.unwrap()
}

async fn paths<P: ProviderFs<ItemId = String>>(fs: &P) -> Vec<String> {
    let mut p: Vec<_> = enumerate_files(fs).await.unwrap().into_keys().collect();
    p.sort();
    p
}

struct NoFullEnumeration<P> {
    inner: P,
    full_enumerations: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
}

impl<P> NoFullEnumeration<P> {
    fn new(inner: P) -> Self {
        Self {
            inner,
            full_enumerations: Arc::new(AtomicUsize::new(0)),
            reads: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn full_enumerations(&self) -> usize {
        self.full_enumerations.load(Ordering::SeqCst)
    }

    fn reads(&self) -> usize {
        self.reads.load(Ordering::SeqCst)
    }
}

#[async_trait::async_trait]
impl<P> ProviderFs for NoFullEnumeration<P>
where
    P: ProviderFs<ItemId = String>,
{
    type ItemId = String;

    async fn root(&self) -> Self::ItemId {
        self.inner.root().await
    }

    async fn lookup(
        &self,
        parent: &Self::ItemId,
        name: &str,
    ) -> Result<Item<Self::ItemId>, ProviderError> {
        self.inner.lookup(parent, name).await
    }

    async fn item(&self, id: &Self::ItemId) -> Result<Item<Self::ItemId>, ProviderError> {
        self.inner.item(id).await
    }

    async fn read(
        &self,
        id: &Self::ItemId,
        offset: u64,
        size: u64,
    ) -> Result<Vec<u8>, ProviderError> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.read(id, offset, size).await
    }

    async fn read_dir(
        &self,
        id: &Self::ItemId,
    ) -> Result<Vec<DirItem<Self::ItemId>>, ProviderError> {
        self.inner.read_dir(id).await
    }

    async fn create_file(
        &self,
        parent: &Self::ItemId,
        name: &str,
    ) -> Result<Item<Self::ItemId>, ProviderError> {
        self.inner.create_file(parent, name).await
    }

    async fn create_dir(
        &self,
        parent: &Self::ItemId,
        name: &str,
    ) -> Result<Item<Self::ItemId>, ProviderError> {
        self.inner.create_dir(parent, name).await
    }

    async fn write(
        &self,
        id: &Self::ItemId,
        offset: u64,
        data: &[u8],
    ) -> Result<u32, ProviderError> {
        self.inner.write(id, offset, data).await
    }

    async fn truncate(&self, id: &Self::ItemId, size: u64) -> Result<(), ProviderError> {
        self.inner.truncate(id, size).await
    }

    async fn remove(&self, parent: &Self::ItemId, name: &str) -> Result<(), ProviderError> {
        self.inner.remove(parent, name).await
    }

    async fn rename(
        &self,
        old_parent: &Self::ItemId,
        old_name: &str,
        new_parent: &Self::ItemId,
        new_name: &str,
    ) -> Result<(), ProviderError> {
        self.inner
            .rename(old_parent, old_name, new_parent, new_name)
            .await
    }

    async fn anchor(&self) -> SyncAnchor {
        self.inner.anchor().await
    }

    async fn changes_since(
        &self,
        anchor: Option<&SyncAnchor>,
    ) -> Result<Vec<PathChange>, ProviderError> {
        if anchor.is_none() {
            self.full_enumerations.fetch_add(1, Ordering::SeqCst);
            return Err(ProviderError::Backend(
                "full enumeration disabled for this test".into(),
            ));
        }
        self.inner.changes_since(anchor).await
    }
}

async fn base_state<P: ProviderFs<ItemId = String>>(
    fs: &P,
) -> BTreeMap<String, crate::conflict::FileSnapshot> {
    enumerate_files(fs)
        .await
        .unwrap()
        .into_iter()
        .map(|(path, entry)| {
            (
                path,
                crate::conflict::FileSnapshot {
                    content_hash: hashtree_core::to_hex(&entry.hash),
                    mtime: 0,
                },
            )
        })
        .collect()
}

#[tokio::test]
async fn empty_to_empty_is_noop() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    let report = sync(&l, &r, "dev").await.unwrap();
    assert_eq!(report, SyncReport::default());
}

#[tokio::test]
async fn local_only_uploads_to_remote() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    write_file(&l, "hello.txt", b"hi there").await;
    let report = sync(&l, &r, "dev").await.unwrap();
    assert_eq!(report.uploaded, vec!["hello.txt".to_string()]);
    assert!(report.downloaded.is_empty());
    assert!(report.conflicts.is_empty());
    assert_eq!(read_file(&r, "hello.txt").await, b"hi there");
}

#[tokio::test]
async fn remote_only_downloads_to_local() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    write_file(&r, "hello.txt", b"from peer").await;
    let report = sync(&l, &r, "dev").await.unwrap();
    assert_eq!(report.downloaded, vec!["hello.txt".to_string()]);
    assert_eq!(read_file(&l, "hello.txt").await, b"from peer");
}

#[tokio::test]
async fn matching_files_are_noop() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    write_file(&l, "hello.txt", b"identical").await;
    write_file(&r, "hello.txt", b"identical").await;
    let report = sync(&l, &r, "dev").await.unwrap();
    assert!(report.uploaded.is_empty());
    assert!(report.downloaded.is_empty());
    assert!(report.conflicts.is_empty());
}

#[tokio::test]
async fn divergent_files_produce_conflict_rename() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    write_file(&l, "report.pdf", b"local-version").await;
    write_file(&r, "report.pdf", b"remote-version").await;
    let report = sync(&l, &r, "dev").await.unwrap();
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(report.conflicts[0].original_path, "report.pdf");
    assert_eq!(
        report.conflicts[0].renamed_to,
        "report (conflict from peer).pdf"
    );
    // Local keeps its original.
    assert_eq!(read_file(&l, "report.pdf").await, b"local-version");
    // Local also has the renamed remote copy.
    assert_eq!(
        read_file(&l, "report (conflict from peer).pdf").await,
        b"remote-version"
    );
    // Remote received local's bytes at the original path.
    assert_eq!(read_file(&r, "report.pdf").await, b"local-version");
}

#[tokio::test]
async fn divergent_existing_conflict_copy_does_not_nest_conflict_name() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    write_file(
        &l,
        "report (conflict from peer).pdf",
        b"local-conflict-copy",
    )
    .await;
    write_file(
        &r,
        "report (conflict from peer).pdf",
        b"remote-conflict-copy",
    )
    .await;

    let report = sync(&l, &r, "dev").await.unwrap();

    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(report.conflicts[0].original_path, "report.pdf");
    assert_eq!(
        report.conflicts[0].renamed_to,
        "report (conflict from peer 2).pdf"
    );
    assert_eq!(
        read_file(&l, "report (conflict from peer 2).pdf").await,
        b"remote-conflict-copy"
    );
    assert!(
        !paths(&l)
            .await
            .contains(&"report (conflict from peer) (conflict from peer).pdf".to_string())
    );
}

#[tokio::test]
async fn nested_path_creates_parent_dirs() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    write_file(&l, "a/b/c.txt", b"deep").await;
    sync(&l, &r, "dev").await.unwrap();
    assert_eq!(read_file(&r, "a/b/c.txt").await, b"deep");
}

#[tokio::test]
async fn two_passes_converge() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    write_file(&l, "x.txt", b"local").await;
    write_file(&r, "y.txt", b"remote").await;
    sync(&l, &r, "dev").await.unwrap();
    assert_eq!(
        paths(&l).await,
        vec!["x.txt".to_string(), "y.txt".to_string()]
    );
    assert_eq!(
        paths(&r).await,
        vec!["x.txt".to_string(), "y.txt".to_string()]
    );
    // Second sync is a no-op.
    let report = sync(&l, &r, "dev").await.unwrap();
    assert_eq!(report, SyncReport::default());
}

#[tokio::test]
async fn second_sync_after_modification_propagates() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    write_file(&l, "x.txt", b"v1").await;
    sync(&l, &r, "dev").await.unwrap();
    assert_eq!(read_file(&r, "x.txt").await, b"v1");

    // local edits, then sync again
    write_file(&l, "x.txt", b"v2-larger").await;
    let report = sync(&l, &r, "dev").await.unwrap();
    assert_eq!(report.uploaded, vec!["x.txt".to_string()]);
    assert_eq!(read_file(&r, "x.txt").await, b"v2-larger");
}

#[tokio::test]
async fn base_state_local_delete_removes_unchanged_remote() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    write_file(&l, "shared.txt", b"v1").await;
    write_file(&r, "shared.txt", b"v1").await;
    let base = base_state(&l).await;

    let root = l.root().await;
    l.remove(&root, "shared.txt").await.unwrap();

    let report = sync_with_base(&l, &r, &base, "peer").await.unwrap();
    assert_eq!(report.deleted_remote, vec!["shared.txt".to_string()]);
    assert!(report.downloaded.is_empty());
    assert_eq!(paths(&l).await, Vec::<String>::new());
    assert_eq!(paths(&r).await, Vec::<String>::new());
}

#[tokio::test]
async fn cache_backed_sync_persists_base_for_next_delete() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    let mut cache = crate::sync_cache::SyncCache::empty();

    write_file(&l, "shared.txt", b"v1").await;
    let first = sync_with_cache(&l, &r, &mut cache, "main", "peer")
        .await
        .unwrap();
    assert_eq!(first.uploaded, vec!["shared.txt".to_string()]);
    assert_eq!(cache.base_snapshots_for_drive("main").len(), 1);

    let root = l.root().await;
    l.remove(&root, "shared.txt").await.unwrap();
    let second = sync_with_cache(&l, &r, &mut cache, "main", "peer")
        .await
        .unwrap();

    assert_eq!(second.deleted_remote, vec!["shared.txt".to_string()]);
    assert_eq!(paths(&l).await, Vec::<String>::new());
    assert_eq!(paths(&r).await, Vec::<String>::new());
    assert!(cache.base_snapshots_for_drive("main").is_empty());
}

#[tokio::test]
async fn cache_backed_sync_keeps_anchor_after_drive_becomes_empty() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    let mut cache = crate::sync_cache::SyncCache::empty();

    write_file(&l, "shared.txt", b"v1").await;
    sync_with_cache(&l, &r, &mut cache, "main", "peer")
        .await
        .unwrap();

    let root = l.root().await;
    l.remove(&root, "shared.txt").await.unwrap();
    sync_with_cache(&l, &r, &mut cache, "main", "peer")
        .await
        .unwrap();
    assert!(cache.base_snapshots_for_drive("main").is_empty());

    write_file(&r, "remote.txt", b"new after empty").await;
    let l = NoFullEnumeration::new(l);
    let r = NoFullEnumeration::new(r);

    let report = sync_with_cache(&l, &r, &mut cache, "main", "peer")
        .await
        .unwrap();

    assert_eq!(report.downloaded, vec!["remote.txt".to_string()]);
    assert_eq!(read_file(&l, "remote.txt").await, b"new after empty");
    assert_eq!(l.full_enumerations(), 0);
    assert_eq!(r.full_enumerations(), 0);
}

#[tokio::test]
async fn cache_backed_sync_keeps_old_base_when_conflicted() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    let mut cache = crate::sync_cache::SyncCache::empty();

    write_file(&l, "report.txt", b"local").await;
    write_file(&r, "report.txt", b"remote").await;
    let report = sync_with_cache(&l, &r, &mut cache, "main", "peer")
        .await
        .unwrap();

    assert_eq!(report.conflicts.len(), 1);
    assert!(cache.base_snapshots_for_drive("main").is_empty());
}

#[tokio::test]
async fn cache_backed_sync_uses_anchor_diff_after_base_exists() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    let mut cache = crate::sync_cache::SyncCache::empty();

    write_file(&l, "shared.txt", b"v1").await;
    sync_with_cache(&l, &r, &mut cache, "main", "peer")
        .await
        .unwrap();

    write_file(&r, "remote.txt", b"new from remote").await;
    let l = NoFullEnumeration::new(l);
    let r = NoFullEnumeration::new(r);

    let report = sync_with_cache(&l, &r, &mut cache, "main", "peer")
        .await
        .unwrap();

    assert_eq!(report.downloaded, vec!["remote.txt".to_string()]);
    assert_eq!(read_file(&l, "remote.txt").await, b"new from remote");
    assert_eq!(l.full_enumerations(), 0);
    assert_eq!(r.full_enumerations(), 0);
    assert!(
        cache
            .base_snapshots_for_drive("main")
            .contains_key("remote.txt")
    );
}

#[tokio::test]
async fn anchored_sync_does_not_reuse_pending_delete_conflict_name() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    let mut cache = crate::sync_cache::SyncCache::empty();

    write_file(&l, "note", b"base").await;
    write_file(&l, "note (conflict from peer)", b"old conflict").await;
    sync_with_cache(&l, &r, &mut cache, "main", "peer")
        .await
        .unwrap();

    write_file(&l, "note", b"local").await;
    write_file(&r, "note", b"remote").await;
    let remote_root = r.root().await;
    r.remove(&remote_root, "note (conflict from peer)")
        .await
        .unwrap();

    let report = sync_with_cache(&l, &r, &mut cache, "main", "peer")
        .await
        .unwrap();

    assert_eq!(
        report.conflicts[0].renamed_to,
        "note (conflict from peer 2)"
    );
    assert_eq!(
        read_file(&l, "note (conflict from peer 2)").await,
        b"remote"
    );
    assert!(
        read_full(&l, "note (conflict from peer)").await.is_err(),
        "old conflict copy should be deleted, not overwritten and kept"
    );
}

#[tokio::test]
async fn anchored_sync_same_content_change_does_not_read_file_bytes() {
    let l = fresh_provider().await;
    let r = fresh_provider().await;
    let mut cache = crate::sync_cache::SyncCache::empty();

    write_file(&l, "same.txt", b"v1").await;
    sync_with_cache(&l, &r, &mut cache, "main", "peer")
        .await
        .unwrap();

    write_file(&l, "same.txt", b"v2").await;
    write_file(&r, "same.txt", b"v2").await;
    let l = NoFullEnumeration::new(l);
    let r = NoFullEnumeration::new(r);

    let report = sync_with_cache(&l, &r, &mut cache, "main", "peer")
        .await
        .unwrap();

    assert_eq!(report, SyncReport::default());
    assert_eq!(l.full_enumerations(), 0);
    assert_eq!(r.full_enumerations(), 0);
    assert_eq!(l.reads(), 0);
    assert_eq!(r.reads(), 0);
}

#[tokio::test]
async fn three_devices_converge_after_two_pairwise_syncs() {
    let a = fresh_provider().await;
    let b = fresh_provider().await;
    let c = fresh_provider().await;
    write_file(&a, "shared.txt", b"alpha").await;
    sync(&a, &b, "a").await.unwrap();
    sync(&b, &c, "b").await.unwrap();
    assert_eq!(read_file(&c, "shared.txt").await, b"alpha");
}
