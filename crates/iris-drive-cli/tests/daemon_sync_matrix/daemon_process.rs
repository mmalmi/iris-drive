struct DaemonChild {
    child: Child,
    log_path: PathBuf,
}

#[derive(Clone, Copy)]
enum FipsTestCarrier {
    Udp { port: Option<u16> },
    WebSocket {
        bind_port: Option<u16>,
        seed_port: Option<u16>,
    },
}

impl DaemonChild {
    fn spawn(config_dir: &Path, relay_url: &str, log_path: PathBuf, gateway_port: u16) -> Self {
        Self::spawn_inner(
            config_dir,
            relay_url,
            log_path,
            gateway_port,
            FipsTestCarrier::Udp { port: None },
            "",
            None,
        )
    }

    fn spawn_with_fips_peers(
        config_dir: &Path,
        relay_url: &str,
        log_path: PathBuf,
        gateway_port: u16,
        fips_port: u16,
        static_peers: &str,
    ) -> Self {
        Self::spawn_inner(
            config_dir,
            relay_url,
            log_path,
            gateway_port,
            FipsTestCarrier::Udp {
                port: Some(fips_port),
            },
            static_peers,
            None,
        )
    }

    fn spawn_websocket_listener(
        config_dir: &Path,
        relay_url: &str,
        log_path: PathBuf,
        gateway_port: u16,
        websocket_port: u16,
        open_discovery_max_pending: usize,
    ) -> Self {
        Self::spawn_inner(
            config_dir,
            relay_url,
            log_path,
            gateway_port,
            FipsTestCarrier::WebSocket {
                bind_port: Some(websocket_port),
                seed_port: None,
            },
            "",
            Some(open_discovery_max_pending),
        )
    }

    fn spawn_websocket_client(
        config_dir: &Path,
        relay_url: &str,
        log_path: PathBuf,
        gateway_port: u16,
        websocket_seed_port: u16,
        open_discovery_max_pending: usize,
    ) -> Self {
        Self::spawn_inner(
            config_dir,
            relay_url,
            log_path,
            gateway_port,
            FipsTestCarrier::WebSocket {
                bind_port: None,
                seed_port: Some(websocket_seed_port),
            },
            "",
            Some(open_discovery_max_pending),
        )
    }

    fn spawn_inner(
        config_dir: &Path,
        relay_url: &str,
        log_path: PathBuf,
        gateway_port: u16,
        carrier: FipsTestCarrier,
        static_peers: &str,
        open_discovery_max_pending: Option<usize>,
    ) -> Self {
        let mut stdout = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .unwrap();
        writeln!(stdout, "\n--- daemon start ---").unwrap();
        let stderr = stdout.try_clone().unwrap();
        let gateway_port = gateway_port.to_string();
        let fips_port = match carrier {
            FipsTestCarrier::Udp { port } => port,
            FipsTestCarrier::WebSocket { .. } => None,
        };
        let (websocket_bind_addr, websocket_seed_urls) = match carrier {
            FipsTestCarrier::Udp { .. } => (String::new(), String::new()),
            FipsTestCarrier::WebSocket {
                bind_port,
                seed_port,
            } => (
                bind_port.map_or_else(String::new, |port| format!("127.0.0.1:{port}")),
                seed_port.map_or_else(String::new, |port| {
                    format!("ws://127.0.0.1:{port}/fips")
                }),
            ),
        };
        let fips_bind = fips_port.map_or_else(
            || "127.0.0.1:0".to_string(),
            |port| format!("127.0.0.1:{port}"),
        );
        let fips_external = fips_port.map_or_else(String::new, |port| format!("127.0.0.1:{port}"));
        let mut command = Command::new(idrive_bin());
        command
            .env("IRIS_DRIVE_CONFIG_DIR", config_dir)
            .env("IRIS_DRIVE_FIPS_ENABLE_BOOTSTRAP", "false")
            .env(
                "IRIS_DRIVE_FIPS_ENABLE_LOCAL_RENDEZVOUS",
                matches!(carrier, FipsTestCarrier::Udp { .. }).to_string(),
            )
            // Keep carrier-specific daemon tests deterministic now that
            // production defaults include public WebSocket entry points.
            .env("IRIS_FIPS_WEBSOCKET_BIND_ADDR", websocket_bind_addr)
            .env("IRIS_FIPS_WEBSOCKET_SEED_URLS", websocket_seed_urls)
            .env(
                "IRIS_DRIVE_FIPS_ENABLE_UDP",
                matches!(carrier, FipsTestCarrier::Udp { .. }).to_string(),
            )
            .env(
                "IRIS_DRIVE_FIPS_ENABLE_WEBRTC",
                "false",
            )
            .env(
                "IRIS_DRIVE_FIPS_ENABLE_LAN_DISCOVERY",
                matches!(carrier, FipsTestCarrier::Udp { .. }).to_string(),
            )
            .env("IRIS_DRIVE_FIPS_STATIC_PEERS", static_peers)
            .env(
                "IRIS_DRIVE_FIPS_OPEN_DISCOVERY_MAX_PENDING",
                open_discovery_max_pending.unwrap_or(0).to_string(),
            )
            .env_remove("IRIS_DRIVE_FIPS_LOCAL_RENDEZVOUS_ADDR")
            .env("IRIS_DRIVE_FIPS_UDP_BIND_ADDR", fips_bind)
            .env("IRIS_DRIVE_FIPS_UDP_EXTERNAL_ADDR", fips_external)
            .env("IRIS_DRIVE_FIPS_UDP_PUBLIC", "false");
        let child = command
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
