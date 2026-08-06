use super::*;

fn snapshot(peer: &str) -> FipsPeerConfigSnapshot {
    let peer = FipsPeerConfig {
        npub: peer.to_string(),
        udp_addresses: Vec::new(),
    };
    FipsPeerConfigSnapshot {
        application: vec![peer.clone()],
        routing: Vec::new(),
        blob: vec![peer],
    }
}

#[tokio::test]
async fn peer_refreshes_serialize_and_only_cache_committed_snapshots() {
    let initial = snapshot("initial");
    let interrupted = snapshot("interrupted");
    let latest = snapshot("latest");
    let state = Arc::new(PeerConfigRefresh::new(Some(initial.clone())));

    let (initial_refresh, ()) = state.load_serialized(|| ()).await;
    assert!(initial_refresh.is_current(&initial));
    drop(initial_refresh);
    let (interrupted_refresh, ()) = state.load_serialized(|| ()).await;
    assert!(!interrupted_refresh.is_current(&interrupted));

    let waiting_state = state.clone();
    let waiting_snapshot = latest.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (entered_tx, mut entered_rx) = tokio::sync::oneshot::channel();
    let waiting_refresh = tokio::spawn(async move {
        started_tx.send(()).unwrap();
        let (refresh, ()) = waiting_state.load_serialized(|| ()).await;
        entered_tx.send(()).unwrap();
        assert!(!refresh.is_current(&waiting_snapshot));
        refresh.commit(waiting_snapshot);
    });

    started_rx.await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut entered_rx)
            .await
            .is_err(),
        "a newer peer update must wait for the in-flight endpoint update"
    );
    drop(interrupted_refresh);
    tokio::time::timeout(std::time::Duration::from_secs(1), &mut entered_rx)
        .await
        .unwrap()
        .unwrap();
    waiting_refresh.await.unwrap();

    assert_eq!(state.applied(), Some(latest.clone()));
    let (latest_refresh, ()) = state.load_serialized(|| ()).await;
    assert!(latest_refresh.is_current(&latest));
    drop(latest_refresh);
    let (retry_refresh, ()) = state.load_serialized(|| ()).await;
    assert!(
        !retry_refresh.is_current(&interrupted),
        "an uncommitted peer update must remain retryable"
    );
}

#[tokio::test]
async fn serialized_loader_reads_state_after_waiting_for_an_older_refresh() {
    let initial = snapshot("initial");
    let superseded = snapshot("stale");
    let latest = snapshot("latest");
    let state = Arc::new(PeerConfigRefresh::new(Some(initial)));
    let desired = Arc::new(Mutex::new(superseded));
    let (blocking_refresh, ()) = state.load_serialized(|| ()).await;

    let waiting_state = state.clone();
    let waiting_desired = desired.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (loaded_tx, mut loaded_rx) = tokio::sync::oneshot::channel();
    let waiting = tokio::spawn(async move {
        started_tx.send(()).unwrap();
        let (refresh, loaded) = waiting_state
            .load_serialized(|| {
                let loaded = waiting_desired
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .clone();
                loaded_tx.send(()).unwrap();
                loaded
            })
            .await;
        refresh.commit(loaded.clone());
        loaded
    });
    started_rx.await.unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), &mut loaded_rx)
            .await
            .is_err(),
        "the persisted state loader must wait behind the active refresh"
    );
    *desired
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = latest.clone();
    drop(blocking_refresh);
    loaded_rx.await.unwrap();

    assert_eq!(waiting.await.unwrap(), latest);
    assert_eq!(state.applied(), Some(latest));
}
