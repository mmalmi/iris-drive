use super::*;

struct ErrorOnGetStore;

#[async_trait]
impl Store for ErrorOnGetStore {
    async fn put(&self, _hash: Hash, _data: Vec<u8>) -> Result<bool, StoreError> {
        Ok(false)
    }

    async fn get(&self, _hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        Err(StoreError::Other("deliberate local read error".to_string()))
    }

    async fn has(&self, _hash: &Hash) -> Result<bool, StoreError> {
        Ok(false)
    }

    async fn delete(&self, _hash: &Hash) -> Result<bool, StoreError> {
        Ok(false)
    }
}

#[derive(Default)]
struct CountingResolver {
    gets: std::sync::atomic::AtomicUsize,
}

#[async_trait]
impl Store for CountingResolver {
    async fn put(&self, _hash: Hash, _data: Vec<u8>) -> Result<bool, StoreError> {
        Ok(false)
    }

    async fn get(&self, _hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        self.gets.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(None)
    }

    async fn has(&self, _hash: &Hash) -> Result<bool, StoreError> {
        Ok(false)
    }

    async fn delete(&self, _hash: &Hash) -> Result<bool, StoreError> {
        Ok(false)
    }
}

#[tokio::test]
async fn local_store_errors_do_not_fall_through_to_another_route() {
    let resolver = Arc::new(CountingResolver::default());
    let store = MeasuredResolverStore::new(Arc::new(ErrorOnGetStore), resolver.clone());

    let error = store.get(&[0x42; 32]).await.unwrap_err();

    assert!(error.to_string().contains("deliberate local read error"));
    assert_eq!(resolver.gets.load(std::sync::atomic::Ordering::Relaxed), 0);
}

#[tokio::test]
async fn downloads_tree_blocks_from_direct_fips_peer() {
    let network = Arc::new(TokioMutex::new(std::collections::HashMap::new()));
    let source_endpoint = FakeEndpoint::new("source", network.clone()).await;
    let target_endpoint = FakeEndpoint::new("target", network).await;

    let source_store = Arc::new(MemoryStore::new());
    let source_tree = HashTree::new(HashTreeConfig::new(source_store.clone()));
    let (file_cid, _) = source_tree.put(b"hello from fips").await.unwrap();
    let root_cid = source_tree
        .put_directory(vec![DirEntry {
            name: "hello.txt".to_string(),
            hash: file_cid.hash,
            key: file_cid.key,
            link_type: LinkType::File,
            size: 15,
            meta: None,
        }])
        .await
        .unwrap();

    let source_transport = Arc::new(HashtreeFipsTransport::new(source_endpoint, source_store));
    let source_task = source_transport.start();

    let target_store = Arc::new(MemoryStore::new());
    let target_transport = Arc::new(HashtreeFipsTransport::new(
        target_endpoint,
        target_store.clone(),
    ));
    target_transport.set_peers(vec!["source".to_string()]).await;
    let target_task = target_transport.start();

    let report = download_tree_with_resolver(target_store.clone(), &root_cid, target_transport)
        .await
        .unwrap();

    assert_eq!(report.fetched, 2);
    assert_eq!(report.already_local, 0);
    assert!(target_store.has(&root_cid.hash).await.unwrap());
    assert!(target_store.has(&file_cid.hash).await.unwrap());

    source_task.abort();
    target_task.abort();
}

#[tokio::test]
async fn download_skips_unavailable_prev_history_target() {
    let network = Arc::new(TokioMutex::new(std::collections::HashMap::new()));
    let source_endpoint = FakeEndpoint::new("source", network.clone()).await;
    let target_endpoint = FakeEndpoint::new("target", network).await;

    let source_store = Arc::new(MemoryStore::new());
    let source_tree = HashTree::new(HashTreeConfig::new(source_store.clone()));
    let (file_cid, _) = source_tree.put(b"current visible bytes").await.unwrap();
    let visible_root = source_tree
        .put_directory(vec![DirEntry {
            name: "current.txt".to_string(),
            hash: file_cid.hash,
            key: file_cid.key,
            link_type: LinkType::File,
            size: 21,
            meta: None,
        }])
        .await
        .unwrap();
    let missing_prev = Cid {
        hash: [7; 32],
        key: None,
    };
    let root_with_history =
        crate::indexer::layer_prev_link(&source_tree, visible_root, &missing_prev)
            .await
            .unwrap();

    let source_transport = Arc::new(HashtreeFipsTransport::new(source_endpoint, source_store));
    let source_task = source_transport.start();

    let target_store = Arc::new(MemoryStore::new());
    let target_transport = Arc::new(HashtreeFipsTransport::new(
        target_endpoint,
        target_store.clone(),
    ));
    target_transport.set_peers(vec!["source".to_string()]).await;
    let target_task = target_transport.start();

    let report =
        download_tree_with_resolver(target_store.clone(), &root_with_history, target_transport)
            .await
            .unwrap();

    assert!(report.fetched >= 3);
    assert!(target_store.has(&root_with_history.hash).await.unwrap());
    assert!(target_store.has(&file_cid.hash).await.unwrap());
    assert!(!target_store.has(&missing_prev.hash).await.unwrap());

    source_task.abort();
    target_task.abort();
}

#[tokio::test]
async fn signed_update_is_replayed_to_a_connected_late_fips_subscriber() {
    let network = Arc::new(TokioMutex::new(std::collections::HashMap::new()));
    let source_endpoint = FakeEndpoint::new("source", network.clone()).await;
    let target_endpoint = FakeEndpoint::new("target", network).await;
    let source_store = Arc::new(MemoryStore::new());
    let target_store = Arc::new(MemoryStore::new());
    let source_transport = Arc::new(HashtreeFipsTransport::new(
        source_endpoint,
        source_store.clone(),
    ));
    let target_transport = Arc::new(HashtreeFipsTransport::new(
        target_endpoint,
        target_store.clone(),
    ));
    source_transport.set_peers(vec!["target".to_string()]).await;
    target_transport.set_peers(vec!["source".to_string()]).await;
    let source_task = source_transport.start();
    let target_task = target_transport.start();
    let source_mesh = Arc::new(
        source_transport
            .start_mesh_pubsub(
                source_store.clone(),
                "source".to_string(),
                Duration::from_millis(200),
            )
            .await
            .unwrap(),
    );
    let target_mesh = Arc::new(
        target_transport
            .start_mesh_pubsub(
                target_store.clone(),
                "target".to_string(),
                Duration::from_millis(200),
            )
            .await
            .unwrap(),
    );
    assert!(wait_for_mesh_neighbors(&source_mesh, &["target"]).await);
    assert!(wait_for_mesh_neighbors(&target_mesh, &["source"]).await);

    let source_sync = FipsBlockSync {
        transport: source_transport.clone(),
        blob_store: source_transport,
        local_store: source_store,
        receiver_task: Some(source_task),
        mesh_pubsub: Some(source_mesh),
        endpoint_npub: "source".to_string(),
        discovery_scope: IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string(),
        transport_settings: FipsTransportSettings::default(),
        last_peer_config: Mutex::new(None),
    };
    let target_sync = FipsBlockSync {
        transport: target_transport.clone(),
        blob_store: target_transport,
        local_store: target_store,
        receiver_task: Some(target_task),
        mesh_pubsub: Some(target_mesh),
        endpoint_npub: "target".to_string(),
        discovery_scope: IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string(),
        transport_settings: FipsTransportSettings::default(),
        last_peer_config: Mutex::new(None),
    };

    let release_keys = Keys::generate();
    let tree_name = "releases/iris-drive";
    let reference = hashtree_updater::UpdateRef {
        npub: release_keys.public_key().to_bech32().unwrap(),
        tree_name: tree_name.to_string(),
        path: Some("latest".to_string()),
    };
    let event = EventBuilder::new(Kind::Custom(30_064), "")
        .tags([
            Tag::identifier(tree_name),
            Tag::custom(
                TagKind::SingleLetter(SingleLetterTag::lowercase(Alphabet::L)),
                ["hashtree"],
            ),
            Tag::custom(TagKind::Custom("hash".into()), ["42".repeat(32)]),
        ])
        .sign_with_keys(&release_keys)
        .unwrap();
    let source_dir = tempfile::tempdir().unwrap();
    let target_dir = tempfile::tempdir().unwrap();
    let mut source_exchange =
        crate::UpdateAnnouncementExchange::load_for_reference(source_dir.path(), &reference)
            .unwrap();
    assert!(
        source_exchange
            .ingest_event(source_dir.path(), event.clone())
            .unwrap()
    );

    // The announcement predates the target's subscription. Once connected,
    // subscribing and the source's peer refresh replay the cached event.
    let mut target_exchange =
        crate::UpdateAnnouncementExchange::load_for_reference(target_dir.path(), &reference)
            .unwrap();
    target_exchange
        .sync_with_peers(target_dir.path(), &target_sync)
        .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(
        source_exchange
            .sync_with_peers(source_dir.path(), &source_sync)
            .await
    );

    let received = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            source_exchange.replay_cached(&source_sync).await;
            tokio::time::sleep(Duration::from_millis(25)).await;
            if let Some(received) = target_sync.drain_mesh_pubsub_events().await.pop() {
                break received;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(received.stream_id, crate::UPDATE_ANNOUNCEMENT_MESH_STREAM);
    assert!(
        target_exchange
            .handle_mesh_event(target_dir.path(), &received)
            .unwrap()
    );
    assert_eq!(
        target_exchange.latest_event().map(|event| event.id),
        Some(event.id)
    );
    let reloaded =
        crate::UpdateAnnouncementExchange::load_for_reference(target_dir.path(), &reference)
            .unwrap();
    assert_eq!(
        reloaded.latest_event().map(|event| event.id),
        Some(event.id)
    );

    source_sync.shutdown().await.unwrap();
    target_sync.shutdown().await.unwrap();
}
