use super::*;
use hashtree_core::Store;
use iris_drive_core::relay_sync::{DriveRootApply, apply_remote_drive_root_event};
use nostr_sdk::{Event, JsonUtil};

#[test]
fn reconnect_recovery_survives_an_intervening_announcement_refresh() {
    let mut exchange = DirectRootExchange::default();
    let peers = ["authorized-peer".to_string()];
    exchange.refresh_known_root_peers(peers.clone(), peers.clone());
    exchange.record_state_request_send(DirectRootAppSendStats {
        selected_peers: 1,
        sent_peers: 1,
        failed_peers: 0,
    });
    exchange.refresh_known_root_peers(peers.clone(), []);
    // Announcing our own root or answering another request observes the
    // reconnect before the daemon's next peer-refresh tick.
    exchange.refresh_known_root_peers(peers.clone(), peers.clone());
    let later_tick = exchange.refresh_known_root_peers(peers.clone(), peers);
    assert!(!later_tick.has_new_publish_peer);
    assert!(
        exchange.peer_state_request_pending(),
        "an ordinary announcement consumed the reconnect's state-repair work"
    );
}

#[test]
fn peer_recovery_retains_unsent_work_and_stops_after_success() {
    let mut exchange = DirectRootExchange::default();
    let peers = ["peer-a".to_string(), "peer-b".to_string()];
    exchange.refresh_known_root_peers(peers.clone(), peers.clone());
    let now = std::time::Instant::now();
    assert!(exchange.should_publish_state_request("scope", ["peer-a", "peer-b"], now));
    assert!(!exchange.should_publish_state_request("scope", ["peer-a", "peer-b"], now));
    assert!(
        exchange.peer_state_request_pending(),
        "throttling is not delivery"
    );
    for stats in [
        DirectRootAppSendStats {
            selected_peers: 0,
            sent_peers: 0,
            failed_peers: 0,
        },
        DirectRootAppSendStats {
            selected_peers: 2,
            sent_peers: 0,
            failed_peers: 2,
        },
        DirectRootAppSendStats {
            selected_peers: 2,
            sent_peers: 1,
            failed_peers: 1,
        },
    ] {
        exchange.record_state_request_send(stats);
        exchange.refresh_known_root_peers(peers.clone(), peers.clone());
        assert!(
            exchange.peer_state_request_pending(),
            "unsent repair was forgotten"
        );
    }
    exchange.record_state_request_send(DirectRootAppSendStats {
        selected_peers: 2,
        sent_peers: 2,
        failed_peers: 0,
    });
    exchange.refresh_known_root_peers(peers.clone(), peers);
    assert!(
        !exchange.peer_state_request_pending(),
        "unchanged idle ticks must stay quiet"
    );

    exchange.refresh_known_root_peers(["revoked-peer".to_string()], []);
    assert!(exchange.peer_state_request_pending());
    exchange.refresh_known_root_peers([], []);
    assert!(
        !exchange.peer_state_request_pending(),
        "revoked identities must be pruned"
    );
}

async fn copy_root_blocks(source: &Daemon, destination: &Daemon, root: &Cid) -> DownloadReport {
    let hashes = iris_drive_core::block_sync::collect_live_sync_hashes(source.tree(), root, 4)
        .await
        .unwrap();
    for hash in &hashes {
        let bytes = source.tree().get_store().get(hash).await.unwrap().unwrap();
        assert_eq!(hashtree_core::sha256(&bytes), *hash);
        destination
            .tree()
            .get_store()
            .put(*hash, bytes)
            .await
            .unwrap();
    }
    assert_eq!(
        iris_drive_core::block_sync::collect_live_sync_hashes(destination.tree(), root, 4)
            .await
            .unwrap(),
        hashes
    );
    DownloadReport {
        total_hashes: hashes.len(),
        fetched: hashes.len(),
        already_local: 0,
    }
}

async fn read_root_file(daemon: &Daemon, root: &Cid, path: &str) -> Vec<u8> {
    let cid = daemon.tree().resolve(root, path).await.unwrap().unwrap();
    daemon.tree().get(&cid, None).await.unwrap().unwrap()
}

#[tokio::test]
async fn complete_old_root_recovers_a_lost_update_through_fresh_signed_reply() {
    let owner_dir = tempfile::tempdir().unwrap();
    let receiver_dir = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut owner = Profile::create(owner_dir.path(), Some("owner".into())).unwrap();
    let receiver = Profile::link_to_profile(
        receiver_dir.path(),
        owner.state.profile_id,
        owner.state.app_key_pubkey.clone(),
        Some("receiver".into()),
    )
    .unwrap();
    owner
        .approve_app_key(&receiver.state.app_key_pubkey, Some("receiver".into()))
        .unwrap();
    let mut owner_config = AppConfig {
        profile: Some(owner.state.clone()),
        ..AppConfig::default()
    };
    owner_config.upsert_drive(Drive::primary(owner.state.root_scope_id()));
    owner_config.save(config_path_in(owner_dir.path())).unwrap();
    let mut receiver_state = receiver.state.clone();
    receiver_state.app_keys = owner.state.app_keys.clone();
    let mut receiver_config = AppConfig {
        profile: Some(receiver_state),
        ..AppConfig::default()
    };
    receiver_config.upsert_drive(Drive::primary(owner.state.root_scope_id()));
    receiver_config
        .save(config_path_in(receiver_dir.path()))
        .unwrap();
    let mut source = Daemon::open(owner_dir.path()).unwrap();
    let destination = Daemon::open(receiver_dir.path()).unwrap();

    std::fs::write(work.path().join("old.txt"), b"still usable").unwrap();
    source.import_source_dir(work.path()).await.unwrap();
    let mut sender = DirectRootExchange::default();
    let scope = owner.state.root_scope_id();
    let initial = sender
        .state_request_current_sync_events(owner_dir.path(), &scope)
        .await
        .unwrap();
    let old_event = initial
        .into_iter()
        .find(|e| e.key.starts_with("drive-root:"))
        .unwrap();
    let signed_old = Event::from_json(&old_event.json).unwrap();
    signed_old.verify().unwrap();
    assert_eq!(
        apply_remote_drive_root_event(
            &mut receiver_config,
            &signed_old,
            Some(receiver.app_key.keys())
        )
        .unwrap(),
        DriveRootApply::Applied
    );
    let old_ref = receiver_config
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots[&owner.state.app_key_pubkey]
        .clone();
    let old_root = Cid::parse(&old_ref.root_cid).unwrap();
    let report = copy_root_blocks(&source, &destination, &old_root).await;
    record_block_sync(receiver_dir.path(), &old_ref.root_cid, "fixture", &report);
    assert_eq!(
        remote_root_blocks_pending_count(receiver_dir.path(), &receiver_config),
        0
    );
    assert_eq!(
        read_root_file(&destination, &old_root, "old.txt").await,
        b"still usable"
    );

    // The receiver remains alive and retains the complete old root. The new
    // announcement is deliberately not delivered to its apply path.
    std::fs::write(work.path().join("new.txt"), b"missed announcement").unwrap();
    source.import_source_dir(work.path()).await.unwrap();
    assert!(
        destination
            .tree()
            .resolve(&old_root, "new.txt")
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        periodic_direct_root_repair_trigger(
            0,
            remote_root_blocks_pending_count(receiver_dir.path(), &receiver_config)
        ),
        "periodic_reconcile",
        "complete known roots must not suppress the existing periodic reconciliation"
    );

    let request_bytes = iris_drive_core::encode_direct_root_state_request_frame(&scope).unwrap();
    let DirectRootWireFrame::Request(request) =
        iris_drive_core::decode_direct_root_wire_frame(&request_bytes).unwrap()
    else {
        panic!("expected the production state-request wire frame");
    };
    let reply = sender
        .state_request_current_sync_events(owner_dir.path(), &request.root_scope_id)
        .await
        .unwrap()
        .into_iter()
        .find(|e| e.key.starts_with("drive-root:"))
        .unwrap();
    let signed_new = Event::from_json(&reply.json).unwrap();
    signed_new.verify().unwrap();
    assert_ne!(signed_new.id, signed_old.id);
    assert_eq!(
        apply_remote_drive_root_event(
            &mut receiver_config,
            &signed_new,
            Some(receiver.app_key.keys())
        )
        .unwrap(),
        DriveRootApply::Applied
    );
    let new_ref = receiver_config
        .drive(PRIMARY_DRIVE_ID)
        .unwrap()
        .app_key_roots[&owner.state.app_key_pubkey]
        .clone();
    assert!(new_ref.app_key_seq > old_ref.app_key_seq);
    let new_root = Cid::parse(&new_ref.root_cid).unwrap();
    copy_root_blocks(&source, &destination, &new_root).await;
    assert_eq!(
        read_root_file(&destination, &new_root, "new.txt").await,
        b"missed announcement"
    );
    assert_eq!(
        read_root_file(&destination, &old_root, "old.txt").await,
        b"still usable"
    );
    assert_eq!(
        apply_remote_drive_root_event(
            &mut receiver_config,
            &signed_old,
            Some(receiver.app_key.keys())
        )
        .unwrap(),
        DriveRootApply::StaleTimestamp
    );
}
