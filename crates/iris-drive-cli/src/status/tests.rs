use super::*;
use std::io::{Error, ErrorKind};

#[test]
fn retry_interrupted_io_retries_until_success() {
    let mut attempts = 0;

    let value = retry_interrupted_io(|| {
        attempts += 1;
        if attempts < 3 {
            Err(Error::from(ErrorKind::Interrupted))
        } else {
            Ok(42)
        }
    })
    .unwrap();

    assert_eq!(value, 42);
    assert_eq!(attempts, 3);
}

#[test]
fn retry_interrupted_io_returns_non_interrupted_errors() {
    let error = retry_interrupted_io(|| -> std::io::Result<()> {
        Err(Error::from(ErrorKind::PermissionDenied))
    })
    .unwrap_err();

    assert_eq!(error.kind(), ErrorKind::PermissionDenied);
}

#[test]
fn block_stats_entry_limit_marks_truncated() {
    let dir = tempfile::tempdir().unwrap();
    for index in 0..3 {
        std::fs::write(dir.path().join(format!("block-{index}")), b"block").unwrap();
    }

    let stats = collect_file_stats_with_entry_limit(dir.path(), Some(2)).unwrap();

    assert!(stats.truncated);
    assert_eq!(stats.file_count, 2);
    assert_eq!(stats.total_bytes, 10);
}

#[test]
fn local_gateway_status_includes_nhash_resolver_host_when_enabled() {
    let status = local_gateway_urls_for_root(None, 17_321, true);
    assert_eq!(status["enabled"], true);
    assert_eq!(
        status["nhash_resolver_url"],
        "http://nhash.iris.localhost:17321/"
    );
}

#[test]
fn local_gateway_status_reports_disabled_resolver() {
    let status = local_gateway_urls_for_root(None, 17_321, false);
    assert_eq!(status["enabled"], false);
    assert_eq!(status["host"], "nhash.iris.localhost");
    assert!(status.get("portal_url").is_none());
}

#[test]
fn status_lists_default_blossom_server_as_backup_target() {
    let config = AppConfig::default();
    let targets = backup_targets_status(&config);

    let target = targets
        .iter()
        .find(|target| target["kind"] == "blossom" && target["target"] == "https://upload.iris.to")
        .expect("default Blossom server should be visible in backup targets");

    assert_eq!(target["enabled"], true);
    assert_eq!(target["label"], "Blossom remote");
}

#[test]
fn network_status_merges_configured_relays_with_daemon_relay_statuses() {
    let mut config = AppConfig::default();
    config.relays = vec![
        "wss://relay.example/".to_owned(),
        "wss://relay.two".to_owned(),
    ];
    let daemon_status = json!({
        "relay_statuses": [
            {"url": "wss://relay.example", "status": "connected"},
            {"url": "wss://unconfigured.example", "status": "connected"}
        ]
    });

    let statuses = normalized_relay_statuses(&config, Some(&daemon_status));

    assert_eq!(
        statuses,
        json!([
            {
                "url": "wss://relay.example/",
                "status": "connected",
                "status_label": "connected",
                "health": "online",
            },
            {
                "url": "wss://relay.two",
                "status": "configured",
                "status_label": "saved",
                "health": "configured",
            }
        ])
    );
}

#[test]
fn fips_diagnostics_emit_normalized_device_counts_and_sets() {
    let config = AppConfig::default();
    let daemon_status = json!({
        "running": true,
        "fresh": true,
        "fips_block_sync": {
            "authorized_peers": ["npub1b", "npub1c"],
            "connected_peers": ["npub1b"],
            "mesh_peers": ["npub1c", "npub1x"],
            "peer_statuses": [{
                "npub": "npub1b",
                "transport_type": "tcp",
                "srtt_ms": 12
            }]
        }
    });

    let fips = fips_network_diagnostics(&config, Some(&daemon_status));

    assert_eq!(fips["state"], "running");
    assert_eq!(fips["state_label"], "Running");
    assert_eq!(fips["roster_label"], "2/2 online");
    assert_eq!(fips["direct_devices"], json!(["npub1b"]));
    assert_eq!(fips["mesh_devices"], json!(["npub1c", "npub1x"]));
    assert_eq!(
        fips["online_devices"],
        json!(["npub1b", "npub1c", "npub1x"])
    );
    assert_eq!(fips["roster_online_device_count"], 2);
    assert_eq!(fips["other_peer_count"], 1);
    assert_eq!(fips["peer_statuses"][0]["connection_label"], "TCP, 12 ms");
}

#[test]
fn peer_statuses_emit_rust_owned_labels_and_connection_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = Account::create(dir.path(), Some("Mac".into())).unwrap();
    let linked_device = nostr_sdk::Keys::generate().public_key().to_hex();
    owner
        .approve_device(&linked_device, Some("Phone".into()))
        .unwrap();
    let mut config = AppConfig {
        account: Some(owner.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(&owner.state.owner_pubkey));
    config.save(config_path_in(dir.path())).unwrap();

    let linked_npub = account_npub(&linked_device);
    let daemon_status = json!({
        "running": true,
        "fresh": true,
        "fips_block_sync": {
            "connected_peers": [linked_npub],
            "peer_statuses": [{
                "npub": linked_npub,
                "transport_type": "tcp",
                "srtt_ms": 17
            }]
        }
    });

    let peers = peer_statuses(dir.path(), &config, Some(&daemon_status));
    let current = peers
        .iter()
        .find(|peer| peer["is_current_device"] == true)
        .expect("current device peer");
    assert_eq!(current["display_label"], "This device");
    assert_eq!(current["role_label"], "Admin");
    assert_eq!(current["connection_state"], "local");
    assert_eq!(current["connection_label"], "This device");

    let linked = peers
        .iter()
        .find(|peer| peer["device_npub"] == linked_npub)
        .expect("linked device peer");
    assert_eq!(linked["display_label"], "Phone");
    assert_eq!(linked["role_label"], "Member");
    assert_eq!(linked["connection_state"], "direct");
    assert_eq!(linked["connection_label"], "Online (TCP, 17 ms)");
}

#[test]
fn daemon_status_writer_persists_normalized_relay_and_fips_statuses() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = AppConfig::default();
    config.relays = vec![
        "wss://relay.example/".to_owned(),
        "wss://relay.two".to_owned(),
    ];
    config.save(config_path_in(dir.path())).unwrap();

    write_daemon_status(
        dir.path(),
        json!({
            "relay_statuses": [
                {"url": "wss://relay.example", "status": "connected"},
                {"url": "wss://unconfigured.example", "status": "connected"}
            ],
            "fips_block_sync": {
                "authorized_peers": ["npub1b", "npub1c"],
                "connected_peers": ["npub1b"],
                "mesh_peers": ["npub1c", "npub1x"]
            }
        }),
    );

    let status: Value =
        serde_json::from_str(&std::fs::read_to_string(daemon_status_path(dir.path())).unwrap())
            .unwrap();

    assert_eq!(
        status["relay_statuses"],
        json!([
            {
                "url": "wss://relay.example/",
                "status": "connected",
                "status_label": "connected",
                "health": "online",
            },
            {
                "url": "wss://relay.two",
                "status": "configured",
                "status_label": "saved",
                "health": "configured",
            }
        ])
    );
    assert_eq!(status["fips"]["direct_devices"], json!(["npub1b"]));
    assert_eq!(
        status["fips"]["online_devices"],
        json!(["npub1b", "npub1c", "npub1x"])
    );
    assert_eq!(status["fips"]["roster_online_device_count"], 2);
    assert_eq!(status["fips"]["other_peer_count"], 1);
}

#[test]
fn daemon_status_writer_prefers_runtime_relays_for_top_level_status() {
    let dir = tempfile::tempdir().unwrap();
    AppConfig::default()
        .save(config_path_in(dir.path()))
        .unwrap();

    write_daemon_status(
        dir.path(),
        json!({
            "relays": ["ws://127.0.0.1:7000"],
            "relay_statuses": [
                {"url": "ws://127.0.0.1:7000/", "status": "connected"}
            ],
        }),
    );

    let status: Value =
        serde_json::from_str(&std::fs::read_to_string(daemon_status_path(dir.path())).unwrap())
            .unwrap();

    assert_eq!(
        status["relay_statuses"],
        json!([
            {
                "url": "ws://127.0.0.1:7000",
                "status": "connected",
                "status_label": "connected",
                "health": "online",
            }
        ])
    );
}
