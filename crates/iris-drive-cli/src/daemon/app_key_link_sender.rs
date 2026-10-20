fn spawn_app_key_link_roster_sender(
    config_dir: &Path,
    client: &nostr_sdk::Client,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    daemon_tasks: &DaemonTaskSet,
) -> tokio::sync::watch::Sender<Arc<BTreeSet<String>>> {
    let config_dir = config_dir.to_path_buf();
    let client = client.clone();
    let (acked_tx, acked_rx) = tokio::sync::watch::channel(Arc::new(BTreeSet::new()));
    daemon_tasks.push(tokio::spawn(async move {
        let relay = run_app_key_link_roster_sender(&config_dir, Some(&client), None, &acked_rx);
        if let Some(fips) = fips_blocks.as_deref() {
            // Each transport stays single-flight with its own retry cache.
            // A slow relay cannot hold a newer direct approval behind its
            // earlier snapshot. Both loops remain owned by this one task.
            let direct = run_app_key_link_roster_sender(&config_dir, None, Some(fips), &acked_rx);
            tokio::join!(direct, relay);
        } else {
            relay.await;
        }
    }));
    acked_tx
}

async fn run_app_key_link_roster_sender(
    config_dir: &Path,
    client: Option<&nostr_sdk::Client>,
    fips_blocks: Option<&FsFipsBlockSync>,
    acked_rx: &tokio::sync::watch::Receiver<Arc<BTreeSet<String>>>,
) {
    let mut cache = AuthorizedAppKeyLinkRosterSendCache::default();
    let mut timer =
        tokio::time::interval(std::time::Duration::from_millis(APP_KEY_LINK_TICK_MILLIS));
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        timer.tick().await;
        // Never hold a watch borrow across network I/O.
        let acked = acked_rx.borrow().clone();
        match Box::pin(send_authorized_app_key_link_rosters(
            config_dir,
            client,
            fips_blocks,
            &mut cache,
            &acked,
        ))
        .await
        {
            Ok(Some(payload)) => println!("{payload}"),
            Ok(None) => {}
            Err(error) => println!(
                "{}",
                json!({"event": "app_key_link_roster_send_error", "error": format!("{error:#}")})
            ),
        }
    }
}
