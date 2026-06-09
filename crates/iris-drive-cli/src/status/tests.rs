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
    assert_eq!(
        status["portal_url"],
        iris_drive_core::gateway::local_portal_url(17_321)
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
        .expect("default file server should be visible in backup targets");

    assert_eq!(target["enabled"], true);
    assert!(target["label"].is_null());
    assert_eq!(target["title"], "upload.iris.to");
}

#[test]
fn backup_target_status_emits_shared_row_fields() {
    let status = backup_target_status(&BackupTarget {
        id: "backup-1".to_owned(),
        kind: BackupTargetKind::Blossom,
        target: "https://backup.example".to_owned(),
        label: Some("Archive".to_owned()),
        enabled: true,
        last_sync: Some(BackupTargetSync {
            state: "uploading".to_owned(),
            root_cid: "root".to_owned(),
            synced_at: 1_700_000_000,
            total_hashes: 5,
            uploaded: 2,
            already_present: 1,
        }),
        last_check: Some(BackupTargetCheck {
            state: "verified".to_owned(),
            root_cid: "root".to_owned(),
            checked_at: 1_700_000_100,
            total_hashes: 5,
            sample_size: 5,
            sampled_hashes: 5,
            present: 5,
            missing: 0,
            unknown: 0,
            latency_ms: Some(35),
            download_bytes: Some(2048),
            download_ms: Some(1000),
            download_bytes_per_second: Some(2048),
            error: None,
        }),
    });

    assert_eq!(status["title"], "Archive");
    assert_eq!(status["state"], "uploading");
    assert_eq!(
        status["detail"],
        "https://backup.example | 2/5 | check verified | 35 ms | 2.0 KB/s"
    );

    let fips_status = backup_target_status(&BackupTarget {
        id: "fips-1".to_owned(),
        kind: BackupTargetKind::Fips,
        target: "abcdefghijklmnopqrstuvwxyz0123456789".to_owned(),
        label: None,
        enabled: true,
        last_sync: None,
        last_check: None,
    });

    assert_eq!(fips_status["title"], "abcdefghijklmn...0123456789");
    assert_eq!(fips_status["state"], "pending");
    assert_eq!(fips_status["detail"], "abcdefghijklmn...0123456789");
}

#[test]
fn network_status_merges_configured_relays_with_daemon_relay_statuses() {
    let config = AppConfig {
        relays: vec![
            "wss://relay.example/".to_owned(),
            "wss://relay.two".to_owned(),
        ],
        ..AppConfig::default()
    };
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
                "url": "wss://relay.example",
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
fn status_summary_emits_shared_setup_and_count_fields() {
    let summary = status_summary(
        true,
        Some(&json!({"authorization_state": "authorized"})),
        2,
        1,
        Some(3),
        Some(42),
        "up to date",
        "current:root-cid",
    );

    assert_eq!(summary["setup_state"], "authorized");
    assert_eq!(summary["setup_complete"], true);
    assert_eq!(summary["awaiting_approval"], false);
    assert_eq!(summary["revoked"], false);
    assert_eq!(summary["setup_label"], "Linked");
    assert_eq!(summary["primary_status"], "ready");
    assert_eq!(summary["primary_status_label"], "Ready");
    assert_eq!(summary["authorized_app_key_count"], 2);
    assert_eq!(summary["online_app_key_count"], 1);
    assert!(summary.get("authorized_device_count").is_none());
    assert!(summary.get("online_device_count").is_none());
    assert_eq!(summary["file_count"], 3);
    assert_eq!(summary["visible_file_bytes"], 42);
    assert_eq!(summary["sync_status"], "up to date");
    assert_eq!(summary["sync_status_label"], "Up to date");
    assert_eq!(summary["provider_refresh_key"], "current:root-cid");

    let unconfigured = status_summary(false, None, 0, 0, None, None, "paused", "");
    assert_eq!(unconfigured["setup_state"], "not_configured");
    assert_eq!(unconfigured["setup_complete"], false);
    assert_eq!(unconfigured["awaiting_approval"], false);
    assert_eq!(unconfigured["revoked"], false);
    assert_eq!(unconfigured["setup_label"], "Not linked");
    assert_eq!(unconfigured["primary_status"], "not_setup");
}

#[test]
fn daemon_sync_status_is_normalized_for_clients() {
    assert_eq!(daemon_sync_status(None), "paused");
    assert_eq!(
        daemon_sync_status(Some(&json!({"running": false, "event": "subscribed"}))),
        "paused"
    );
    assert_eq!(
        daemon_sync_status(Some(&json!({"running": true, "event": "subscribed"}))),
        "up to date"
    );
    assert_eq!(
        daemon_sync_status(Some(&json!({
            "running": true,
            "blossom_upload": {
                "uploaded": 1,
                "already_present": 1,
                "total_hashes": 3
            }
        }))),
        "syncing"
    );
    assert_eq!(
        daemon_sync_status(Some(&json!({"running": true, "event": "apply_error"}))),
        "sync error"
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
    let mut owner = Profile::create(dir.path(), Some("Mac".into())).unwrap();
    let linked_device = nostr_sdk::Keys::generate().public_key().to_hex();
    owner
        .approve_app_key(&linked_device, Some("Phone".into()))
        .unwrap();
    let mut config = AppConfig {
        profile: Some(owner.state.clone()),
        ..AppConfig::default()
    };
    config.upsert_drive(Drive::primary(owner.state.root_scope_id()));
    config.save(config_path_in(dir.path())).unwrap();

    let linked_npub = pubkey_npub(&linked_device);
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
        .find(|peer| peer["is_current_app_key"] == true)
        .expect("current AppKey peer");
    assert!(current.get("is_current_device").is_none());
    assert_eq!(current["display_label"], "This Device");
    assert_eq!(current["role_label"], "Admin");
    assert_eq!(current["connection_state"], "local");
    assert_eq!(current["connection_label"], "This Device");
    assert_eq!(current["detail"], "This Device | Admin | not imported");
    assert_eq!(current["can_revoke"], false);
    assert_eq!(current["can_appoint_admin"], false);
    assert_eq!(current["can_demote_admin"], false);

    let linked = peers
        .iter()
        .find(|peer| peer["app_key_npub"] == linked_npub)
        .expect("linked AppKey peer");
    assert!(linked.get("device_npub").is_none());
    assert_eq!(linked["display_label"], "Phone");
    assert_eq!(linked["role_label"], "Member");
    assert_eq!(linked["connection_state"], "direct");
    assert_eq!(linked["connection_label"], "Online (TCP, 17 ms)");
    assert_eq!(linked["detail"], "Member | waiting for root");
    assert_eq!(linked["can_revoke"], true);
    assert_eq!(linked["can_appoint_admin"], true);
    assert_eq!(linked["can_demote_admin"], false);
}

#[test]
fn daemon_status_writer_persists_normalized_relay_and_fips_statuses() {
    let dir = tempfile::tempdir().unwrap();
    let config = AppConfig {
        relays: vec![
            "wss://relay.example/".to_owned(),
            "wss://relay.two".to_owned(),
        ],
        ..AppConfig::default()
    };
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
                "url": "wss://relay.example",
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
fn daemon_status_writer_persists_normalized_summary_for_clients() {
    let dir = tempfile::tempdir().unwrap();
    let mut owner = Profile::create(dir.path(), Some("Mac".into())).unwrap();
    let linked_device = nostr_sdk::Keys::generate().public_key().to_hex();
    owner
        .approve_app_key(&linked_device, Some("Phone".into()))
        .unwrap();
    let config = AppConfig {
        profile: Some(owner.state.clone()),
        ..AppConfig::default()
    };
    config.save(config_path_in(dir.path())).unwrap();

    let linked_npub = pubkey_npub(&linked_device);
    write_daemon_status(
        dir.path(),
        json!({
            "event": "relay_statuses",
            "fips_block_sync": {
                "connected_peers": [linked_npub],
            }
        }),
    );

    let status: Value =
        serde_json::from_str(&std::fs::read_to_string(daemon_status_path(dir.path())).unwrap())
            .unwrap();

    assert_eq!(status["summary"]["setup_state"], "authorized");
    assert_eq!(status["summary"]["setup_complete"], true);
    assert_eq!(status["summary"]["setup_label"], "Linked");
    assert_eq!(status["summary"]["primary_status"], "ready");
    assert_eq!(status["summary"]["primary_status_label"], "Ready");
    assert_eq!(status["summary"]["authorized_app_key_count"], 2);
    assert_eq!(status["summary"]["online_app_key_count"], 2);
    assert_eq!(status["summary"]["sync_status"], "up to date");
    assert_eq!(status["summary"]["sync_status_label"], "Up to date");
}

#[test]
fn daemon_status_summary_does_not_walk_roots_inside_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let owner = Profile::create(dir.path(), Some("Mac".into())).unwrap();
    let root_cid = Cid::encrypted([0x11; 32], [0x22; 32]).to_string();
    let mut drive = Drive::primary(owner.state.root_scope_id());
    drive.app_key_roots.insert(
        owner.state.app_key_pubkey.clone(),
        AppKeyRootRef::legacy(&root_cid, 10, 1),
    );
    let config = AppConfig {
        profile: Some(owner.state.clone()),
        drives: vec![drive],
        ..AppConfig::default()
    };
    config.save(config_path_in(dir.path())).unwrap();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        write_daemon_status(dir.path(), json!({"event": "relay_statuses"}));
    });

    let status: Value =
        serde_json::from_str(&std::fs::read_to_string(daemon_status_path(dir.path())).unwrap())
            .unwrap();
    assert_eq!(status["summary"]["authorized_app_key_count"], 1);
    assert_eq!(
        status["summary"]["provider_refresh_key"],
        format!("current:{root_cid}")
    );
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
