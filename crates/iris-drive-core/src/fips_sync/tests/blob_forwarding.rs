use super::*;
use async_trait::async_trait;
use hashtree_core::{BlobReply, BlobRequest, BlobRoute, Hash, Store, StoreError};
use hashtree_fips_transport::TcpBlobTransport;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use super::mesh_fallback::{bind_test_endpoint, reserve_udp_address, wait_for_peer_connection};

/// Exercise Drive's actual inbound router and peer route over local FIPS
/// endpoints. Both serving peers can forward to each other, just as two
/// authorized Drive devices do when neither has the requested child block.
#[tokio::test]
async fn drive_blob_forwarding_preserves_local_hits_and_bounds_missing_cycles() {
    let reader_key = AppKey::generate("blob-hop-reader");
    let first_key = AppKey::generate("blob-hop-first");
    let second_key = AppKey::generate("blob-hop-second");
    let rendezvous = reserve_udp_address();
    let reader_addr = reserve_udp_address();
    let first_addr = reserve_udp_address();
    let second_addr = reserve_udp_address();
    let reader_bound = bind_test_endpoint(
        &reader_key,
        "blob-hop-reader",
        rendezvous,
        reader_addr,
        true,
    )
    .await
    .unwrap();
    let first_bound =
        bind_test_endpoint(&first_key, "blob-hop-first", rendezvous, first_addr, true)
            .await
            .unwrap();
    let second_bound = bind_test_endpoint(
        &second_key,
        "blob-hop-second",
        rendezvous,
        second_addr,
        true,
    )
    .await
    .unwrap();

    let first_store = Arc::new(RecordingStore::default());
    let second_store = Arc::new(RecordingStore::default());
    let local_data = b"local bytes remain readable with no forwarding budget".to_vec();
    let remote_data = b"one forwarding hop reaches verified remote bytes".to_vec();
    let exhausted_data = b"remote bytes must not bypass an exhausted hop budget".to_vec();
    let local_hash = hashtree_core::sha256(&local_data);
    let remote_hash = hashtree_core::sha256(&remote_data);
    let exhausted_hash = hashtree_core::sha256(&exhausted_data);
    let missing_hash = hashtree_core::sha256(b"no device has this child");
    first_store
        .put(local_hash, local_data.clone())
        .await
        .unwrap();
    second_store
        .put(remote_hash, remote_data.clone())
        .await
        .unwrap();
    second_store
        .put(exhausted_hash, exhausted_data)
        .await
        .unwrap();

    let first = DriveBlobRuntime::bind(
        first_bound.native_endpoint.clone(),
        first_store.clone(),
        &[
            FipsPeerConfig::new(reader_key.pubkey_bech32()),
            FipsPeerConfig::new(second_key.pubkey_bech32()),
        ],
        None,
    )
    .await
    .unwrap();
    let second = DriveBlobRuntime::bind(
        second_bound.native_endpoint.clone(),
        second_store.clone(),
        &[
            FipsPeerConfig::new(reader_key.pubkey_bech32()),
            FipsPeerConfig::new(first_key.pubkey_bech32()),
        ],
        None,
    )
    .await
    .unwrap();
    let reader = Arc::new(
        TcpBlobTransport::bind(
            reader_bound.native_endpoint.clone(),
            Arc::new(MemoryStore::new()),
        )
        .await
        .unwrap(),
    );
    // DriveBlobRuntime owns blob authorization, while the existing endpoint
    // policy owns connection establishment. Supply real loopback hints and
    // application peers as the production FipsBlockSync startup does.
    for (bound, key, remotes) in [
        (
            &reader_bound,
            &reader_key,
            [(&first_key, first_addr), (&second_key, second_addr)],
        ),
        (
            &first_bound,
            &first_key,
            [(&reader_key, reader_addr), (&second_key, second_addr)],
        ),
        (
            &second_bound,
            &second_key,
            [(&reader_key, reader_addr), (&first_key, first_addr)],
        ),
    ] {
        let peers = remotes
            .into_iter()
            .map(|(key, address)| FipsPeerConfig {
                npub: key.pubkey_bech32(),
                udp_addresses: vec![address.to_string()],
            })
            .collect::<Vec<_>>();
        set_drive_fips_peer_configs(
            bound.native_endpoint.as_ref(),
            &key.pubkey_bech32(),
            &peers,
            &[],
            peers.clone(),
        )
        .await
        .unwrap();
    }
    wait_for_peer_connection(&reader_bound, &first_key.pubkey_bech32()).await;
    wait_for_peer_connection(&first_bound, &second_key.pubkey_bech32()).await;
    wait_for_peer_connection(&second_bound, &first_key.pubkey_bech32()).await;
    let route =
        reader.route_to(fips_core::PeerIdentity::from_npub(&first_key.pubkey_bech32()).unwrap());

    // Collect outcomes before asserting so a deliberately red regression
    // still shuts down every owned endpoint and transport.
    let local = bounded_request(&route, local_hash, 0).await;
    let exhausted = bounded_request(&route, exhausted_hash, 0).await;
    let exhausted_remote_reads = second_store.reads(&exhausted_hash);
    let remote = bounded_request(&route, remote_hash, 1).await;
    let remote_cached = first_store.inner.get(&remote_hash).await.unwrap();
    let missing = bounded_request(&route, missing_hash, 2).await;
    let missing_first_reads = first_store.reads(&missing_hash);
    let missing_second_reads = second_store.reads(&missing_hash);
    // A finite miss must not poison a subsequent valid request.
    let after_miss = bounded_request(&route, local_hash, 0).await;

    drop(route);
    Arc::try_unwrap(reader)
        .ok()
        .unwrap()
        .shutdown()
        .await
        .unwrap();
    drop(first);
    drop(second);
    reader_bound.native_endpoint.shutdown().await.unwrap();
    first_bound.native_endpoint.shutdown().await.unwrap();
    second_bound.native_endpoint.shutdown().await.unwrap();

    assert_eq!(local.unwrap(), BlobReply::Data(local_data.clone()));
    assert_eq!(exhausted.unwrap(), BlobReply::NoResult);
    assert_eq!(exhausted_remote_reads, 0, "HTL zero reached another peer");
    assert_eq!(remote.unwrap(), BlobReply::Data(remote_data.clone()));
    assert_eq!(remote_cached, Some(remote_data));
    assert_eq!(missing.unwrap(), BlobReply::NoResult);
    assert!(
        missing_first_reads <= 2,
        "the request cycled through its origin"
    );
    assert_eq!(missing_second_reads, 1, "the request fanned out repeatedly");
    assert_eq!(after_miss.unwrap(), BlobReply::Data(local_data));
}

async fn bounded_request(route: &dyn BlobRoute, hash: Hash, htl: u8) -> Result<BlobReply, String> {
    tokio::time::timeout(
        Duration::from_secs(15),
        route.route(BlobRequest { hash, htl }),
    )
    .await
    .map_err(|_| "local hop-budget fixture exceeded its outer bound".to_string())?
    .map_err(|error| error.to_string())
}

#[derive(Default)]
struct RecordingStore {
    inner: MemoryStore,
    reads: Mutex<HashMap<Hash, usize>>,
}

impl RecordingStore {
    fn reads(&self, hash: &Hash) -> usize {
        self.reads.lock().unwrap().get(hash).copied().unwrap_or(0)
    }
}

#[async_trait]
impl Store for RecordingStore {
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
        self.inner.put(hash, data).await
    }

    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        *self.reads.lock().unwrap().entry(*hash).or_default() += 1;
        self.inner.get(hash).await
    }

    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.inner.has(hash).await
    }

    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.inner.delete(hash).await
    }
}
