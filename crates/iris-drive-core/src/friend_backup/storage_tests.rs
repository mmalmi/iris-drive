use super::*;
use hashtree_core::{DirEntry, HashTreeConfig, LinkType, MemoryStore, StoreBlobRoute};
use nostr_sdk::nips::nip44;
use nostr_sdk::{Keys, ToBech32};

use std::sync::atomic::{AtomicUsize, Ordering};

struct RawRoute(Arc<MemoryStore>);
#[async_trait::async_trait]
impl BlobRoute for RawRoute {
    async fn route(
        &self,
        req: BlobRequest,
    ) -> std::result::Result<BlobReply, hashtree_core::StoreError> {
        Ok(self
            .0
            .get(&req.hash)
            .await?
            .map_or(BlobReply::NoResult, BlobReply::Data))
    }
}

struct RevokingRoute {
    config_dir: PathBuf,
    source: Arc<dyn BlobRoute>,
}
#[async_trait::async_trait]
impl BlobRoute for RevokingRoute {
    async fn route(
        &self,
        req: BlobRequest,
    ) -> std::result::Result<BlobReply, hashtree_core::StoreError> {
        crate::friend_backup::remove_friend(&self.config_dir, &npub()).unwrap();
        self.source.route(req).await
    }
}

struct ReducingRoute {
    config_dir: PathBuf,
    host: FriendBackupStore,
    source: Arc<dyn BlobRoute>,
    calls: AtomicUsize,
}
#[async_trait::async_trait]
impl BlobRoute for ReducingRoute {
    async fn route(
        &self,
        request: BlobRequest,
    ) -> std::result::Result<BlobReply, hashtree_core::StoreError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 1 {
            let partial_usage = self.host.usage_bytes(&npub()).unwrap();
            assert!(partial_usage > 0);
            crate::friend_backup::upsert_friend(
                &self.config_dir,
                &npub(),
                None,
                partial_usage,
                true,
            )
            .unwrap();
        }
        self.source.route(request).await
    }
}

fn owner() -> Keys {
    Keys::parse(&"01".repeat(32)).unwrap()
}

fn npub() -> String {
    owner().public_key().to_bech32().unwrap()
}

fn capsule(root: &Cid) -> String {
    let keys = owner();
    nip44::encrypt(
        keys.secret_key(),
        &keys.public_key(),
        root.to_string(),
        nip44::Version::V2,
    )
    .unwrap()
}

async fn fixture(path: &Path, contents: &[u8]) -> (HashTree<FsBlobStore>, Cid) {
    let store = Arc::new(FsBlobStore::new(path).unwrap());
    let tree = HashTree::new(HashTreeConfig::new(store));
    let (file, _) = tree.put(contents).await.unwrap();
    let root = tree
        .put_directory(vec![DirEntry {
            name: "private-finances.txt".into(),
            hash: file.hash,
            key: file.key,
            link_type: LinkType::File,
            size: contents.len() as u64,
            meta: None,
        }])
        .await
        .unwrap();
    (tree, root)
}

#[tokio::test]
async fn encrypted_backup_can_restore_without_exposing_filenames_or_keys() {
    let temp = tempfile::tempdir().unwrap();
    let (tree, root) = fixture(&temp.path().join("own"), b"only the owner can read this").await;
    let prepared = prepare_backup(&tree, &root, capsule(&root), temp.path().join("export"))
        .await
        .unwrap();
    let encoded = prepared.manifest.to_bytes().unwrap();
    let text = String::from_utf8(encoded.clone()).unwrap();
    assert!(!text.contains("private-finances"));
    assert!(!text.contains(&hex::encode(root.key.unwrap())));
    assert!(!text.contains("only the owner"));
    let host = FriendBackupStore::open(temp.path().join("host")).unwrap();
    let report = host
        .retain(
            &npub(),
            10_000_000,
            &prepared.manifest,
            prepared.route().as_ref(),
        )
        .await
        .unwrap();
    assert_eq!(report.downloaded, prepared.manifest.entries.len());
    assert_eq!(
        host.current_manifest(&npub()).unwrap(),
        Some(prepared.manifest.clone())
    );
    let route = host.route_for(&npub()).unwrap();
    let restored_store = Arc::new(MemoryStore::new());
    for entry in &prepared.manifest.entries {
        let hash = parse_hash(&entry.hash).unwrap();
        let BlobReply::Data(bytes) = route.route(BlobRequest { hash, htl: 0 }).await.unwrap()
        else {
            panic!("retained block missing")
        };
        assert!(
            !bytes
                .windows(b"private-finances".len())
                .any(|b| b == b"private-finances")
        );
        restored_store.put(hash, bytes).await.unwrap();
    }
    let restored = HashTree::new(HashTreeConfig::new(restored_store));
    assert!(
        restored
            .list_directory_required(&Cid::public(root.hash))
            .await
            .is_err()
    );
    let recovered_cid = nip44::decrypt(
        owner().secret_key(),
        &owner().public_key(),
        &prepared.manifest.encrypted_root_cid,
    )
    .unwrap();
    let recovered = Cid::parse(&recovered_cid).unwrap();
    let entry = restored.list_directory(&recovered).await.unwrap().remove(0);
    assert_eq!(
        restored
            .get(
                &Cid {
                    hash: entry.hash,
                    key: entry.key
                },
                None
            )
            .await
            .unwrap()
            .unwrap(),
        b"only the owner can read this"
    );
    assert_eq!(
        host.all_usage_bytes().unwrap()[&npub()],
        report.retained_bytes
    );
    assert_eq!(
        route
            .route(BlobRequest {
                hash: prepared.manifest_hash,
                htl: 0
            })
            .await
            .unwrap(),
        BlobReply::Data(encoded)
    );
}

#[tokio::test]
async fn quota_is_checked_before_fetching_or_creating_peer_data() {
    let temp = tempfile::tempdir().unwrap();
    let (tree, root) = fixture(&temp.path().join("own"), b"private").await;
    let prepared = prepare_backup(&tree, &root, capsule(&root), temp.path().join("export"))
        .await
        .unwrap();
    let host = FriendBackupStore::open(temp.path().join("host")).unwrap();
    let empty = StoreBlobRoute::new(Arc::new(MemoryStore::new()));
    let error = host
        .retain(&npub(), 1, &prepared.manifest, &empty)
        .await
        .unwrap_err();
    assert!(matches!(error, FriendBackupError::Quota { .. }));
    assert_eq!(host.usage_bytes(&npub()).unwrap(), 0);
    assert!(host.current_manifest(&npub()).unwrap().is_none());
}

#[tokio::test]
async fn failed_update_keeps_old_snapshot_and_charges_partial_bytes() {
    let temp = tempfile::tempdir().unwrap();
    let (tree, root) = fixture(&temp.path().join("own"), b"first version").await;
    let first = prepare_backup(&tree, &root, capsule(&root), temp.path().join("export"))
        .await
        .unwrap();
    let host = FriendBackupStore::open(temp.path().join("host")).unwrap();
    host.retain(&npub(), 10_000_000, &first.manifest, first.route().as_ref())
        .await
        .unwrap();
    let before = host.usage_bytes(&npub()).unwrap();
    let new_bytes = b"new encrypted blob".to_vec();
    let new_hash = sha256(&new_bytes);
    let missing_hash = sha256(b"missing");
    let manifest = BackupManifest {
        version: 1,
        root_hash: hex::encode(new_hash),
        encrypted_root_cid: capsule(&root),
        entries: vec![
            BackupEntry {
                hash: hex::encode(new_hash),
                size: new_bytes.len() as u64,
            },
            BackupEntry {
                hash: hex::encode(missing_hash),
                size: 7,
            },
        ],
    };
    let source = Arc::new(MemoryStore::new());
    source.put(new_hash, new_bytes.clone()).await.unwrap();
    let route = StoreBlobRoute::new(source.clone());
    let error = host
        .retain(&npub(), 10_000_000, &manifest, &route)
        .await
        .unwrap_err();
    assert!(matches!(error, FriendBackupError::Missing(_)));
    assert_eq!(
        host.current_manifest(&npub()).unwrap(),
        Some(first.manifest)
    );
    assert_eq!(
        host.usage_bytes(&npub()).unwrap(),
        before + new_bytes.len() as u64
    );
    source.put(missing_hash, b"missing".to_vec()).await.unwrap();
    let result = host
        .retain(&npub(), 10_000_000, &manifest, &route)
        .await
        .unwrap();
    assert_eq!(result.already_present, 1);
    assert_eq!(result.downloaded, 1);
    let repeated = host
        .retain(&npub(), result.retained_bytes, &manifest, &route)
        .await
        .unwrap();
    assert_eq!(repeated.downloaded, 0);
    assert_eq!(repeated.retained_bytes, result.retained_bytes);
}

#[tokio::test]
async fn corrupt_source_never_commits_and_partial_files_count_towards_quota() {
    let temp = tempfile::tempdir().unwrap();
    let bytes = b"the correct bytes";
    let hash = sha256(bytes);
    let root = Cid::encrypted(hash, [2; 32]);
    let manifest = BackupManifest {
        version: 1,
        root_hash: hex::encode(hash),
        encrypted_root_cid: capsule(&root),
        entries: vec![BackupEntry {
            hash: hex::encode(hash),
            size: bytes.len() as u64,
        }],
    };
    // MemoryStore intentionally permits mismatched data; the retaining host must verify.
    let source = Arc::new(MemoryStore::new());
    source
        .put(hash, b"wrong stored bytes".to_vec())
        .await
        .unwrap();
    let host = FriendBackupStore::open(temp.path().join("host")).unwrap();
    let error = host
        .retain(&npub(), 10_000_000, &manifest, &RawRoute(source))
        .await
        .unwrap_err();
    assert!(matches!(error, FriendBackupError::InvalidBlob(_)));
    assert!(host.current_manifest(&npub()).unwrap().is_none());
    let peer = host.peer_dir(&npub()).unwrap();
    std::fs::create_dir_all(&peer).unwrap();
    std::fs::write(peer.join("crashed-upload.tmp"), vec![0; 2048]).unwrap();
    assert_eq!(host.usage_bytes(&npub()).unwrap(), 2048);
    let empty = StoreBlobRoute::new(Arc::new(MemoryStore::new()));
    assert!(matches!(
        host.retain(&npub(), 2048, &manifest, &empty).await,
        Err(FriendBackupError::Quota { .. })
    ));
}

#[tokio::test]
async fn public_roots_are_refused_and_export_omits_unselected_blobs() {
    let temp = tempfile::tempdir().unwrap();
    let (tree, root) = fixture(&temp.path().join("own"), b"selected").await;
    let (unselected, _) = tree.put(b"not selected for friends").await.unwrap();
    let prepared = prepare_backup(&tree, &root, capsule(&root), temp.path().join("export"))
        .await
        .unwrap();
    assert_eq!(
        prepared
            .route()
            .route(BlobRequest {
                hash: unselected.hash,
                htl: 0
            })
            .await
            .unwrap(),
        BlobReply::NoResult
    );
    let public = Cid::public(root.hash);
    assert!(
        prepare_backup(&tree, &public, capsule(&root), temp.path().join("bad"))
            .await
            .is_err()
    );
    let plaintext = b"an accidentally public file";
    let public_hash = sha256(plaintext);
    tree.get_store()
        .put(public_hash, plaintext.to_vec())
        .await
        .unwrap();
    let mixed_root = tree
        .put_directory(vec![DirEntry {
            name: "public.txt".into(),
            hash: public_hash,
            key: None,
            link_type: LinkType::File,
            size: plaintext.len() as u64,
            meta: None,
        }])
        .await
        .unwrap();
    assert!(
        prepare_backup(
            &tree,
            &mixed_root,
            capsule(&mixed_root),
            temp.path().join("mixed")
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn authorization_is_rechecked_after_fetch_and_retained_bytes_protect_allocations() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    super::super::set_capacity(&config_dir, 10_000_000).unwrap();
    super::super::upsert_friend(&config_dir, &npub(), None, 10_000_000, true).unwrap();
    let (tree, root) = fixture(&temp.path().join("own"), b"authorization matters").await;
    let prepared = prepare_backup(&tree, &root, capsule(&root), temp.path().join("export"))
        .await
        .unwrap();
    let host = FriendBackupStore::open(config_dir.join("friend-backups")).unwrap();
    let revoking = RevokingRoute {
        config_dir: config_dir.clone(),
        source: prepared.route(),
    };
    let error = host
        .retain_authorized(&config_dir, &npub(), &prepared.manifest, &revoking)
        .await
        .unwrap_err();
    assert!(matches!(error, FriendBackupError::Policy(_)));
    assert_eq!(host.usage_bytes(&npub()).unwrap(), 0);
    super::super::upsert_friend(&config_dir, &npub(), None, 10_000_000, true).unwrap();
    host.retain_authorized(
        &config_dir,
        &npub(),
        &prepared.manifest,
        prepared.route().as_ref(),
    )
    .await
    .unwrap();
    assert!(super::super::upsert_friend(&config_dir, &npub(), None, 0, true).is_err());
    assert!(super::super::remove_friend(&config_dir, &npub()).is_err());
    assert!(host.current_manifest(&npub()).unwrap().is_some());
}

#[test]
fn manifests_reject_unbounded_or_ambiguous_inputs_and_plaintext_cids() {
    assert!(BackupManifest::from_bytes(&vec![b' '; MAX_MANIFEST_BYTES + 1]).is_err());
    let root = Cid::encrypted([3; 32], [4; 32]);
    let mut manifest = BackupManifest {
        version: 1,
        root_hash: hex::encode(root.hash),
        encrypted_root_cid: capsule(&root),
        entries: vec![BackupEntry {
            hash: hex::encode(root.hash),
            size: 1,
        }],
    };
    manifest.validate().unwrap();
    manifest.encrypted_root_cid = root.to_string();
    assert!(manifest.validate().is_err());
    manifest.encrypted_root_cid = capsule(&root);
    manifest.entries.push(manifest.entries[0].clone());
    assert!(manifest.validate().is_err());
    manifest.entries.pop();
    manifest.entries[0].size = MAX_BLOB_BYTES as u64 + 1;
    assert!(manifest.validate().is_err());
    manifest.entries[0].size = 1;
    manifest.root_hash = hex::encode([5; 32]);
    assert!(manifest.validate().is_err());
}

#[tokio::test]
async fn reducing_allocation_during_fetch_interrupts_reserved_transfer_and_retry_deduplicates() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    crate::friend_backup::set_capacity(&config_dir, 10_000_000).unwrap();
    crate::friend_backup::upsert_friend(&config_dir, &npub(), None, 10_000_000, true).unwrap();
    let (tree, root) = fixture(&temp.path().join("own"), b"reserve a complete snapshot").await;
    let prepared = prepare_backup(&tree, &root, capsule(&root), temp.path().join("export"))
        .await
        .unwrap();
    assert!(prepared.manifest.entries.len() >= 2);
    let host = FriendBackupStore::open(config_dir.join("friend-backups")).unwrap();
    let source = ReducingRoute {
        config_dir: config_dir.clone(),
        host: host.clone(),
        source: prepared.route(),
        calls: AtomicUsize::new(0),
    };
    let error = host
        .retain_authorized(&config_dir, &npub(), &prepared.manifest, &source)
        .await
        .unwrap_err();
    assert!(matches!(error, FriendBackupError::Quota { .. }));
    assert_eq!(source.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        host.usage_bytes(&npub()).unwrap(),
        prepared.manifest.entries[0].size
    );
    assert!(host.current_manifest(&npub()).unwrap().is_none());
    crate::friend_backup::upsert_friend(&config_dir, &npub(), None, 10_000_000, true).unwrap();
    let retried = host
        .retain_authorized(
            &config_dir,
            &npub(),
            &prepared.manifest,
            prepared.route().as_ref(),
        )
        .await
        .unwrap();
    assert_eq!(retried.already_present, 1);
    assert_eq!(retried.downloaded, prepared.manifest.entries.len() - 1);
}

#[tokio::test]
async fn initial_reservation_rejects_orphaned_retained_data_before_network() {
    let temp = tempfile::tempdir().unwrap();
    let config_dir = temp.path().join("config");
    std::fs::create_dir_all(&config_dir).unwrap();
    crate::friend_backup::set_capacity(&config_dir, 10_000_000).unwrap();
    crate::friend_backup::upsert_friend(&config_dir, &npub(), None, 10_000_000, true).unwrap();
    let (tree, root) = fixture(&temp.path().join("own"), b"selected private snapshot").await;
    let prepared = prepare_backup(&tree, &root, capsule(&root), temp.path().join("export"))
        .await
        .unwrap();
    let base = config_dir.join("friend-backups");
    let orphan = base.join(Keys::generate().public_key().to_hex());
    std::fs::create_dir_all(&orphan).unwrap();
    std::fs::write(
        orphan.join("interrupted-write"),
        b"previously accepted bytes",
    )
    .unwrap();
    let host = FriendBackupStore::open(base).unwrap();
    let empty = StoreBlobRoute::new(Arc::new(MemoryStore::new()));
    let error = host
        .retain_authorized(&config_dir, &npub(), &prepared.manifest, &empty)
        .await
        .unwrap_err();
    assert!(matches!(error, FriendBackupError::Policy(_)));
    assert_eq!(host.usage_bytes(&npub()).unwrap(), 0);
}
