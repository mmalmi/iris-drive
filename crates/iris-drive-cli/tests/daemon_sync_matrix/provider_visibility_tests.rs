#[allow(clippy::wildcard_imports)]
use super::*;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_provider_add_appears_on_all_connected_devices() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start_three(Duration::ZERO).await;
    cluster.wait_until_authorized().await;
    cluster.wait_until_direct_peers_connected().await;

    let path = "connected/new-from-macos.txt";
    let bytes = b"added through connected macos provider";
    cluster.provider_write(Client::MacOS, path, bytes).await;

    cluster
        .wait_for_provider_entry(path, "file", "connected provider add visible everywhere")
        .await;
    let mut expected = DirSnapshot::new();
    expected.insert(
        path.to_owned(),
        FileSnapshot {
            len: bytes.len() as u64,
            sha256: to_hex(&sha256(bytes)),
            bytes: bytes.to_vec(),
        },
    );
    cluster
        .wait_for_visible_snapshot(&expected, "connected provider add materialized everywhere")
        .await;
    for client in Client::THREE_VM {
        cluster.assert_provider_entry(client, path, "file");
        cluster.assert_file(client, path, bytes);
        cluster.assert_status_counts(client, 1, 3);
    }
}
