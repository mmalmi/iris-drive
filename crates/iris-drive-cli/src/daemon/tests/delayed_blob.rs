use super::*;
use async_trait::async_trait;
use hashtree_core::{Hash, MemoryStore, StoreError};
use iris_drive_core::{AppKey, FipsBlockSync};

const CHILD_ENV: &str = "IRIS_DRIVE_DELAYED_BLOB_TEST_CHILD";

/// Run real FIPS endpoints in an isolated process so discovery settings cannot
/// affect concurrently running tests or contact the user's configured peers.
#[test]
fn blossom_miss_does_not_cancel_a_delayed_valid_peer_blob() {
    if std::env::var_os(CHILD_ENV).is_some() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(30), delayed_peer_download())
                .await
                .expect("local delayed-blob fixture did not finish");
        });
        return;
    }

    let directory = tempfile::tempdir().unwrap();
    let rendezvous = std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "daemon::tests::delayed_blob::blossom_miss_does_not_cancel_a_delayed_valid_peer_blob",
            "--nocapture",
        ])
        .env(CHILD_ENV, "1")
        .env("HTREE_DATA_DIR", directory.path().join("shared"))
        .env("HTREE_CONFIG_DIR", directory.path().join("htree-config"))
        .env("IRIS_DRIVE_FIPS_ENABLE_UDP", "true")
        .env("IRIS_DRIVE_FIPS_UDP_BIND_ADDR", "127.0.0.1:0")
        .env("IRIS_DRIVE_FIPS_UDP_PUBLIC", "false")
        .env("IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR", "")
        .env("IRIS_DRIVE_FIPS_ENABLE_LOCAL_RENDEZVOUS", "true")
        .env(
            "IRIS_DRIVE_FIPS_LOCAL_RENDEZVOUS_ADDR",
            rendezvous.to_string(),
        )
        .env("IRIS_DRIVE_FIPS_ENABLE_WEBRTC", "false")
        .env("IRIS_DRIVE_FIPS_ENABLE_LAN_DISCOVERY", "false")
        .env("IRIS_DRIVE_FIPS_ENABLE_NOSTR_DISCOVERY", "false")
        .env("IRIS_DRIVE_FIPS_ENABLE_MESH_PUBSUB", "false")
        .env("IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP", "false")
        .env("IRIS_DRIVE_FIPS_STATIC_PEERS", "")
        .env("IRIS_FIPS_WEBSOCKET_BIND_ADDR", "")
        .env("IRIS_FIPS_WEBSOCKET_SEED_URLS", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "delayed-blob fixture failed: {}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

async fn delayed_peer_download() {
    let directory = tempfile::tempdir().unwrap();
    let profile =
        iris_drive_core::Profile::create(directory.path(), Some("reader".into())).unwrap();
    let source_key = AppKey::generate(directory.path().join("source-key"));
    let mut config = AppConfig {
        profile: Some(profile.state),
        relays: Vec::new(),
        blossom_servers: vec!["http://127.0.0.1:1".to_string()],
        ..AppConfig::default()
    };
    config
        .profile
        .as_mut()
        .unwrap()
        .app_keys
        .as_mut()
        .unwrap()
        .app_actors
        .push(iris_drive_core::app_keys::AppActorEntry::member(
            source_key.pubkey_hex(),
            1,
            None,
        ));
    let source_store = Arc::new(DelayedStore(MemoryStore::new()));
    let source_tree = HashTree::new(HashTreeConfig::new(source_store.clone()));
    let bytes = b"a valid peer response that arrives after the old three-second cutoff";
    let (root, _) = source_tree.put(bytes).await.unwrap();
    let mut source_config = config.clone();
    source_config.profile.as_mut().unwrap().app_key_pubkey = source_key.pubkey_hex();
    let source = FipsBlockSync::start(&source_key, source_store, &source_config)
        .await
        .unwrap();
    let local = Arc::new(FsBlobStore::new(directory.path().join("reader-blocks")).unwrap());
    let reader = FipsBlockSync::start(&profile.app_key, local.clone(), &config)
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            if reader
                .fips_peer_statuses()
                .await
                .iter()
                .any(|peer| peer.npub == source_key.pubkey_bech32() && peer.connected)
            {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("authorized local FIPS peer did not connect");

    let fips = Box::pin(async {
        download_tree_over_fips_with_retry(&reader, &root, fips_download_policy(&config))
            .await
            .map_err(|error| error.to_string())
    });
    let blossom = Box::pin(async { Err("Blossom has no copy".to_string()) });
    let outcome = first_successful_block_download(Some(fips), Some(blossom)).await;
    reader.shutdown_endpoint().await.unwrap();
    source.shutdown_endpoint().await.unwrap();
    let outcome = outcome.expect("a backup miss must not cancel the valid FIPS response");
    assert_eq!(outcome.transport, BlockDownloadTransport::Fips);
    assert_eq!(outcome.prior_errors.len(), 1);
    assert_eq!(
        outcome.prior_errors[0].transport,
        BlockDownloadTransport::Blossom
    );
    let tree = HashTree::new(HashTreeConfig::new(local));
    assert_eq!(tree.get(&root, None).await.unwrap().unwrap(), bytes);
}

struct DelayedStore(MemoryStore);

#[async_trait]
impl Store for DelayedStore {
    async fn put(&self, hash: Hash, data: Vec<u8>) -> Result<bool, StoreError> {
        self.0.put(hash, data).await
    }

    async fn get(&self, hash: &Hash) -> Result<Option<Vec<u8>>, StoreError> {
        let data = self.0.get(hash).await?;
        if data.is_some() {
            // Model bytes arriving within the transport's ten-second search
            // window, but after the obsolete before-Blossom deadline.
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
        }
        Ok(data)
    }

    async fn has(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.0.has(hash).await
    }

    async fn delete(&self, hash: &Hash) -> Result<bool, StoreError> {
        self.0.delete(hash).await
    }
}
