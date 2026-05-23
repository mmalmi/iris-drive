//! Live daemon sync tests and transfer benches.
//!
//! The shape follows Seafile's sync-auto-test and Syncthing's integration
//! benches: run real clients, mutate one worktree at a time, wait for
//! convergence, then compare on-disk contents instead of trusting status text.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::OpenOptions;
use std::io::Write as _;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use axum::{
    Router,
    body::{Body, Bytes},
    extract::{
        Path as AxumPath, State,
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode, header},
    response::Response,
    routing::{get, put},
};
use futures::{SinkExt, StreamExt};
use hashtree_core::{sha256, to_hex};
use serde_json::{Value, json};
use tempfile::{TempDir, tempdir};
use tokio::net::TcpListener;
use tokio::sync::{Mutex, broadcast};

const WAIT_TIMEOUT: Duration = Duration::from_secs(25);
const POLL_INTERVAL: Duration = Duration::from_millis(250);
static LIVE_DAEMON_TEST_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

type DirSnapshot = BTreeMap<String, FileSnapshot>;

async fn live_daemon_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    LIVE_DAEMON_TEST_LOCK.lock().await
}

fn bytes_per_second(bytes: u64, elapsed: Duration) -> u64 {
    let millis = elapsed.as_millis().max(1);
    let rate = u128::from(bytes).saturating_mul(1_000) / millis;
    u64::try_from(rate).unwrap_or(u64::MAX)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSnapshot {
    len: u64,
    sha256: String,
}

impl FileSnapshot {
    const fn len(&self) -> u64 {
        self.len
    }
}

#[derive(Clone)]
struct SeedFile {
    client: Client,
    path: String,
    bytes: Vec<u8>,
}

impl SeedFile {
    fn new(client: Client, path: &str, bytes: &[u8]) -> Self {
        Self {
            client,
            path: path.to_string(),
            bytes: bytes.to_vec(),
        }
    }
}

#[derive(Default)]
struct SyncClusterOptions {
    blossom_upload_delay: Duration,
    seed_files: Vec<SeedFile>,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_sync_windows_create_edit_rename_delete_to_ubuntu_peer() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start(Duration::from_millis(250)).await;
    cluster.wait_until_authorized().await;

    cluster
        .write(
            Client::Windows,
            "new-from-windows.txt",
            b"version 1 from windows",
        )
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "windows create")
        .await;
    cluster.assert_file(
        Client::Ubuntu,
        "new-from-windows.txt",
        b"version 1 from windows",
    );

    cluster
        .write(
            Client::Windows,
            "new-from-windows.txt",
            b"version 2 from windows",
        )
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "windows edit")
        .await;
    cluster.assert_file(
        Client::Ubuntu,
        "new-from-windows.txt",
        b"version 2 from windows",
    );

    cluster
        .write(Client::Ubuntu, "ubuntu/nested.txt", b"ubuntu side create")
        .await;
    cluster
        .wait_for_convergence_from(Client::Ubuntu, "ubuntu nested create")
        .await;
    cluster.assert_file(Client::Windows, "ubuntu/nested.txt", b"ubuntu side create");

    cluster
        .rename(
            Client::Windows,
            "new-from-windows.txt",
            "renamed-from-windows.txt",
        )
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "windows rename")
        .await;
    cluster.assert_missing(Client::Ubuntu, "new-from-windows.txt");
    cluster.assert_file(
        Client::Ubuntu,
        "renamed-from-windows.txt",
        b"version 2 from windows",
    );

    cluster.remove(Client::Ubuntu, "ubuntu/nested.txt").await;
    cluster
        .wait_for_convergence_from(Client::Ubuntu, "ubuntu delete")
        .await;
    cluster.assert_missing(Client::Windows, "ubuntu/nested.txt");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_sync_when_relay_drops_root_events_after_fips_connect() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start(Duration::ZERO).await;
    cluster.wait_until_authorized().await;
    cluster.wait_until_direct_peers_connected().await;
    cluster.drop_relay_kinds(&[
        iris_drive_core::nostr_events::KIND_DRIVE_ROOT,
        iris_drive_core::nostr_events::KIND_HASHTREE_ROOT,
    ]);

    cluster
        .write(
            Client::Windows,
            "direct-fips-root.txt",
            b"root event moved over direct fips",
        )
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "direct fips root sync")
        .await;
    cluster.assert_file(
        Client::Ubuntu,
        "direct-fips-root.txt",
        b"root event moved over direct fips",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_initial_merge_existing_trees_from_both_peers() {
    let _guard = live_daemon_test_guard().await;
    let seed_files = vec![
        SeedFile::new(
            Client::Windows,
            "seed/windows/alpha.txt",
            b"already on windows",
        ),
        SeedFile::new(Client::Ubuntu, "seed/ubuntu/beta.txt", b"already on ubuntu"),
        SeedFile::new(Client::Windows, "shared/same.txt", b"same bytes"),
        SeedFile::new(
            Client::Ubuntu,
            "unicode/Raksmorgas-动作-Адрес.txt",
            b"unicode path bytes",
        ),
    ];
    let cluster = SyncCluster::start_with_options(SyncClusterOptions {
        blossom_upload_delay: Duration::ZERO,
        seed_files,
    })
    .await;
    cluster.wait_until_authorized().await;

    let mut expected = dir_snapshot(cluster.path(Client::Windows));
    expected.extend(dir_snapshot(cluster.path(Client::Ubuntu)));
    cluster
        .wait_for_snapshot(&expected, "initial two-device merge")
        .await;
    cluster.assert_file(
        Client::Ubuntu,
        "seed/windows/alpha.txt",
        b"already on windows",
    );
    cluster.assert_file(
        Client::Windows,
        "seed/ubuntu/beta.txt",
        b"already on ubuntu",
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_match_seafile_release_operation_sequence() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start(Duration::from_millis(100)).await;
    cluster.wait_until_authorized().await;

    seafile_add_delete_add_sequence(&cluster).await;
    seafile_create_update_sequence(&cluster).await;
    seafile_rename_sequence(&cluster).await;
    seafile_download_side_sequence(&cluster).await;
}

async fn seafile_add_delete_add_sequence(cluster: &SyncCluster) {
    cluster
        .write(Client::Windows, "release/add-delete-add/1.txt", b"aaaaaaaa")
        .await;
    cluster
        .remove_all(Client::Windows, "release/add-delete-add")
        .await;
    cluster
        .write(Client::Windows, "release/add-delete-add/1.txt", b"aaaaaaaa")
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "add-delete-add")
        .await;
}

async fn seafile_create_update_sequence(cluster: &SyncCluster) {
    cluster
        .write(Client::Windows, "release/test/1.txt", b"111")
        .await;
    cluster
        .write(Client::Windows, "release/test/1.txt", b"222")
        .await;
    cluster
        .write(Client::Windows, "release/copy-source/1.txt", b"111")
        .await;
    cluster
        .write(Client::Windows, "release/copy-source/2/2.txt", b"222")
        .await;
    cluster
        .write(Client::Windows, "release/test/copied/1.txt", b"111")
        .await;
    cluster
        .write(Client::Windows, "release/test/copied/2/2.txt", b"222")
        .await;
    cluster.mkdir(Client::Windows, "release/empty").await;
    cluster
        .write(
            Client::Windows,
            "release/empty/test.md",
            b"dddddddddddddddddddddd",
        )
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "create-update-deep-copy")
        .await;
}

async fn seafile_rename_sequence(cluster: &SyncCluster) {
    cluster
        .write(Client::Windows, "release/rename/1.txt", b"111")
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/1.txt",
            "release/rename/2.txt",
        )
        .await;
    cluster
        .write(Client::Windows, "release/rename/3.txt", b"222")
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/2.txt",
            "release/rename/3.txt",
        )
        .await;
    cluster
        .write(Client::Windows, "release/rename/test.txt", b"test")
        .await;
    cluster.mkdir(Client::Windows, "release/rename/test").await;
    cluster
        .write(Client::Windows, "release/rename/4.txt", b"444")
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/test.txt",
            "release/rename/test/test.txt",
        )
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/3.txt",
            "release/rename/test/3.txt",
        )
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/4.txt",
            "release/rename/test/4.txt",
        )
        .await;
    cluster.mkdir(Client::Windows, "release/rename/test2").await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/test",
            "release/rename/test2/test",
        )
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/test2/test",
            "release/rename/test",
        )
        .await;
    cluster
        .write(Client::Windows, "release/rename/test/4.txt", b"444555")
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/test",
            "release/rename/test2/test",
        )
        .await;
    cluster
        .rename(
            Client::Windows,
            "release/rename/test2",
            "release/rename/test3",
        )
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "rename chain")
        .await;
}

async fn seafile_download_side_sequence(cluster: &SyncCluster) {
    cluster
        .write(Client::Ubuntu, "download/1.txt", b"11111")
        .await;
    cluster
        .write(Client::Ubuntu, "download/1.txt", b"22222")
        .await;
    cluster.mkdir(Client::Ubuntu, "download/dir1").await;
    cluster
        .rename(Client::Ubuntu, "download/1.txt", "download/2.txt")
        .await;
    cluster
        .rename(Client::Ubuntu, "download/dir1", "download/dir2")
        .await;
    cluster
        .write(Client::Ubuntu, "download/dir2/1.txt", b"1111111")
        .await;
    cluster
        .rename(Client::Ubuntu, "download/dir2", "download/dir3")
        .await;
    cluster.remove(Client::Ubuntu, "download/dir3/1.txt").await;
    cluster
        .write(Client::Ubuntu, "download/dir4/2.txt", b"2222222")
        .await;
    cluster
        .rename(Client::Ubuntu, "download/dir4", "download/dir3/dir4")
        .await;
    cluster.remove(Client::Ubuntu, "download/2.txt").await;
    cluster.remove_all(Client::Ubuntu, "download/dir3").await;
    cluster
        .wait_for_convergence_from(Client::Ubuntu, "download-side sequence")
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_sync_file_type_replacements() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start(Duration::ZERO).await;
    cluster.wait_until_authorized().await;

    cluster
        .write(Client::Windows, "types/file-to-dir", b"old file")
        .await;
    cluster
        .write(Client::Windows, "types/dir-to-file/old.txt", b"old child")
        .await;
    cluster
        .write(
            Client::Windows,
            "types/non-empty-dir/old.txt",
            b"old non-empty child",
        )
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "initial file types")
        .await;

    cluster
        .remove_all(Client::Windows, "types/file-to-dir")
        .await;
    cluster
        .write(Client::Windows, "types/file-to-dir/new.txt", b"new child")
        .await;
    cluster
        .remove_all(Client::Windows, "types/dir-to-file")
        .await;
    cluster
        .write(Client::Windows, "types/dir-to-file", b"new file")
        .await;
    cluster
        .remove_all(Client::Windows, "types/non-empty-dir")
        .await;
    cluster
        .write(Client::Windows, "types/non-empty-dir", b"replacement file")
        .await;

    cluster
        .wait_for_convergence_from(Client::Windows, "file type replacements")
        .await;
    cluster.assert_file(Client::Ubuntu, "types/file-to-dir/new.txt", b"new child");
    cluster.assert_file(Client::Ubuntu, "types/dir-to-file", b"new file");
    cluster.assert_file(Client::Ubuntu, "types/non-empty-dir", b"replacement file");
    cluster.assert_missing(Client::Ubuntu, "types/dir-to-file/old.txt");
    cluster.assert_missing(Client::Ubuntu, "types/non-empty-dir/old.txt");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_ignore_noise_and_temporary_files() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start(Duration::ZERO).await;
    cluster.wait_until_authorized().await;

    cluster
        .write(Client::Windows, "noise/keep.txt", b"keep")
        .await;
    cluster
        .write(Client::Windows, "noise/.DS_Store", b"finder")
        .await;
    cluster
        .write(Client::Windows, "noise/._keep.txt", b"resource fork")
        .await;
    cluster
        .write(Client::Windows, "noise/Thumbs.db", b"windows thumbs")
        .await;
    cluster
        .write(Client::Windows, "noise/desktop.ini", b"windows desktop")
        .await;
    cluster
        .write(Client::Windows, "noise/draft~", b"editor backup")
        .await;
    cluster
        .write(Client::Windows, "noise/#draft#", b"emacs backup")
        .await;
    cluster
        .write(Client::Windows, "noise/backup.sbak", b"seafile backup")
        .await;
    cluster
        .write(Client::Windows, ".hashtree/prev", b"internal")
        .await;
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        cluster.path(Client::Windows).join("noise/keep.txt"),
        cluster.path(Client::Windows).join("noise/link.txt"),
    )
    .unwrap();

    let expected = visible_dir_snapshot(cluster.path(Client::Windows));
    cluster
        .wait_for_visible_snapshot(&expected, "ignored noise")
        .await;
    cluster.assert_file(Client::Ubuntu, "noise/keep.txt", b"keep");
    for ignored in [
        "noise/.DS_Store",
        "noise/._keep.txt",
        "noise/Thumbs.db",
        "noise/desktop.ini",
        "noise/draft~",
        "noise/#draft#",
        "noise/backup.sbak",
        ".hashtree/prev",
        "noise/link.txt",
    ] {
        cluster.assert_missing(Client::Ubuntu, ignored);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_reconnect_sender_and_receiver_without_losing_updates() {
    let _guard = live_daemon_test_guard().await;
    let mut cluster = SyncCluster::start(Duration::ZERO).await;
    cluster.wait_until_authorized().await;
    cluster.wait_until_direct_peers_connected().await;

    for index in 0..16 {
        let path = format!("reconnect/baseline/file-{index:03}.bin");
        cluster
            .write(Client::Windows, &path, &deterministic_bytes(index, 512))
            .await;
    }
    cluster
        .wait_for_convergence_from(Client::Windows, "reconnect baseline")
        .await;

    cluster.stop_daemon(Client::Ubuntu);
    for index in 0..24 {
        let path = format!("reconnect/receiver-stopped/file-{index:03}.bin");
        cluster
            .write(
                Client::Windows,
                &path,
                &deterministic_bytes(100 + index, 768),
            )
            .await;
    }
    cluster.start_daemon(Client::Ubuntu);
    cluster.wait_until_direct_peers_connected().await;
    cluster
        .wait_for_convergence_from(Client::Windows, "receiver restarted")
        .await;

    cluster.stop_daemon(Client::Windows);
    for index in 0..24 {
        let path = format!("reconnect/sender-stopped/file-{index:03}.bin");
        cluster
            .write(
                Client::Ubuntu,
                &path,
                &deterministic_bytes(200 + index, 768),
            )
            .await;
    }
    cluster.start_daemon(Client::Windows);
    cluster.wait_until_direct_peers_connected().await;
    cluster
        .wait_for_convergence_from(Client::Ubuntu, "sender restarted")
        .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "transfer bench; run with `cargo test -p idrive --test daemon_sync_matrix -- --ignored --nocapture`"]
async fn bench_live_daemon_transfer_many_files() {
    let _guard = live_daemon_test_guard().await;
    let files = env_usize("IRIS_DRIVE_SYNC_BENCH_FILES", 1_000);
    let bytes_per_file = env_usize("IRIS_DRIVE_SYNC_BENCH_FILE_BYTES", 4096);
    let cluster = SyncCluster::start(Duration::ZERO).await;
    cluster.wait_until_authorized().await;

    let start = Instant::now();
    for index in 0..files {
        let path = format!("many/{:04}/file-{:06}.bin", index / 100, index);
        let bytes = deterministic_bytes(index, bytes_per_file);
        cluster.write(Client::Windows, &path, &bytes).await;
    }
    let expected = dir_snapshot(cluster.path(Client::Windows));
    cluster
        .wait_for_snapshot(&expected, "many-file transfer")
        .await;
    let elapsed = start.elapsed();
    let total_bytes = expected.values().map(FileSnapshot::len).sum::<u64>();

    println!(
        "{}",
        json!({
            "bench": "live_daemon_transfer_many_files",
            "files": files,
            "bytes": total_bytes,
            "elapsed_ms": elapsed.as_millis(),
            "bytes_per_second": bytes_per_second(total_bytes, elapsed),
        })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "large-file transfer bench; set IRIS_DRIVE_SYNC_BENCH_LARGE_BYTES to size"]
async fn bench_live_daemon_transfer_large_file() {
    let _guard = live_daemon_test_guard().await;
    let bytes = env_usize("IRIS_DRIVE_SYNC_BENCH_LARGE_BYTES", 64 * 1024 * 1024);
    let cluster = SyncCluster::start(Duration::ZERO).await;
    cluster.wait_until_authorized().await;

    let payload = deterministic_bytes(7, bytes);
    let start = Instant::now();
    cluster
        .write(Client::Windows, "large/one-file.bin", &payload)
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "large-file transfer")
        .await;
    let elapsed = start.elapsed();

    println!(
        "{}",
        json!({
            "bench": "live_daemon_transfer_large_file",
            "bytes": bytes,
            "elapsed_ms": elapsed.as_millis(),
            "bytes_per_second": bytes_per_second(u64::try_from(bytes).unwrap_or(u64::MAX), elapsed),
        })
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "same-files convergence bench; set IRIS_DRIVE_SYNC_BENCH_FILES and IRIS_DRIVE_SYNC_BENCH_FILE_BYTES"]
async fn bench_live_daemon_same_files_already_present() {
    let _guard = live_daemon_test_guard().await;
    let files = env_usize("IRIS_DRIVE_SYNC_BENCH_FILES", 1_000);
    let bytes_per_file = env_usize("IRIS_DRIVE_SYNC_BENCH_FILE_BYTES", 4096);
    let mut seed_files = Vec::new();
    let mut total_bytes = 0u64;
    for index in 0..files {
        let path = format!("same/{:04}/file-{:06}.bin", index / 100, index);
        let bytes = deterministic_bytes(index, bytes_per_file);
        total_bytes += u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        seed_files.push(SeedFile::new(Client::Windows, &path, &bytes));
        seed_files.push(SeedFile::new(Client::Ubuntu, &path, &bytes));
    }

    let start = Instant::now();
    let cluster = SyncCluster::start_with_options(SyncClusterOptions {
        blossom_upload_delay: Duration::ZERO,
        seed_files,
    })
    .await;
    cluster.wait_until_authorized().await;
    cluster
        .wait_for_convergence_from(Client::Windows, "same-files already present")
        .await;
    let elapsed = start.elapsed();

    println!(
        "{}",
        json!({
            "bench": "live_daemon_same_files_already_present",
            "files": files,
            "bytes": total_bytes,
            "elapsed_ms": elapsed.as_millis(),
            "bytes_per_second": bytes_per_second(total_bytes, elapsed),
        })
    );
}

#[derive(Clone, Copy)]
enum Client {
    Windows,
    Ubuntu,
}

struct SyncCluster {
    relay: LocalNostrRelay,
    _blossom: LocalBlossomServer,
    windows_cfg: TempDir,
    ubuntu_cfg: TempDir,
    windows_work: TempDir,
    ubuntu_work: TempDir,
    windows_daemon: Option<DaemonChild>,
    ubuntu_daemon: Option<DaemonChild>,
}

impl SyncCluster {
    async fn start(blossom_upload_delay: Duration) -> Self {
        Self::start_with_options(SyncClusterOptions {
            blossom_upload_delay,
            ..SyncClusterOptions::default()
        })
        .await
    }

    async fn start_with_options(options: SyncClusterOptions) -> Self {
        let relay = LocalNostrRelay::spawn().await;
        let blossom = LocalBlossomServer::spawn(options.blossom_upload_delay).await;

        let windows_cfg = tempdir().unwrap();
        let ubuntu_cfg = tempdir().unwrap();
        let windows_work = tempdir().unwrap();
        let ubuntu_work = tempdir().unwrap();

        configure_local_blossom(windows_cfg.path(), &blossom.url);
        configure_local_blossom(ubuntu_cfg.path(), &blossom.url);

        let init = run_json(windows_cfg.path(), &["init", "--label", "win11-dev"]);
        let owner_npub = init["owner_npub"].as_str().unwrap();
        let linked = run_json(
            ubuntu_cfg.path(),
            &["link", owner_npub, "--label", "ubuntu-dev"],
        );
        let request = linked["device_link_request"]["url"].as_str().unwrap();
        run_json(windows_cfg.path(), &["approve", request]);

        for seed in &options.seed_files {
            let root = match seed.client {
                Client::Windows => windows_work.path(),
                Client::Ubuntu => ubuntu_work.path(),
            };
            let path = root.join(&seed.path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, &seed.bytes).unwrap();
        }

        run_json(
            windows_cfg.path(),
            &["import", windows_work.path().to_str().unwrap()],
        );
        run_json(
            ubuntu_cfg.path(),
            &["import", ubuntu_work.path().to_str().unwrap()],
        );

        let windows_daemon = Some(DaemonChild::spawn(
            windows_cfg.path(),
            &relay.url,
            windows_cfg.path().join("win.log"),
        ));
        let ubuntu_daemon = Some(DaemonChild::spawn(
            ubuntu_cfg.path(),
            &relay.url,
            ubuntu_cfg.path().join("ubuntu.log"),
        ));

        Self {
            relay,
            _blossom: blossom,
            windows_cfg,
            ubuntu_cfg,
            windows_work,
            ubuntu_work,
            windows_daemon,
            ubuntu_daemon,
        }
    }

    async fn wait_until_authorized(&self) {
        self.wait_until("ubuntu authorized", || {
            let status = run_json(self.ubuntu_cfg.path(), &["status"]);
            status["account"]["authorization_state"] == "authorized"
        })
        .await;
    }

    async fn wait_until_direct_peers_connected(&self) {
        self.wait_until("direct fips peers connected", || {
            let windows = run_json(self.windows_cfg.path(), &["status"]);
            let ubuntu = run_json(self.ubuntu_cfg.path(), &["status"]);
            windows["network"]["fips"]["connected_peer_count"]
                .as_u64()
                .unwrap_or(0)
                > 0
                && ubuntu["network"]["fips"]["connected_peer_count"]
                    .as_u64()
                    .unwrap_or(0)
                    > 0
        })
        .await;
    }

    fn drop_relay_kinds(&self, kinds: &[u16]) {
        self.relay.drop_kinds(kinds);
    }

    async fn write(&self, client: Client, path: &str, bytes: &[u8]) {
        let path = self.path(client).join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        tokio::fs::write(path, bytes).await.unwrap();
    }

    async fn rename(&self, client: Client, from: &str, to: &str) {
        let root = self.path(client);
        let destination = root.join(to);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        tokio::fs::rename(root.join(from), destination)
            .await
            .unwrap();
    }

    async fn remove(&self, client: Client, path: &str) {
        tokio::fs::remove_file(self.path(client).join(path))
            .await
            .unwrap();
    }

    async fn remove_all(&self, client: Client, path: &str) {
        let path = self.path(client).join(path);
        let metadata = match tokio::fs::symlink_metadata(&path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("metadata failed for {}: {error}", path.display()),
        };
        if metadata.is_dir() {
            tokio::fs::remove_dir_all(path).await.unwrap();
        } else {
            tokio::fs::remove_file(path).await.unwrap();
        }
    }

    async fn mkdir(&self, client: Client, path: &str) {
        tokio::fs::create_dir_all(self.path(client).join(path))
            .await
            .unwrap();
    }

    fn assert_file(&self, client: Client, path: &str, expected: &[u8]) {
        let actual = std::fs::read(self.path(client).join(path)).unwrap();
        assert_eq!(actual, expected, "{}", self.debug_state());
    }

    fn assert_missing(&self, client: Client, path: &str) {
        assert!(
            !self.path(client).join(path).exists(),
            "{} should be absent\n{}",
            path,
            self.debug_state()
        );
    }

    async fn wait_for_convergence_from(&self, client: Client, label: &str) {
        let expected = dir_snapshot(self.path(client));
        self.wait_for_snapshot(&expected, label).await;
    }

    async fn wait_for_snapshot(&self, expected: &DirSnapshot, label: &str) {
        self.wait_until(label, || {
            dir_snapshot(self.windows_work.path()) == *expected
                && dir_snapshot(self.ubuntu_work.path()) == *expected
        })
        .await;
    }

    async fn wait_for_visible_snapshot(&self, expected: &DirSnapshot, label: &str) {
        self.wait_until(label, || {
            visible_dir_snapshot(self.windows_work.path()) == *expected
                && visible_dir_snapshot(self.ubuntu_work.path()) == *expected
        })
        .await;
    }

    async fn wait_until(&self, label: &str, mut ready: impl FnMut() -> bool) {
        let start = Instant::now();
        while start.elapsed() < WAIT_TIMEOUT {
            if ready() {
                return;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        panic!("timed out waiting for {label}\n{}", self.debug_state());
    }

    fn path(&self, client: Client) -> &Path {
        match client {
            Client::Windows => self.windows_work.path(),
            Client::Ubuntu => self.ubuntu_work.path(),
        }
    }

    fn stop_daemon(&mut self, client: Client) {
        let daemon = match client {
            Client::Windows => &mut self.windows_daemon,
            Client::Ubuntu => &mut self.ubuntu_daemon,
        };
        drop(daemon.take());
    }

    fn start_daemon(&mut self, client: Client) {
        let (slot, config_dir, log_path) = match client {
            Client::Windows => (
                &mut self.windows_daemon,
                self.windows_cfg.path(),
                self.windows_cfg.path().join("win.log"),
            ),
            Client::Ubuntu => (
                &mut self.ubuntu_daemon,
                self.ubuntu_cfg.path(),
                self.ubuntu_cfg.path().join("ubuntu.log"),
            ),
        };
        assert!(slot.is_none(), "daemon is already running");
        *slot = Some(DaemonChild::spawn(config_dir, &self.relay.url, log_path));
    }

    fn debug_state(&self) -> String {
        format!(
            "windows: {:#?}\nubuntu: {:#?}\nwindows status: {}\nubuntu status: {}\nwindows log:\n{}\nubuntu log:\n{}",
            dir_snapshot(self.windows_work.path()),
            dir_snapshot(self.ubuntu_work.path()),
            serde_json::to_string_pretty(&run_json(self.windows_cfg.path(), &["status"]))
                .unwrap_or_default(),
            serde_json::to_string_pretty(&run_json(self.ubuntu_cfg.path(), &["status"]))
                .unwrap_or_default(),
            self.windows_daemon
                .as_ref()
                .map_or_else(|| "<stopped>".to_string(), DaemonChild::log),
            self.ubuntu_daemon
                .as_ref()
                .map_or_else(|| "<stopped>".to_string(), DaemonChild::log),
        )
    }
}

struct DaemonChild {
    child: Child,
    log_path: PathBuf,
}

impl DaemonChild {
    fn spawn(config_dir: &Path, relay_url: &str, log_path: PathBuf) -> Self {
        let mut stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .unwrap();
        writeln!(stdout, "\n--- daemon start ---").unwrap();
        let stderr = stdout.try_clone().unwrap();
        let child = Command::new(idrive_bin())
            .env("IRIS_DRIVE_CONFIG_DIR", config_dir)
            .args([
                "daemon",
                "--relay",
                relay_url,
                "--watch-interval",
                "1",
                "--watch-debounce-ms",
                "100",
                "--no-gateway",
            ])
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(stderr))
            .spawn()
            .unwrap();
        Self { child, log_path }
    }

    fn log(&self) -> String {
        std::fs::read_to_string(&self.log_path).unwrap_or_default()
    }
}

impl Drop for DaemonChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn idrive_bin() -> PathBuf {
    assert_cmd::cargo::cargo_bin("idrive")
}

fn idrive(config_dir: &Path) -> Command {
    let mut command = Command::new(idrive_bin());
    command.env("IRIS_DRIVE_CONFIG_DIR", config_dir);
    command
}

fn run_json(config_dir: &Path, args: &[&str]) -> Value {
    let output = idrive(config_dir).args(args).output().unwrap();
    assert_success(&output);
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid json: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "command failed\nstatus: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn configure_local_blossom(config_dir: &Path, url: &str) {
    assert_success(
        &idrive(config_dir)
            .args(["blossom-servers", "remove", "https://upload.iris.to"])
            .output()
            .unwrap(),
    );
    assert_success(
        &idrive(config_dir)
            .args(["blossom-servers", "add", url])
            .output()
            .unwrap(),
    );
}

fn dir_snapshot(root: &Path) -> DirSnapshot {
    let mut snapshot = BTreeMap::new();
    collect_dir_snapshot(root, root, &mut snapshot, SnapshotFilter::All);
    snapshot
}

fn visible_dir_snapshot(root: &Path) -> DirSnapshot {
    let mut snapshot = BTreeMap::new();
    collect_dir_snapshot(root, root, &mut snapshot, SnapshotFilter::UserVisible);
    snapshot
}

#[derive(Clone, Copy)]
enum SnapshotFilter {
    All,
    UserVisible,
}

fn collect_dir_snapshot(
    root: &Path,
    dir: &Path,
    snapshot: &mut DirSnapshot,
    filter: SnapshotFilter,
) {
    let mut entries = std::fs::read_dir(dir)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    entries.sort_by_key(std::fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if matches!(filter, SnapshotFilter::UserVisible) && should_ignore_name(&name) {
            continue;
        }
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            collect_dir_snapshot(root, &path, snapshot, filter);
        } else if file_type.is_file() {
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .iter()
                .map(|part| part.to_string_lossy())
                .collect::<Vec<_>>()
                .join("/");
            let bytes = std::fs::read(&path).unwrap();
            snapshot.insert(
                relative,
                FileSnapshot {
                    len: bytes.len() as u64,
                    sha256: to_hex(&sha256(&bytes)),
                },
            );
        }
    }
}

fn should_ignore_name(name: &str) -> bool {
    matches!(
        name,
        ".DS_Store" | ".hashtree" | "Thumbs.db" | "desktop.ini"
    ) || name.starts_with("._")
        || name.ends_with('~')
        || (name.starts_with('#') && name.ends_with('#'))
        || Path::new(name)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("sbak"))
}

fn deterministic_bytes(seed: usize, len: usize) -> Vec<u8> {
    let mut value = seed as u64 ^ 0xA5A5_5A5A_1234_5678;
    let mut bytes = Vec::with_capacity(len);
    while bytes.len() < len {
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    bytes.truncate(len);
    bytes
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[derive(Clone)]
struct LocalRelayState {
    events: Arc<Mutex<Vec<Value>>>,
    broadcasts: broadcast::Sender<Value>,
    drop_kinds: Arc<StdMutex<BTreeSet<u64>>>,
}

struct LocalNostrRelay {
    url: String,
    task: tokio::task::JoinHandle<()>,
    drop_kinds: Arc<StdMutex<BTreeSet<u64>>>,
}

impl LocalNostrRelay {
    async fn spawn() -> Self {
        let (broadcasts, _rx) = broadcast::channel(256);
        let state = LocalRelayState {
            events: Arc::new(Mutex::new(Vec::new())),
            broadcasts,
            drop_kinds: Arc::new(StdMutex::new(BTreeSet::new())),
        };
        let drop_kinds = state.drop_kinds.clone();
        let app = Router::new().route("/", get(relay_ws)).with_state(state);
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url: format!("ws://{addr}"),
            task,
            drop_kinds,
        }
    }

    fn drop_kinds(&self, kinds: &[u16]) {
        self.drop_kinds
            .lock()
            .unwrap()
            .extend(kinds.iter().map(|kind| u64::from(*kind)));
    }
}

impl Drop for LocalNostrRelay {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone)]
struct Subscription {
    id: String,
    filters: Vec<Value>,
}

async fn relay_ws(ws: WebSocketUpgrade, State(state): State<LocalRelayState>) -> Response {
    ws.on_upgrade(move |socket| relay_socket(socket, state))
}

async fn relay_socket(socket: WebSocket, state: LocalRelayState) {
    let (mut sender, mut receiver) = socket.split();
    let mut broadcasts = state.broadcasts.subscribe();
    let mut subscriptions: Vec<Subscription> = Vec::new();

    loop {
        tokio::select! {
            message = receiver.next() => {
                let Some(Ok(message)) = message else {
                    break;
                };
                let text = match message {
                    WsMessage::Text(text) => text,
                    WsMessage::Ping(bytes) => {
                        let _ = sender.send(WsMessage::Pong(bytes)).await;
                        continue;
                    }
                    WsMessage::Close(_) => break,
                    _ => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };
                let Some(items) = value.as_array() else {
                    continue;
                };
                let Some(command) = items.first().and_then(Value::as_str) else {
                    continue;
                };
                match command {
                    "EVENT" => {
                        let Some(event) = items.get(1).cloned() else {
                            continue;
                        };
                        let event_id = event["id"].as_str().unwrap_or_default().to_string();
                        let kind = event["kind"].as_u64().unwrap_or_default();
                        if state.drop_kinds.lock().unwrap().contains(&kind) {
                            let _ = sender
                                .send(WsMessage::Text(json!(["OK", event_id, true, ""]).to_string()))
                                .await;
                            continue;
                        }
                        state.events.lock().await.push(event.clone());
                        let _ = state.broadcasts.send(event);
                        let _ = sender
                            .send(WsMessage::Text(json!(["OK", event_id, true, ""]).to_string()))
                            .await;
                    }
                    "REQ" => {
                        let Some(subscription_id) = items.get(1).and_then(Value::as_str) else {
                            continue;
                        };
                        let filters = items.iter().skip(2).cloned().collect::<Vec<_>>();
                        subscriptions.push(Subscription {
                            id: subscription_id.to_string(),
                            filters: filters.clone(),
                        });
                        let events = state.events.lock().await.clone();
                        for event in events {
                            if filters.iter().any(|filter| event_matches_filter(&event, filter)) {
                                let _ = sender
                                    .send(WsMessage::Text(
                                        json!(["EVENT", subscription_id, event]).to_string(),
                                    ))
                                    .await;
                            }
                        }
                        let _ = sender
                            .send(WsMessage::Text(json!(["EOSE", subscription_id]).to_string()))
                            .await;
                    }
                    "CLOSE" => {
                        if let Some(subscription_id) = items.get(1).and_then(Value::as_str) {
                            subscriptions.retain(|subscription| subscription.id != subscription_id);
                        }
                    }
                    _ => {}
                }
            }
            event = broadcasts.recv() => {
                let Ok(event) = event else {
                    continue;
                };
                for subscription in &subscriptions {
                    if subscription
                        .filters
                        .iter()
                        .any(|filter| event_matches_filter(&event, filter))
                    {
                        let _ = sender
                            .send(WsMessage::Text(
                                json!(["EVENT", subscription.id, event]).to_string(),
                            ))
                            .await;
                    }
                }
            }
        }
    }
}

fn event_matches_filter(event: &Value, filter: &Value) -> bool {
    if let Some(kinds) = filter.get("kinds").and_then(Value::as_array) {
        let Some(kind) = event.get("kind").and_then(Value::as_u64) else {
            return false;
        };
        if !kinds
            .iter()
            .any(|candidate| candidate.as_u64() == Some(kind))
        {
            return false;
        }
    }
    if let Some(authors) = filter.get("authors").and_then(Value::as_array) {
        let Some(author) = event.get("pubkey").and_then(Value::as_str) else {
            return false;
        };
        if !authors
            .iter()
            .any(|candidate| candidate.as_str() == Some(author))
        {
            return false;
        }
    }
    if let Some(d_values) = filter.get("#d").and_then(Value::as_array) {
        let Some(tags) = event.get("tags").and_then(Value::as_array) else {
            return false;
        };
        let has_matching_d_tag = tags.iter().any(|tag| {
            let Some(tag_items) = tag.as_array() else {
                return false;
            };
            tag_items.first().and_then(Value::as_str) == Some("d")
                && tag_items
                    .get(1)
                    .and_then(Value::as_str)
                    .is_some_and(|value| {
                        d_values
                            .iter()
                            .any(|candidate| candidate.as_str() == Some(value))
                    })
        });
        if !has_matching_d_tag {
            return false;
        }
    }
    true
}

#[derive(Clone)]
struct LocalBlossomState {
    blobs: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    upload_delay: Duration,
}

struct LocalBlossomServer {
    url: String,
    task: tokio::task::JoinHandle<()>,
}

impl LocalBlossomServer {
    async fn spawn(upload_delay: Duration) -> Self {
        let state = LocalBlossomState {
            blobs: Arc::new(Mutex::new(BTreeMap::new())),
            upload_delay,
        };
        let app = Router::new()
            .route("/upload", put(blossom_upload))
            .route("/:name", get(blossom_get).head(blossom_head))
            .with_state(state);
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
            .await
            .unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            url: format!("http://{addr}"),
            task,
        }
    }
}

impl Drop for LocalBlossomServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn blossom_upload(
    State(state): State<LocalBlossomState>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !state.upload_delay.is_zero() {
        tokio::time::sleep(state.upload_delay).await;
    }
    let hash = to_hex(&sha256(&body));
    if let Some(expected) = headers
        .get("x-sha-256")
        .and_then(|value| value.to_str().ok())
        && expected != hash
    {
        return text_response(StatusCode::BAD_REQUEST, "hash mismatch");
    }
    let mut blobs = state.blobs.lock().await;
    if blobs.contains_key(&hash) {
        return text_response(StatusCode::CONFLICT, "already exists");
    }
    blobs.insert(hash, body.to_vec());
    text_response(StatusCode::CREATED, "created")
}

async fn blossom_get(
    State(state): State<LocalBlossomState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    let Some(hash) = name.strip_suffix(".bin") else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };
    let Some(bytes) = state.blobs.lock().await.get(hash).cloned() else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };
    blob_response(StatusCode::OK, bytes)
}

async fn blossom_head(
    State(state): State<LocalBlossomState>,
    AxumPath(name): AxumPath<String>,
) -> Response {
    let Some(hash) = name.strip_suffix(".bin") else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };
    let Some(size) = state.blobs.lock().await.get(hash).map(Vec::len) else {
        return text_response(StatusCode::NOT_FOUND, "not found");
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, size.to_string())
        .body(Body::empty())
        .unwrap()
}

fn blob_response(status: StatusCode, bytes: Vec<u8>) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .body(Body::from(bytes))
        .unwrap()
}

fn text_response(status: StatusCode, text: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(text.to_string()))
        .unwrap()
}
