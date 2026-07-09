use super::*;

#[tokio::test]
async fn fips_block_sync_falls_back_to_mesh_after_direct_miss() {
    let network = Arc::new(TokioMutex::new(std::collections::HashMap::new()));
    let links = Arc::new(TokioMutex::new(std::collections::BTreeMap::from([
        ("target".to_string(), vec!["relay".to_string()]),
        (
            "relay".to_string(),
            vec!["target".to_string(), "source".to_string()],
        ),
        ("source".to_string(), vec!["relay".to_string()]),
    ])));
    let source_endpoint = FakeEndpoint::new_linked("source", network.clone(), links.clone()).await;
    let relay_endpoint = FakeEndpoint::new_linked("relay", network.clone(), links.clone()).await;
    let target_endpoint = FakeEndpoint::new_linked("target", network, links).await;

    let source_store = Arc::new(MemoryStore::new());
    let source_tree = HashTree::new(HashTreeConfig::new(source_store.clone()));
    let (file_cid, _) = source_tree.put(b"hello after direct miss").await.unwrap();
    let root_cid = source_tree
        .put_directory(vec![DirEntry {
            name: "hello.txt".to_string(),
            hash: file_cid.hash,
            key: file_cid.key,
            link_type: LinkType::File,
            size: 23,
            meta: None,
        }])
        .await
        .unwrap();

    let source_transport = Arc::new(HashtreeFipsTransport::new(
        source_endpoint,
        source_store.clone(),
    ));
    let relay_store = Arc::new(MemoryStore::new());
    let relay_transport = Arc::new(HashtreeFipsTransport::new(
        relay_endpoint,
        relay_store.clone(),
    ));
    let target_store = Arc::new(MemoryStore::new());
    let target_transport = Arc::new(HashtreeFipsTransport::new(
        target_endpoint,
        target_store.clone(),
    ));
    target_transport
        .set_peers(vec!["source".to_string(), "relay".to_string()])
        .await;

    let source_task = source_transport.start();
    let relay_task = relay_transport.start();
    let target_task = target_transport.start();
    let _source_mesh = Arc::new(
        source_transport
            .start_mesh_pubsub(
                source_store.clone(),
                "source".to_string(),
                Duration::from_secs(2),
            )
            .await
            .unwrap(),
    );
    let _relay_mesh = Arc::new(
        relay_transport
            .start_mesh_pubsub(relay_store, "relay".to_string(), Duration::from_secs(2))
            .await
            .unwrap(),
    );
    let target_mesh = Arc::new(
        target_transport
            .start_mesh_pubsub(
                target_store.clone(),
                "target".to_string(),
                Duration::from_secs(2),
            )
            .await
            .unwrap(),
    );

    assert!(wait_for_mesh_neighbors(&target_mesh, &["relay"]).await);
    let sync = FipsBlockSync {
        transport: target_transport,
        local_store: target_store.clone(),
        receiver_task: None,
        mesh_pubsub: Some(target_mesh),
        endpoint_npub: "target".to_string(),
        discovery_scope: IRIS_DRIVE_FIPS_DISCOVERY_SCOPE.to_string(),
        transport_settings: FipsTransportSettings::default(),
        last_peer_config: Mutex::new(None),
    };

    let report = sync.download_tree(&root_cid).await.unwrap();

    assert_eq!(report.fetched, 2);
    assert_eq!(report.already_local, 0);
    assert!(target_store.has(&root_cid.hash).await.unwrap());
    assert!(target_store.has(&file_cid.hash).await.unwrap());

    source_task.abort();
    relay_task.abort();
    target_task.abort();
}
