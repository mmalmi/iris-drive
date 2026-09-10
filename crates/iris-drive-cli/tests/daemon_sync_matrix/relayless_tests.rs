#[allow(clippy::wildcard_imports)]
use super::*;
use iris_drive_core::{AppConfig, Drive, Profile, paths::config_path_in};

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_daemons_sync_signed_roots_and_blobs_without_relays_or_seeds() {
    assert_relayless_daemons_sync(false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_daemons_sync_signed_roots_and_blobs_with_unavailable_shared_store() {
    assert_relayless_daemons_sync(true).await;
}

#[allow(clippy::too_many_lines)]
async fn assert_relayless_daemons_sync(shared_store_unavailable: bool) {
    let _guard = live_daemon_test_guard().await;
    let owner_dir = tempdir().unwrap();
    let linked_dir = tempdir().unwrap();
    let mut owner = Profile::create(owner_dir.path(), Some("owner".into())).unwrap();
    let mut linked = Profile::link_to_profile(
        linked_dir.path(),
        owner.state.profile_id,
        owner.state.app_key_pubkey.clone(),
        Some("linked".into()),
    )
    .unwrap();
    owner
        .approve_app_key(&linked.state.app_key_pubkey, Some("linked".into()))
        .unwrap();
    linked
        .state
        .profile_roster_ops
        .clone_from(&owner.state.profile_roster_ops);
    let linked = Profile::load(linked.state, linked_dir.path()).unwrap();
    assert!(owner.state.is_authorized() && linked.state.is_authorized());
    assert_ne!(owner.state.app_key_pubkey, linked.state.app_key_pubkey);
    assert_eq!(owner.current_dck().unwrap(), linked.current_dck().unwrap());
    let dirs = [owner_dir.path(), linked_dir.path()];
    for (dir, profile) in dirs.into_iter().zip([&owner, &linked]) {
        let mut config = AppConfig {
            profile: Some(profile.state.clone()),
            relays: Vec::new(),
            blossom_servers: Vec::new(),
            ..AppConfig::default()
        };
        config.upsert_drive(Drive::primary(profile.state.root_scope_id()));
        config.save(config_path_in(dir)).unwrap();
    }
    let ports = [unused_udp_loopback_port(), unused_udp_loopback_port()];
    let peers = [
        owner.app_key.pubkey_bech32(),
        linked.app_key.pubkey_bech32(),
    ];
    let mut daemons = Vec::new();
    for (i, dir) in dirs.iter().enumerate() {
        if shared_store_unavailable {
            // The optional read route cannot use a file as its data directory.
            // Drive's own store and the isolated Hashtree config remain valid.
            std::fs::write(dir.join("shared-hashtree"), b"unavailable shared store").unwrap();
        }
        daemons.push(DaemonChild::spawn_with_fips_peers(
            dir,
            "",
            dir.join("daemon.log"),
            unused_loopback_port(),
            ports[i],
            &format!("{}=127.0.0.1:{}", peers[1 - i], ports[1 - i]),
        ));
    }
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        for daemon in &mut daemons {
            assert!(
                daemon.child.try_wait().unwrap().is_none(),
                "{}",
                daemon.log()
            );
        }
        if dirs.iter().all(|dir| {
            let status = run_json(dir, &["status"]);
            status["network"]["fips"]["roster_connected_peer_count"].as_u64() == Some(1)
        }) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "{}\n{}",
            daemons[0].log(),
            daemons[1].log()
        );
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    for dir in dirs {
        let settings: Value = serde_json::from_slice(
            &std::fs::read(dir.join("Hashtree/config/browser_settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(settings["nostrRelays"], serde_json::json!([]));
        assert_eq!(settings["blossomReadServers"], serde_json::json!([]));
    }
    for (index, name) in ["from-owner.txt", "from-linked.txt"]
        .into_iter()
        .enumerate()
    {
        let bytes = format!("authenticated root and blob from device {index}").into_bytes();
        let source = dirs[index].join("source.bin");
        std::fs::write(&source, &bytes).unwrap();
        let mut command = idrive(dirs[index]);
        command
            .env("IRIS_FIPS_WEBSOCKET_SEED_URLS", "")
            .args(["provider", "write", name])
            .arg(&source);
        let output = run_command(&mut command, "relayless provider write");
        assert_command_success(&output, "relayless provider write");
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if config_visible_snapshot(dirs[1 - index])
                .await
                .is_some_and(|snapshot| snapshot.get(name).is_some_and(|file| file.bytes == bytes))
            {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "{}\n{}",
                daemons[0].log(),
                daemons[1].log()
            );
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
    if shared_store_unavailable {
        for (dir, daemon) in dirs.into_iter().zip(&daemons) {
            let log = daemon.log();
            assert!(
                log.contains("continuing without optional shared blob route"),
                "{log}"
            );
            assert!(log.contains("shared Hashtree LMDB:"), "{log}");
            assert_eq!(
                std::fs::read(dir.join("shared-hashtree")).unwrap(),
                b"unavailable shared store"
            );
        }
        return;
    }
    #[cfg(unix)]
    for dir in dirs {
        let sampler = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/idle-cpu-gate.sh");
        let mut command = Command::new("bash");
        command
            .arg(sampler)
            .args(["--platform", "auto"])
            .env("IRIS_DRIVE_IDLE_CPU_REQUIRED_ROLES", "daemon")
            .env("IRIS_DRIVE_IDLE_CPU_COMMAND_MATCH", dir)
            .env("IRIS_DRIVE_IDLE_CPU_WARMUP_SECS", "2")
            .env("IRIS_DRIVE_IDLE_CPU_DURATION_SECS", "15")
            .env("IRIS_DRIVE_IDLE_CPU_INTERVAL_SECS", "1")
            .env("IRIS_DRIVE_IDLE_CPU_DAEMON_MAX", "10");
        let output = run_command(&mut command, "relayless daemon idle CPU");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
    }
}
