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
        let mut cache = AuthorizedAppKeyLinkRosterSendCache::default();
        let mut timer = tokio::time::interval(std::time::Duration::from_millis(
            APP_KEY_LINK_TICK_MILLIS,
        ));
        timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            timer.tick().await;
            // One persistent sender keeps retries single-flight and retains the
            // cache across ticks. Never hold a watch borrow across network I/O.
            let acked = acked_rx.borrow().clone();
            match Box::pin(send_authorized_app_key_link_rosters(
                &config_dir,
                &client,
                fips_blocks.as_deref(),
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
    }));
    acked_tx
}
