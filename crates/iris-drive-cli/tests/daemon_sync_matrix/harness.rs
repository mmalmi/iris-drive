#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Client {
    Windows,
    Ubuntu,
    MacOS,
}

impl Client {
    const THREE_VM: [Self; 3] = [Self::Windows, Self::Ubuntu, Self::MacOS];

    fn label(self) -> &'static str {
        match self {
            Self::Windows => "windows",
            Self::Ubuntu => "ubuntu",
            Self::MacOS => "macos",
        }
    }
}

struct SyncCluster {
    test_name: String,
    relay: LocalNostrRelay,
    _blossom: LocalBlossomServer,
    clients: Vec<Client>,
    windows_cfg: TempDir,
    ubuntu_cfg: TempDir,
    macos_cfg: Option<TempDir>,
    windows_work: TempDir,
    ubuntu_work: TempDir,
    macos_work: Option<TempDir>,
    windows_gateway_port: u16,
    ubuntu_gateway_port: u16,
    macos_gateway_port: Option<u16>,
    windows_daemon: Option<DaemonChild>,
    ubuntu_daemon: Option<DaemonChild>,
    macos_daemon: Option<DaemonChild>,
}

impl SyncCluster {
    async fn start(blossom_upload_delay: Duration) -> Self {
        Self::start_with_options(SyncClusterOptions {
            blossom_upload_delay,
            ..SyncClusterOptions::default()
        })
        .await
    }

    async fn start_three(blossom_upload_delay: Duration) -> Self {
        Self::start_with_options(SyncClusterOptions {
            blossom_upload_delay,
            clients: Client::THREE_VM.to_vec(),
            ..SyncClusterOptions::default()
        })
        .await
    }

    #[allow(clippy::too_many_lines)]
    async fn start_with_options(options: SyncClusterOptions) -> Self {
        let test_name = current_test_name();
        assert!(
            options.clients.contains(&Client::Windows) && options.clients.contains(&Client::Ubuntu),
            "live daemon matrix expects at least windows + ubuntu clients"
        );
        let include_macos = options.clients.contains(&Client::MacOS);
        let relay = LocalNostrRelay::spawn().await;
        let blossom =
            LocalBlossomServer::spawn_with_upload_delay(options.blossom_upload_delay).await;

        let windows_cfg = tempdir().unwrap();
        let ubuntu_cfg = tempdir().unwrap();
        let macos_cfg = include_macos.then(|| tempdir().unwrap());
        let windows_work = tempdir().unwrap();
        let ubuntu_work = tempdir().unwrap();
        let macos_work = include_macos.then(|| tempdir().unwrap());
        let windows_gateway_port = unused_loopback_port();
        let ubuntu_gateway_port = unused_loopback_port();
        let macos_gateway_port = include_macos.then(unused_loopback_port);

        configure_local_blossom(windows_cfg.path(), &blossom.url);
        configure_local_blossom(ubuntu_cfg.path(), &blossom.url);
        if let Some(config) = macos_cfg.as_ref() {
            configure_local_blossom(config.path(), &blossom.url);
        }

        let init = run_json(windows_cfg.path(), &["init", "--label", "windows-peer"]);
        let owner_npub = init["owner_npub"].as_str().unwrap();
        let linked = run_json(
            ubuntu_cfg.path(),
            &["link", owner_npub, "--label", "linux-peer"],
        );
        let request = linked["device_link_request"]["url"].as_str().unwrap();
        run_json(windows_cfg.path(), &["approve", request]);
        if let Some(config) = macos_cfg.as_ref() {
            let linked = run_json(
                config.path(),
                &["link", owner_npub, "--label", "macos-peer"],
            );
            let request = linked["device_link_request"]["url"].as_str().unwrap();
            run_json(windows_cfg.path(), &["approve", request]);
        }

        for seed in &options.seed_files {
            let root = match seed.client {
                Client::Windows => windows_work.path(),
                Client::Ubuntu => ubuntu_work.path(),
                Client::MacOS => macos_work
                    .as_ref()
                    .expect("macos seed requires macos client")
                    .path(),
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
        if let (Some(config), Some(work)) = (macos_cfg.as_ref(), macos_work.as_ref()) {
            run_json(config.path(), &["import", work.path().to_str().unwrap()]);
        }

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
        let macos_daemon =
            if let (Some(config), Some(gateway_port)) = (macos_cfg.as_ref(), macos_gateway_port) {
                Some(DaemonChild::spawn(
                    config.path(),
                    &relay.url,
                    config.path().join("macos.log"),
                    gateway_port,
                ))
            } else {
                None
            };

        Self {
            test_name,
            relay,
            _blossom: blossom,
            clients: options.clients,
            windows_cfg,
            ubuntu_cfg,
            macos_cfg,
            windows_work,
            ubuntu_work,
            macos_work,
            windows_gateway_port,
            ubuntu_gateway_port,
            macos_gateway_port,
            windows_daemon,
            ubuntu_daemon,
            macos_daemon,
        }
    }

    async fn wait_until_authorized(&self) {
        self.wait_until("linked peers authorized", || {
            self.clients()
                .into_iter()
                .filter(|client| *client != Client::Windows)
                .all(|client| {
                    let status = run_json(self.config_path(client), &["status"]);
                    status["account"]["authorization_state"] == "authorized"
                })
        })
        .await;
    }

    async fn wait_until_direct_peers_connected(&self) {
        self.wait_until("direct fips peers connected", || {
            self.clients().into_iter().all(|client| {
                let status = run_json(self.config_path(client), &["status"]);
                status["network"]["fips"]["connected_peer_count"]
                    .as_u64()
                    .unwrap_or(0)
                    > 0
            })
        })
        .await;
    }

    fn drop_relay_kinds(&self, kinds: &[u16]) {
        self.relay.drop_kinds(kinds);
    }

    async fn write(&self, client: Client, path: &str, bytes: &[u8]) {
        if test_ignored_path(path) {
            self.write_local_only(client, path, bytes).await;
        } else {
            self.provider_write(client, path, bytes).await;
        }
    }

    async fn write_local_only(&self, client: Client, path: &str, bytes: &[u8]) {
        let local_path = self.path(client).join(path);
        if let Some(parent) = local_path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        tokio::fs::write(local_path, bytes).await.unwrap();
    }

    async fn provider_write(&self, client: Client, path: &str, bytes: &[u8]) -> String {
        let source = self
            .config_path(client)
            .join(format!("provider-source-{}.bin", path_hash_label(path)));
        if let Some(parent) = source.parent() {
            tokio::fs::create_dir_all(parent).await.unwrap();
        }
        tokio::fs::write(&source, bytes).await.unwrap();
        let output = idrive(self.config_path(client))
            .args(["provider", "write", path])
            .arg(&source)
            .output()
            .unwrap();
        assert_command_success(
            &output,
            &format!("provider write {} {path}", client.label()),
        );
        let value = json_output(&output);
        self.refresh_view(client).await;
        value["root_cid"].as_str().unwrap().to_string()
    }

    async fn provider_rename(&self, client: Client, from: &str, to: &str) -> String {
        let output = idrive(self.config_path(client))
            .args(["provider", "rename", from, to])
            .output()
            .unwrap();
        assert_command_success(
            &output,
            &format!("provider rename {} {from} -> {to}", client.label()),
        );
        let value = json_output(&output);
        self.refresh_view(client).await;
        value["root_cid"].as_str().unwrap().to_string()
    }

    async fn provider_delete(&self, client: Client, path: &str) -> String {
        let output = idrive(self.config_path(client))
            .args(["provider", "delete", path])
            .output()
            .unwrap();
        assert_command_success(
            &output,
            &format!("provider delete {} {path}", client.label()),
        );
        let value = json_output(&output);
        self.refresh_view(client).await;
        value["root_cid"].as_str().unwrap().to_string()
    }

    async fn provider_mkdir(&self, client: Client, path: &str) -> String {
        let output = idrive(self.config_path(client))
            .args(["provider", "mkdir", path])
            .output()
            .unwrap();
        assert_command_success(
            &output,
            &format!("provider mkdir {} {path}", client.label()),
        );
        let value = json_output(&output);
        self.refresh_view(client).await;
        value["root_cid"].as_str().unwrap().to_string()
    }

    async fn rename(&self, client: Client, from: &str, to: &str) {
        if test_ignored_path(from) || test_ignored_path(to) {
            let root = self.path(client);
            let destination = root.join(to);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            tokio::fs::rename(root.join(from), destination)
                .await
                .unwrap();
        } else {
            self.provider_rename(client, from, to).await;
        }
    }

    async fn remove(&self, client: Client, path: &str) {
        if test_ignored_path(path) {
            tokio::fs::remove_file(self.path(client).join(path))
                .await
                .unwrap();
        } else {
            self.provider_delete(client, path).await;
        }
    }

    async fn remove_all(&self, client: Client, path: &str) {
        let relative = path.to_string();
        if test_ignored_path(&relative) {
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
        } else {
            self.provider_delete(client, &relative).await;
        }
    }

    async fn mkdir(&self, client: Client, path: &str) {
        if test_ignored_path(path) {
            tokio::fs::create_dir_all(self.path(client).join(path))
                .await
                .unwrap();
        } else {
            self.provider_mkdir(client, path).await;
        }
    }

    fn assert_file(&self, client: Client, path: &str, expected: &[u8]) {
        let actual = std::fs::read(self.path(client).join(path)).unwrap();
        assert_eq!(
            actual,
            expected,
            "{}\n{}",
            self.debug_state(),
            self.rerun_hint()
        );
    }

    fn assert_missing(&self, client: Client, path: &str) {
        assert!(
            !self.path(client).join(path).exists(),
            "{} should be absent\n{}",
            path,
            self.debug_state_with_rerun_hint()
        );
    }

    fn assert_status_counts(&self, client: Client, expected_files: u64, expected_devices: u64) {
        let status = run_json(self.config_path(client), &["status"]);
        assert_eq!(
            status["hashtree"]["file_count"].as_u64(),
            Some(expected_files),
            "{} status should report {expected_files} files\n{}",
            client.label(),
            self.debug_state_with_rerun_hint()
        );
        assert_eq!(
            status["network"]["authorized_device_count"].as_u64(),
            Some(expected_devices),
            "{} status should report {expected_devices} authorized devices\n{}",
            client.label(),
            self.debug_state_with_rerun_hint()
        );
    }

    fn provider_entry_kind(&self, client: Client, path: &str) -> Option<String> {
        let list = run_json(self.config_path(client), &["provider", "list"]);
        list["entries"].as_array()?.iter().find_map(|entry| {
            (entry["path"].as_str() == Some(path))
                .then(|| entry["kind"].as_str().unwrap_or_default().to_string())
        })
    }

    fn assert_provider_entry(&self, client: Client, path: &str, kind: &str) {
        assert_eq!(
            self.provider_entry_kind(client, path).as_deref(),
            Some(kind),
            "{} provider should contain {kind} {path}\n{}",
            client.label(),
            self.debug_state_with_rerun_hint()
        );
    }

    fn assert_provider_missing(&self, client: Client, path: &str) {
        assert!(
            self.provider_entry_kind(client, path).is_none(),
            "{} provider should not contain {path}\n{}",
            client.label(),
            self.debug_state_with_rerun_hint()
        );
    }

    async fn wait_for_convergence_from(&self, client: Client, label: &str) {
        let expected = visible_dir_snapshot(self.path(client));
        self.wait_for_visible_snapshot(&expected, label).await;
    }

    async fn wait_for_provider_entry(&self, path: &str, kind: &str, label: &str) {
        self.wait_until(label, || {
            self.clients()
                .into_iter()
                .all(|client| self.provider_entry_kind(client, path).as_deref() == Some(kind))
        })
        .await;
    }

    async fn wait_for_provider_missing(&self, path: &str, label: &str) {
        self.wait_until(label, || {
            self.clients()
                .into_iter()
                .all(|client| self.provider_entry_kind(client, path).is_none())
        })
        .await;
    }

    async fn wait_for_provider_publish(&self, client: Client, root_cid: &str, label: &str) {
        self.wait_until(label, || {
            let status = run_json(self.config_path(client), &["status"]);
            let daemon = &status["daemon"];
            daemon["event"] == "provider_root_publish_finished"
                && daemon["context"]["root_key"]
                    .as_str()
                    .is_some_and(|key| key.ends_with(root_cid))
                && daemon["publish"]["published_drive_root"]
                    .as_bool()
                    .unwrap_or(false)
        })
        .await;
    }

    async fn wait_for_snapshot(&self, expected: &DirSnapshot, label: &str) {
        let start = Instant::now();
        while start.elapsed() < WAIT_TIMEOUT {
            for client in self.clients() {
                self.refresh_view(client).await;
            }
            if self
                .clients()
                .into_iter()
                .all(|client| dir_snapshot(self.path(client)) == *expected)
            {
                return;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        panic!(
            "timed out waiting for {label}\n{}",
            self.debug_state_with_rerun_hint()
        );
    }

    async fn wait_for_visible_snapshot(&self, expected: &DirSnapshot, label: &str) {
        let start = Instant::now();
        while start.elapsed() < WAIT_TIMEOUT {
            for client in self.clients() {
                self.refresh_view(client).await;
            }
            if self
                .clients()
                .into_iter()
                .all(|client| visible_dir_snapshot(self.path(client)) == *expected)
            {
                return;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
        for client in self.clients() {
            self.refresh_view(client).await;
        }
        if self
            .clients()
            .into_iter()
            .all(|client| visible_dir_snapshot(self.path(client)) == *expected)
        {
            return;
        }
        panic!(
            "timed out waiting for {label}\nexpected visible: {expected:#?}\n{}",
            self.debug_state_with_rerun_hint()
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
        panic!(
            "timed out waiting for {label}\n{}",
            self.debug_state_with_rerun_hint()
        );
    }

    fn path(&self, client: Client) -> &Path {
        match client {
            Client::Windows => self.windows_work.path(),
            Client::Ubuntu => self.ubuntu_work.path(),
            Client::MacOS => self
                .macos_work
                .as_ref()
                .expect("macos client is not active")
                .path(),
        }
    }

    fn config_path(&self, client: Client) -> &Path {
        match client {
            Client::Windows => self.windows_cfg.path(),
            Client::Ubuntu => self.ubuntu_cfg.path(),
            Client::MacOS => self
                .macos_cfg
                .as_ref()
                .expect("macos client is not active")
                .path(),
        }
    }

    fn clients(&self) -> Vec<Client> {
        self.clients.clone()
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
            Client::MacOS => &mut self.macos_daemon,
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
            Client::MacOS => (
                &mut self.macos_daemon,
                self.macos_cfg
                    .as_ref()
                    .expect("macos client is not active")
                    .path(),
                self.macos_cfg
                    .as_ref()
                    .expect("macos client is not active")
                    .path()
                    .join("macos.log"),
                self.macos_gateway_port.expect("macos client is not active"),
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
            Client::MacOS => (
                self.macos_cfg
                    .as_ref()
                    .expect("macos client is not active")
                    .path(),
                self.macos_work
                    .as_ref()
                    .expect("macos client is not active")
                    .path(),
            ),
        };
        run_json(config_dir, &["import", work_dir.to_str().unwrap()]);
    }

    fn debug_state(&self) -> String {
        use std::fmt::Write as _;

        let mut out = String::new();
        for client in self.clients() {
            let _ = writeln!(
                out,
                "{}: {:#?}",
                client.label(),
                dir_snapshot(self.path(client))
            );
            let status =
                serde_json::to_string_pretty(&run_json(self.config_path(client), &["status"]))
                    .unwrap_or_default();
            let _ = writeln!(out, "{} status: {status}", client.label());
            let log = match client {
                Client::Windows => self
                    .windows_daemon
                    .as_ref()
                    .map_or_else(|| "<stopped>".to_string(), DaemonChild::log),
                Client::Ubuntu => self
                    .ubuntu_daemon
                    .as_ref()
                    .map_or_else(|| "<stopped>".to_string(), DaemonChild::log),
                Client::MacOS => self
                    .macos_daemon
                    .as_ref()
                    .map_or_else(|| "<stopped>".to_string(), DaemonChild::log),
            };
            let _ = writeln!(out, "{} log:\n{log}", client.label());
        }
        out
    }

    fn debug_state_with_rerun_hint(&self) -> String {
        format!("{}\n{}", self.debug_state(), self.rerun_hint())
    }

    fn rerun_hint(&self) -> String {
        rerun_hint(&self.test_name)
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
    json_output(&output)
}

fn json_output(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid json: {error}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn assert_success(output: &Output) {
    assert_command_success(output, "command");
}

fn assert_command_success(output: &Output, context: &str) {
    assert!(
        output.status.success(),
        "{context} failed\nstatus: {}\nstdout: {}\nstderr: {}",
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

fn path_hash_label(path: &str) -> String {
    to_hex(&sha256(path.as_bytes()))[..16].to_string()
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
