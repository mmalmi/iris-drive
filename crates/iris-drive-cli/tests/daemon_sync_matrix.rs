//! Live daemon sync tests and transfer benches.
//!
//! The shape follows Seafile's sync-auto-test and Syncthing's integration
//! benches: run real clients, mutate one worktree at a time, wait for
//! convergence, then compare on-disk contents instead of trusting status text.
//!
//! Development loop:
//! - Rerun one failure:
//!   `cargo test -p idrive --test daemon_sync_matrix <test-name> -- --exact --nocapture`
//! - Stop the matrix after the first failure:
//!   `cargo nextest run -p idrive --test daemon_sync_matrix --fail-fast`

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use hashtree_core::{Cid, LinkType, sha256, to_hex};
use serde_json::Value;
use tempfile::{TempDir, tempdir};
use tokio::sync::Mutex;

mod support;

include!("daemon_sync_matrix/harness.rs");
#[path = "daemon_sync_matrix/provider_harness.rs"]
mod provider_harness;
use support::{LocalBlossomServer, LocalNostrRelay, add_config_relay};

const WAIT_TIMEOUT: Duration = Duration::from_mins(3);
const POLL_INTERVAL: Duration = Duration::from_millis(250);
const REFRESH_VIEW_TIMEOUT: Duration = Duration::from_secs(30);
const CLI_COMMAND_TIMEOUT: Duration = Duration::from_mins(1);
const CLI_COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(50);
static LIVE_DAEMON_TEST_LOCK: std::sync::LazyLock<Mutex<()>> =
    std::sync::LazyLock::new(|| Mutex::new(()));

type DirSnapshot = BTreeMap<String, FileSnapshot>;

fn current_test_name() -> String {
    std::thread::current()
        .name()
        .unwrap_or("daemon_sync_matrix")
        .to_string()
}

fn rerun_hint(test_name: &str) -> String {
    format!(
        "rerun this test: cargo test -p idrive --test daemon_sync_matrix {test_name} -- --exact --nocapture\n\
         rerun matrix with fast fail: cargo nextest run -p idrive --test daemon_sync_matrix --fail-fast"
    )
}

fn matrix_progress(label: impl AsRef<str>) {
    eprintln!("[daemon-sync-matrix] {}", label.as_ref());
}

async fn live_daemon_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    LIVE_DAEMON_TEST_LOCK.lock().await
}

fn bytes_per_second(bytes: u64, elapsed: Duration) -> u64 {
    let millis = elapsed.as_millis().max(1);
    let rate = u128::from(bytes).saturating_mul(1_000) / millis;
    u64::try_from(rate).unwrap_or(u64::MAX)
}

#[derive(Clone, PartialEq, Eq)]
struct FileSnapshot {
    len: u64,
    sha256: String,
    bytes: Vec<u8>,
}

impl FileSnapshot {
    const fn len(&self) -> u64 {
        self.len
    }
}

impl std::fmt::Debug for FileSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSnapshot")
            .field("len", &self.len)
            .field("sha256", &self.sha256)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
struct SyncLatency {
    root_cid: String,
    local_edit_to_remote_visible: Duration,
    source_viewer_done_to_remote_visible: Duration,
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

struct SyncClusterOptions {
    blossom_upload_delay: Duration,
    seed_files: Vec<SeedFile>,
    clients: Vec<Client>,
}

impl Default for SyncClusterOptions {
    fn default() -> Self {
        Self {
            blossom_upload_delay: Duration::ZERO,
            seed_files: Vec::new(),
            clients: vec![Client::Windows, Client::Ubuntu],
        }
    }
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
async fn live_daemons_linked_devices_see_each_others_edits_after_authorization() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start(Duration::from_millis(250)).await;
    cluster.wait_until_authorized().await;

    cluster
        .write(
            Client::Windows,
            "linked/windows-note.txt",
            b"created after device authorization on windows",
        )
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "authorized windows edit")
        .await;
    cluster.assert_file(
        Client::Ubuntu,
        "linked/windows-note.txt",
        b"created after device authorization on windows",
    );

    cluster
        .write(
            Client::Ubuntu,
            "linked/ubuntu-note.txt",
            b"created after device authorization on ubuntu",
        )
        .await;
    cluster
        .wait_for_convergence_from(Client::Ubuntu, "authorized ubuntu edit")
        .await;
    cluster.assert_file(
        Client::Windows,
        "linked/ubuntu-note.txt",
        b"created after device authorization on ubuntu",
    );
    cluster.assert_status_counts(Client::Windows, 2, 2);
    cluster.assert_status_counts(Client::Ubuntu, 2, 2);
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
async fn live_daemons_provider_write_viewer_to_viewer_latency_probe() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start_three(Duration::ZERO).await;
    cluster.wait_until_authorized().await;
    cluster.wait_until_direct_peers_connected().await;

    let hops = [
        (
            Client::Windows,
            Client::Ubuntu,
            "latency/windows-to-ubuntu.txt",
        ),
        (Client::Ubuntu, Client::MacOS, "latency/ubuntu-to-macos.txt"),
        (
            Client::MacOS,
            Client::Windows,
            "latency/macos-to-windows.txt",
        ),
    ];

    for (source, target, path) in hops {
        let bytes = format!(
            "{} to {} viewer latency probe",
            source.label(),
            target.label()
        )
        .into_bytes();
        let label = format!("{} to {} viewer latency", source.label(), target.label());
        let latency = cluster
            .measure_provider_write_to_remote_visible(source, target, path, &bytes, &label)
            .await;
        println!(
            "{}",
            serde_json::json!({
                "event": "viewer_to_viewer_latency_probe",
                "source": source.label(),
                "target": target.label(),
                "path": path,
                "root_cid": &latency.root_cid,
                "local_edit_to_remote_visible_ms": latency.local_edit_to_remote_visible.as_millis(),
                "source_viewer_done_to_remote_visible_ms": latency.source_viewer_done_to_remote_visible.as_millis(),
            })
        );
        assert!(
            latency.source_viewer_done_to_remote_visible < Duration::from_secs(10),
            "{} to {} viewer latency was {:?}, expected under 10s",
            source.label(),
            target.label(),
            latency.source_viewer_done_to_remote_visible,
        );
    }
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
        ..SyncClusterOptions::default()
    })
    .await;
    cluster.wait_until_authorized().await;
    cluster.wait_until_direct_peers_connected().await;

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
async fn live_daemons_three_vm_matrix_syncs_provider_mutations() {
    let _guard = live_daemon_test_guard().await;
    let cluster = SyncCluster::start_three(Duration::ZERO).await;
    cluster.wait_until_authorized().await;

    cluster
        .write(Client::Windows, "three-vm/windows.txt", b"from windows")
        .await;
    cluster
        .wait_for_convergence_from(Client::Windows, "three vm windows create")
        .await;
    cluster.assert_file(Client::Ubuntu, "three-vm/windows.txt", b"from windows");
    cluster.assert_file(Client::MacOS, "three-vm/windows.txt", b"from windows");

    cluster
        .write(Client::Ubuntu, "three-vm/ubuntu.txt", b"from ubuntu")
        .await;
    cluster
        .wait_for_convergence_from(Client::Ubuntu, "three vm ubuntu create")
        .await;
    cluster.assert_file(Client::Windows, "three-vm/ubuntu.txt", b"from ubuntu");
    cluster.assert_file(Client::MacOS, "three-vm/ubuntu.txt", b"from ubuntu");

    let root = cluster
        .provider_write(Client::MacOS, "three-vm/macos.txt", b"from macos provider")
        .await;
    cluster
        .wait_for_provider_publish(Client::MacOS, &root, "macos provider create published")
        .await;
    cluster
        .wait_for_convergence_from(Client::MacOS, "three vm macos provider create")
        .await;
    cluster.assert_file(Client::Ubuntu, "three-vm/macos.txt", b"from macos provider");
    cluster.assert_file(
        Client::Windows,
        "three-vm/macos.txt",
        b"from macos provider",
    );

    let root = cluster
        .provider_rename(
            Client::MacOS,
            "three-vm/macos.txt",
            "three-vm/macos-renamed.txt",
        )
        .await;
    cluster
        .wait_for_provider_publish(Client::MacOS, &root, "macos provider rename published")
        .await;
    cluster
        .wait_for_convergence_from(Client::MacOS, "three vm macos provider rename")
        .await;
    for client in Client::THREE_VM {
        cluster.assert_missing(client, "three-vm/macos.txt");
        cluster.assert_file(client, "three-vm/macos-renamed.txt", b"from macos provider");
    }

    let root = cluster
        .provider_delete(Client::MacOS, "three-vm/macos-renamed.txt")
        .await;
    cluster
        .wait_for_provider_publish(Client::MacOS, &root, "macos provider delete published")
        .await;
    cluster
        .wait_for_convergence_from(Client::MacOS, "three vm macos provider delete")
        .await;
    for client in Client::THREE_VM {
        cluster.assert_missing(client, "three-vm/macos-renamed.txt");
        cluster.assert_status_counts(client, 2, 3);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn live_daemons_three_vm_macos_provider_changes_catch_up_without_source_online() {
    let _guard = live_daemon_test_guard().await;
    let mut cluster = SyncCluster::start_three(Duration::ZERO).await;
    cluster.wait_until_authorized().await;

    cluster.stop_daemon(Client::Windows);
    cluster.stop_daemon(Client::Ubuntu);

    let root = cluster
        .provider_write(
            Client::MacOS,
            "offline-receivers/original.txt",
            b"macos original",
        )
        .await;
    cluster
        .wait_for_provider_publish(Client::MacOS, &root, "macos offline create published")
        .await;

    let root = cluster
        .provider_rename(
            Client::MacOS,
            "offline-receivers/original.txt",
            "offline-receivers/renamed.txt",
        )
        .await;
    cluster
        .wait_for_provider_publish(Client::MacOS, &root, "macos offline rename published")
        .await;

    let root = cluster
        .provider_write(
            Client::MacOS,
            "offline-receivers/added.txt",
            b"macos added while receivers were offline",
        )
        .await;
    cluster
        .wait_for_provider_publish(Client::MacOS, &root, "macos offline add published")
        .await;

    cluster.stop_daemon(Client::MacOS);
    cluster.start_daemon(Client::Windows);
    cluster.start_daemon(Client::Ubuntu);

    cluster
        .wait_for_convergence_from(
            Client::MacOS,
            "offline receivers catch up from macos provider",
        )
        .await;
    for client in [Client::Windows, Client::Ubuntu] {
        cluster.assert_missing(client, "offline-receivers/original.txt");
        cluster.assert_file(client, "offline-receivers/renamed.txt", b"macos original");
        cluster.assert_file(
            client,
            "offline-receivers/added.txt",
            b"macos added while receivers were offline",
        );
        cluster.assert_status_counts(client, 2, 3);
    }
}

#[path = "daemon_sync_matrix/app_key_link_tests.rs"]
mod app_key_link_tests;
#[path = "daemon_sync_matrix/provider_visibility_tests.rs"]
mod provider_visibility_tests;
#[path = "daemon_sync_matrix/scenario_tests.rs"]
mod scenario_tests;
