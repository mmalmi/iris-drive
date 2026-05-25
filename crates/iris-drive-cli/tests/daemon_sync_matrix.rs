//! Live daemon sync tests and transfer benches.
//!
//! The shape follows Seafile's sync-auto-test and Syncthing's integration
//! benches: run real clients, mutate one worktree at a time, wait for
//! convergence, then compare on-disk contents instead of trusting status text.

use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::Write as _;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use hashtree_core::{Cid, LinkType, sha256, to_hex};
use serde_json::Value;
use tempfile::{TempDir, tempdir};
use tokio::sync::Mutex;

mod support;

use support::{LocalBlossomServer, LocalNostrRelay};

const WAIT_TIMEOUT: Duration = Duration::from_secs(90);
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

#[path = "daemon_sync_matrix/scenario_tests.rs"]
mod scenario_tests;

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
    windows_gateway_port: u16,
    ubuntu_gateway_port: u16,
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
        let blossom =
            LocalBlossomServer::spawn_with_upload_delay(options.blossom_upload_delay).await;

        let windows_cfg = tempdir().unwrap();
        let ubuntu_cfg = tempdir().unwrap();
        let windows_work = tempdir().unwrap();
        let ubuntu_work = tempdir().unwrap();
        let windows_gateway_port = unused_loopback_port();
        let ubuntu_gateway_port = unused_loopback_port();

        configure_local_blossom(windows_cfg.path(), &blossom.url);
        configure_local_blossom(ubuntu_cfg.path(), &blossom.url);

        let init = run_json(windows_cfg.path(), &["init", "--label", "windows-peer"]);
        let owner_npub = init["owner_npub"].as_str().unwrap();
        let linked = run_json(
            ubuntu_cfg.path(),
            &["link", owner_npub, "--label", "linux-peer"],
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
            windows_gateway_port,
        ));
        let ubuntu_daemon = Some(DaemonChild::spawn(
            ubuntu_cfg.path(),
            &relay.url,
            ubuntu_cfg.path().join("ubuntu.log"),
            ubuntu_gateway_port,
        ));

        Self {
            relay,
            _blossom: blossom,
            windows_cfg,
            ubuntu_cfg,
            windows_work,
            ubuntu_work,
            windows_gateway_port,
            ubuntu_gateway_port,
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
        self.write_local_only(client, path, bytes).await;
        if !test_ignored_path(path) {
            let response = self.webdav_request(client, "PUT", path, &[], bytes).await;
            assert!(
                response.starts_with("HTTP/1.1 201 Created")
                    || response.starts_with("HTTP/1.1 204 No Content"),
                "{response}\n{}",
                self.debug_state()
            );
        }
    }

    async fn write_local_only(&self, client: Client, path: &str, bytes: &[u8]) {
        let local_path = self.path(client).join(path);
        if let Some(parent) = local_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        tokio::fs::write(local_path, bytes).await.unwrap();
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
        if !test_ignored_path(from) && !test_ignored_path(to) {
            let destination = format!(
                "http://127.0.0.1:{}{}",
                self.gateway_port(client),
                webdav_path(to)
            );
            let response = self
                .webdav_request(client, "MOVE", from, &[("Destination", &destination)], b"")
                .await;
            assert!(
                response.starts_with("HTTP/1.1 201 Created")
                    || response.starts_with("HTTP/1.1 204 No Content"),
                "{response}\n{}",
                self.debug_state()
            );
        }
    }

    async fn remove(&self, client: Client, path: &str) {
        tokio::fs::remove_file(self.path(client).join(path))
            .await
            .unwrap();
        if !test_ignored_path(path) {
            let response = self.webdav_request(client, "DELETE", path, &[], b"").await;
            assert!(
                response.starts_with("HTTP/1.1 204 No Content"),
                "{response}\n{}",
                self.debug_state()
            );
        }
    }

    async fn remove_all(&self, client: Client, path: &str) {
        let relative = path.to_string();
        let local_path = self.path(client).join(path);
        let metadata = match tokio::fs::symlink_metadata(&local_path).await {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
            Err(error) => panic!("metadata failed for {}: {error}", local_path.display()),
        };
        if metadata.is_dir() {
            tokio::fs::remove_dir_all(local_path).await.unwrap();
        } else {
            tokio::fs::remove_file(local_path).await.unwrap();
        }
        if !test_ignored_path(&relative) {
            let response = self
                .webdav_request(client, "DELETE", &relative, &[], b"")
                .await;
            assert!(
                response.starts_with("HTTP/1.1 204 No Content"),
                "{response}\n{}",
                self.debug_state()
            );
        }
    }

    async fn mkdir(&self, client: Client, path: &str) {
        tokio::fs::create_dir_all(self.path(client).join(path))
            .await
            .unwrap();
        if !test_ignored_path(path) {
            let response = self.webdav_request(client, "MKCOL", path, &[], b"").await;
            assert!(
                response.starts_with("HTTP/1.1 201 Created")
                    || response.starts_with("HTTP/1.1 405 Method Not Allowed"),
                "{response}\n{}",
                self.debug_state()
            );
        }
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
        let expected = visible_dir_snapshot(self.path(client));
        self.wait_for_visible_snapshot(&expected, label).await;
    }

    async fn wait_for_snapshot(&self, expected: &DirSnapshot, label: &str) {
        let start = Instant::now();
        while start.elapsed() < WAIT_TIMEOUT {
            self.refresh_view(Client::Windows).await;
            self.refresh_view(Client::Ubuntu).await;
            if dir_snapshot(self.windows_work.path()) == *expected
                && dir_snapshot(self.ubuntu_work.path()) == *expected
            {
                return;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        panic!("timed out waiting for {label}\n{}", self.debug_state());
    }

    async fn wait_for_visible_snapshot(&self, expected: &DirSnapshot, label: &str) {
        let start = Instant::now();
        while start.elapsed() < WAIT_TIMEOUT {
            self.refresh_view(Client::Windows).await;
            self.refresh_view(Client::Ubuntu).await;
            let windows = visible_dir_snapshot(self.windows_work.path());
            let ubuntu = visible_dir_snapshot(self.ubuntu_work.path());
            if windows == *expected && ubuntu == *expected {
                return;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        self.refresh_view(Client::Windows).await;
        self.refresh_view(Client::Ubuntu).await;
        let windows = visible_dir_snapshot(self.windows_work.path());
        let ubuntu = visible_dir_snapshot(self.ubuntu_work.path());
        if windows == *expected && ubuntu == *expected {
            return;
        }
        panic!(
            "timed out waiting for {label}\nexpected visible: {expected:#?}\nwindows visible: {windows:#?}\nubuntu visible: {ubuntu:#?}\n{}",
            self.debug_state()
        );
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

    fn config_path(&self, client: Client) -> &Path {
        match client {
            Client::Windows => self.windows_cfg.path(),
            Client::Ubuntu => self.ubuntu_cfg.path(),
        }
    }

    fn gateway_port(&self, client: Client) -> u16 {
        match client {
            Client::Windows => self.windows_gateway_port,
            Client::Ubuntu => self.ubuntu_gateway_port,
        }
    }

    async fn webdav_request(
        &self,
        client: Client,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> String {
        http_request(
            SocketAddr::from((Ipv4Addr::LOCALHOST, self.gateway_port(client))),
            method,
            "127.0.0.1",
            &webdav_path(path),
            headers,
            body,
        )
        .await
    }

    async fn refresh_view(&self, client: Client) {
        let Some(snapshot) = config_visible_snapshot(self.config_path(client)).await else {
            return;
        };
        write_snapshot_to_dir(self.path(client), &snapshot);
    }

    fn stop_daemon(&mut self, client: Client) {
        let daemon = match client {
            Client::Windows => &mut self.windows_daemon,
            Client::Ubuntu => &mut self.ubuntu_daemon,
        };
        drop(daemon.take());
    }

    fn start_daemon(&mut self, client: Client) {
        let (slot, config_dir, log_path, gateway_port) = match client {
            Client::Windows => (
                &mut self.windows_daemon,
                self.windows_cfg.path(),
                self.windows_cfg.path().join("win.log"),
                self.windows_gateway_port,
            ),
            Client::Ubuntu => (
                &mut self.ubuntu_daemon,
                self.ubuntu_cfg.path(),
                self.ubuntu_cfg.path().join("ubuntu.log"),
                self.ubuntu_gateway_port,
            ),
        };
        assert!(slot.is_none(), "daemon is already running");
        *slot = Some(DaemonChild::spawn(
            config_dir,
            &self.relay.url,
            log_path,
            gateway_port,
        ));
    }

    fn import_source_dir(&self, client: Client) {
        let (config_dir, work_dir) = match client {
            Client::Windows => (self.windows_cfg.path(), self.windows_work.path()),
            Client::Ubuntu => (self.ubuntu_cfg.path(), self.ubuntu_work.path()),
        };
        run_json(config_dir, &["import", work_dir.to_str().unwrap()]);
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
    fn spawn(config_dir: &Path, relay_url: &str, log_path: PathBuf, gateway_port: u16) -> Self {
        let mut stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .unwrap();
        writeln!(stdout, "\n--- daemon start ---").unwrap();
        let stderr = stdout.try_clone().unwrap();
        let gateway_port = gateway_port.to_string();
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
                "--gateway-port",
                &gateway_port,
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

fn unused_loopback_port() -> u16 {
    std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

async fn config_visible_snapshot(config_dir: &Path) -> Option<DirSnapshot> {
    let daemon = iris_drive_core::Daemon::open(config_dir).ok()?;
    let visible = iris_drive_core::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .ok()?;
    let mut snapshot = BTreeMap::new();
    let mut stack = vec![(visible.root_cid, String::new())];
    while let Some((dir, prefix)) = stack.pop() {
        let mut entries = daemon.tree().list_directory(&dir).await.ok()?;
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        for entry in entries {
            if should_ignore_name(&entry.name) {
                continue;
            }
            let path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{prefix}/{}", entry.name)
            };
            let cid = Cid {
                hash: entry.hash,
                key: entry.key,
            };
            if entry.link_type == LinkType::Dir {
                stack.push((cid, path));
            } else {
                let bytes = daemon
                    .tree()
                    .read_file_range_cid(&cid, 0, None)
                    .await
                    .ok()??;
                snapshot.insert(
                    path,
                    FileSnapshot {
                        len: bytes.len() as u64,
                        sha256: to_hex(&sha256(&bytes)),
                        bytes,
                    },
                );
            }
        }
    }
    Some(snapshot)
}

fn write_snapshot_to_dir(root: &Path, snapshot: &DirSnapshot) {
    clear_dir(root);
    for (relative, file) in snapshot {
        let path = root.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, &file.bytes).unwrap();
    }
}

fn clear_dir(root: &Path) {
    let entries = std::fs::read_dir(root)
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for entry in entries {
        let path = entry.path();
        let file_type = entry.file_type().unwrap();
        if file_type.is_dir() {
            std::fs::remove_dir_all(path).unwrap();
        } else {
            std::fs::remove_file(path).unwrap();
        }
    }
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

fn snapshot_has_hashes_with_prefix(
    snapshot: &DirSnapshot,
    prefix: &str,
    expected_hashes: &[String],
) -> bool {
    let matching = snapshot
        .iter()
        .filter(|(path, _)| path.starts_with(prefix))
        .collect::<Vec<_>>();
    matching.len() >= expected_hashes.len()
        && expected_hashes
            .iter()
            .all(|hash| matching.iter().any(|(_, file)| &file.sha256 == hash))
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
                    bytes,
                },
            );
        }
    }
}

fn test_ignored_path(path: &str) -> bool {
    path.split('/').any(should_ignore_name)
}

async fn http_request(
    addr: SocketAddr,
    method: &str,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nContent-Length: {}\r\nConnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        request.push_str(name);
        request.push_str(": ");
        request.push_str(value);
        request.push_str("\r\n");
    }
    request.push_str("\r\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.write_all(body).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    String::from_utf8_lossy(&response).into_owned()
}

fn webdav_path(path: &str) -> String {
    let mut out = String::from("/dav");
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        out.push('/');
        out.push_str(&percent_encode_path_segment(segment));
    }
    out
}

fn percent_encode_path_segment(segment: &str) -> String {
    let mut encoded = String::new();
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(&mut encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

fn should_ignore_name(name: &str) -> bool {
    matches!(
        name,
        ".DS_Store" | ".hashtree" | ".Trash" | "$RECYCLE.BIN" | "Thumbs.db" | "desktop.ini"
    ) || name.starts_with("._")
        || name.starts_with(".Trash-")
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
