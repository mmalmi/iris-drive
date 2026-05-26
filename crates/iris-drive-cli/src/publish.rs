#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn cmd_publish(
    config_dir: &std::path::Path,
    relay_override: &[String],
    timeout_secs: u64,
) -> Result<()> {
    use iris_drive_core::relay_sync;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let config = AppConfig::load_or_default(config_path_in(config_dir))?;
        let state = config
            .account
            .clone()
            .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
        let relays = pick_relays(&config, relay_override);
        let _ = timeout_secs; // connect timeout not used by add_relay; kept for future
        let client = relay_sync::connect(&relays)
            .await
            .context("connecting to relays")?;

        let report = publish_current_state(&client, config_dir, &config, &state, true).await?;

        let _ = client.disconnect().await;
        let drive_iris_to_url = report
            .root_cid
            .as_ref()
            .and_then(|_| drive_iris_to_url_for_primary_drive(&config));
        let snapshot_url = report
            .root_cid
            .as_deref()
            .and_then(drive_iris_to_snapshot_url_for_root);
        println!(
            "{}",
            json!({
                "relays": relays,
                "blossom_servers": config.blossom_servers,
                "published_app_keys": report.published_app_keys,
                "app_keys_publish_error": report.app_keys_publish_error,
                "published_drive_root": report.published_drive_root,
                "drive_root_publish_error": report.drive_root_publish_error,
                "published_files_root": report.published_files_root,
                "files_root_publish_error": report.files_root_publish_error,
                "root_cid": report.root_cid,
                "drive_iris_to_url": drive_iris_to_url,
                "files_iris_to_url": drive_iris_to_url,
                "snapshot_url": snapshot_url,
                "permalink_url": snapshot_url,
                "blossom_upload_error": report.blossom_upload_error,
                "blossom_upload": report.blossom_upload.map(|r| json!({
                    "total_hashes": r.total_hashes,
                    "uploaded": r.uploaded,
                    "already_present": r.already_present,
                })),
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}

#[derive(Debug, Default)]
pub(crate) struct PublishStateReport {
    published_app_keys: bool,
    app_keys_publish_error: Option<String>,
    published_drive_root: bool,
    drive_root_publish_error: Option<String>,
    published_files_root: bool,
    files_root_publish_error: Option<String>,
    root_cid: Option<String>,
    blossom_upload: Option<UploadReport>,
    blossom_upload_error: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct DirectRootEvent {
    pub(crate) key: String,
    event_id: String,
    kind: u16,
    json: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct DirectRootFrame {
    key: String,
    event_id: String,
    event_json: String,
}

pub(crate) const DIRECT_ROOT_APP_TOPIC: &str = "iris-drive/root-events/v1/direct";

#[derive(Default)]
pub(crate) struct DirectRootExchange {
    cached_events: BTreeMap<String, DirectRootEvent>,
    published_keys: BTreeMap<String, std::time::Instant>,
    seen_keys: BTreeSet<String>,
    subscribed_streams: BTreeSet<String>,
    known_mesh_peers: BTreeSet<String>,
    next_mesh_publish_seq: u64,
}

impl DirectRootExchange {
    async fn subscribe_owner_stream(&mut self, owner_pubkey: &str, sync: Option<&FsFipsBlockSync>) {
        let Some(sync) = sync else {
            self.subscribed_streams.clear();
            return;
        };
        let stream = direct_root_mesh_stream(owner_pubkey);
        let peers_changed = self.refresh_known_mesh_peers(sync).await;
        if self.subscribed_streams.insert(stream.clone()) || peers_changed {
            let subscribe_stats = sync.subscribe_mesh_pubsub(stream.clone()).await;
            println!(
                "{}",
                json!({
                    "event": "direct_root_mesh_subscribe",
                    "stream": stream,
                    "selected_peers": subscribe_stats.selected_peers,
                    "sent_peers": subscribe_stats.sent_peers,
                })
            );
        }
    }

    async fn announce_current_state(
        &mut self,
        config_dir: &Path,
        config: &AppConfig,
        state: &AccountState,
        fips_blocks: Option<&FsFipsBlockSync>,
    ) -> Result<()> {
        let Some(sync) = fips_blocks else {
            return Ok(());
        };
        self.subscribe_owner_stream(&state.owner_pubkey, Some(sync))
            .await;
        let stream = direct_root_mesh_stream(&state.owner_pubkey);
        let events = build_current_sync_events(config_dir, config, state).await?;
        let now = std::time::Instant::now();
        for event in events {
            let event = self.event_for_publish(event);
            let should_publish = self.should_publish_key(&event.key, now);
            self.cache_event(event.clone());
            if !should_publish {
                continue;
            }
            let frame = DirectRootFrame {
                key: event.key.clone(),
                event_id: event.event_id.clone(),
                event_json: event.json.clone(),
            };
            let bytes = serde_json::to_vec(&frame)?;
            let selected_app_peers = sync.authorized_peer_ids().await.len();
            let sent_app_peers = sync
                .broadcast_app_message(DIRECT_ROOT_APP_TOPIC, bytes.clone())
                .await?;
            println!(
                "{}",
                json!({
                    "event": "direct_root_app_publish",
                    "topic": DIRECT_ROOT_APP_TOPIC,
                    "root_key": event.key.clone(),
                    "root_event_id": event.event_id.clone(),
                    "kind": event.kind,
                    "selected_peers": selected_app_peers,
                    "sent_peers": sent_app_peers,
                    "sent_bytes": bytes.len(),
                })
            );
            let seq = self.next_mesh_publish_seq();
            let publish_stats = sync.publish_mesh_pubsub(stream.clone(), seq, bytes).await;
            println!(
                "{}",
                json!({
                    "event": "direct_root_mesh_publish",
                    "stream": stream,
                    "seq": seq,
                    "root_key": event.key,
                    "root_event_id": event.event_id,
                    "kind": event.kind,
                    "selected_peers": publish_stats.selected_peers,
                    "sent_peers": publish_stats.sent_peers,
                    "sent_bytes": publish_stats.sent_bytes,
                })
            );
        }
        Ok(())
    }

    async fn apply_direct_root_frame(
        &mut self,
        client: &nostr_sdk::Client,
        config_dir: &Path,
        sync: Arc<FsFipsBlockSync>,
        mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
        frame: DirectRootFrame,
    ) -> Result<bool> {
        if self.seen_keys.contains(&frame.key) {
            return Ok(false);
        }
        let event: Event =
            serde_json::from_str(&frame.event_json).context("parsing direct root event")?;
        if event.id.to_hex() != frame.event_id {
            return Err(anyhow::anyhow!("direct root event id mismatch"));
        }
        self.seen_keys.insert(frame.key.clone());
        if let Err(error) = apply_one_event(
            client,
            config_dir,
            &event,
            Some(sync.clone()),
            mount_refresh.clone(),
        )
        .await
        {
            self.seen_keys.remove(&frame.key);
            return Err(error);
        }
        let config = AppConfig::load_or_default(config_path_in(config_dir))?;
        if let Some(state) = config.account.as_ref() {
            self.announce_current_state(config_dir, &config, state, Some(sync.as_ref()))
                .await?;
        }
        Ok(true)
    }

    pub(crate) async fn request_roots_from_new_peers(
        &mut self,
        config_dir: &Path,
        sync: Option<&FsFipsBlockSync>,
    ) {
        let Some(sync) = sync else {
            self.subscribed_streams.clear();
            return;
        };
        let Ok(config) = AppConfig::load_or_default(config_path_in(config_dir)) else {
            return;
        };
        if let Some(state) = config.account.as_ref() {
            self.subscribe_owner_stream(&state.owner_pubkey, Some(sync))
                .await;
        }
    }

    pub(crate) async fn drain_mesh_events(
        &mut self,
        client: &nostr_sdk::Client,
        config_dir: &Path,
        sync: Arc<FsFipsBlockSync>,
        mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
    ) -> Result<()> {
        for message in sync.drain_mesh_pubsub_events().await {
            if !message
                .stream_id
                .starts_with(DIRECT_ROOT_MESH_STREAM_PREFIX)
            {
                continue;
            }
            let frame: DirectRootFrame =
                serde_json::from_slice(&message.payload).context("parsing mesh root frame")?;
            let root_key = frame.key.clone();
            let root_event_id = frame.event_id.clone();
            if !self
                .apply_direct_root_frame(
                    client,
                    config_dir,
                    sync.clone(),
                    mount_refresh.clone(),
                    frame,
                )
                .await?
            {
                continue;
            }
            println!(
                "{}",
                json!({
                    "event": "direct_root_mesh_event",
                    "stream": message.stream_id,
                    "peer": message.from_peer_id,
                    "origin": message.origin_peer_id,
                    "seq": message.seq,
                    "root_key": root_key,
                    "root_event_id": root_event_id,
                })
            );
        }
        Ok(())
    }

    pub(crate) async fn handle_app_message(
        &mut self,
        client: &nostr_sdk::Client,
        config_dir: &Path,
        sync: Arc<FsFipsBlockSync>,
        mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
        message: iris_drive_core::FipsAppMessage,
    ) -> Result<()> {
        if message.topic != DIRECT_ROOT_APP_TOPIC {
            return Ok(());
        }
        let frame: DirectRootFrame =
            serde_json::from_slice(&message.data).context("parsing app root frame")?;
        let root_key = frame.key.clone();
        let root_event_id = frame.event_id.clone();
        if !self
            .apply_direct_root_frame(client, config_dir, sync, mount_refresh, frame)
            .await?
        {
            return Ok(());
        }
        println!(
            "{}",
            json!({
                "event": "direct_root_app_event",
                "topic": message.topic,
                "peer": message.peer_id,
                "root_key": root_key,
                "root_event_id": root_event_id,
            })
        );
        Ok(())
    }

    fn cache_event(&mut self, event: DirectRootEvent) {
        self.seen_keys.insert(event.key.clone());
        self.cached_events.insert(event.key.clone(), event);
        while self.cached_events.len() > DIRECT_ROOT_EVENT_CACHE_CAP {
            let Some(key) = self.cached_events.keys().next().cloned() else {
                break;
            };
            self.cached_events.remove(&key);
            self.published_keys.remove(&key);
        }
    }

    fn event_for_publish(&self, event: DirectRootEvent) -> DirectRootEvent {
        self.cached_events.get(&event.key).cloned().unwrap_or(event)
    }

    fn should_publish_key(&mut self, key: &str, now: std::time::Instant) -> bool {
        if self.published_keys.get(key).is_some_and(|last| {
            now.duration_since(*last)
                < std::time::Duration::from_secs(DIRECT_ROOT_REPUBLISH_INTERVAL_SECS)
        }) {
            return false;
        }
        self.published_keys.insert(key.to_string(), now);
        true
    }

    async fn refresh_known_mesh_peers(&mut self, sync: &FsFipsBlockSync) -> bool {
        let mut peers = sync
            .connected_peer_ids()
            .await
            .into_iter()
            .collect::<BTreeSet<_>>();
        peers.extend(sync.mesh_peer_ids().await);
        if peers != self.known_mesh_peers {
            self.known_mesh_peers = peers;
            self.published_keys.clear();
            return true;
        }
        false
    }

    fn next_mesh_publish_seq(&mut self) -> u64 {
        if self.next_mesh_publish_seq == 0 {
            self.next_mesh_publish_seq = direct_root_initial_seq();
        } else {
            self.next_mesh_publish_seq = self.next_mesh_publish_seq.saturating_add(1);
        }
        self.next_mesh_publish_seq
    }
}

pub(crate) async fn build_current_sync_events(
    config_dir: &Path,
    config: &AppConfig,
    state: &AccountState,
) -> Result<Vec<DirectRootEvent>> {
    let mut events = Vec::new();

    if state.has_owner_signing_authority
        && let Some(snap) = state.app_keys.as_ref()
    {
        let account = Account::load(state.clone(), config_dir).context("loading account")?;
        let owner_keys = account
            .owner_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("owner key missing on disk"))?
            .keys();
        let event = iris_drive_core::nostr_events::build_app_keys_event(owner_keys, snap)
            .context("building AppKeys event")?;
        events.push(direct_root_event(
            format!(
                "appkeys:{}:{}:{}:{}",
                snap.owner_pubkey,
                snap.created_at,
                snap.dck_generation,
                snap.devices
                    .iter()
                    .map(|device| device.pubkey.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            &event,
        )?);
    }

    if let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)
        && let Some(root) = publishable_device_root(config_dir, drive, state).await?
    {
        ensure_publishable_root_locally_available(config_dir, &root.root_cid).await?;
        let authorized_devices = authorized_device_pubkeys(state);
        let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
        let event = iris_drive_core::nostr_events::build_drive_root_event(
            device.keys(),
            &state.owner_pubkey,
            &drive.drive_id,
            &root,
            &authorized_devices,
        )
        .context("building drive-root event")?;
        events.push(direct_root_event(
            format!(
                "drive-root:{}:{}:{}:{}:{}",
                state.device_pubkey,
                drive.drive_id,
                root.device_seq,
                root.root_cid,
                authorized_devices.join(",")
            ),
            &event,
        )?);

        if state.has_owner_signing_authority {
            let account = Account::load(state.clone(), config_dir).context("loading account")?;
            let owner_keys = account
                .owner_key
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("owner key missing on disk"))?
                .keys();
            let event = iris_drive_core::nostr_events::build_private_hashtree_root_event(
                owner_keys,
                &drive.drive_id,
                &root,
            )
            .context("building files-root event")?;
            events.push(direct_root_event(
                format!(
                    "files-root:{}:{}:{}",
                    state.owner_pubkey, drive.drive_id, root.root_cid
                ),
                &event,
            )?);
        }
    }

    Ok(events)
}

pub(crate) async fn publishable_device_root(
    config_dir: &Path,
    drive: &Drive,
    state: &AccountState,
) -> Result<Option<DeviceRootRef>> {
    let Some(root) = drive.device_roots.get(&state.device_pubkey).cloned() else {
        return Ok(None);
    };
    if !root.materialized_only {
        return Ok(Some(root));
    }
    publishable_parent_root(config_dir, state, root).await
}

pub(crate) async fn publishable_parent_root(
    config_dir: &Path,
    state: &AccountState,
    mut root: DeviceRootRef,
) -> Result<Option<DeviceRootRef>> {
    let daemon = Daemon::open(config_dir).context("opening daemon for publishable root lookup")?;
    let mut seen = BTreeSet::new();
    for _ in 0..32 {
        if !seen.insert(root.root_cid.clone()) {
            return Ok(None);
        }
        let cid = Cid::parse(&root.root_cid)
            .with_context(|| format!("parsing root cid {}", root.root_cid))?;
        let Some(meta) = iris_drive_core::indexer::read_root_meta(daemon.tree(), &cid)
            .await
            .with_context(|| format!("reading root metadata for {}", root.root_cid))?
        else {
            return Ok(None);
        };
        let Some(parent) = meta
            .parents
            .iter()
            .find(|parent| parent.device_id == state.device_pubkey)
        else {
            return Ok(None);
        };
        let parent_cid = Cid::parse(&parent.root_cid)
            .with_context(|| format!("parsing parent root cid {}", parent.root_cid))?;
        let parent_root = match iris_drive_core::indexer::read_root_meta(daemon.tree(), &parent_cid)
            .await
            .with_context(|| format!("reading parent root metadata for {}", parent.root_cid))?
        {
            Some(parent_meta) => DeviceRootRef::from_meta(
                parent.root_cid.clone(),
                parent_meta.created_at,
                &parent_meta,
            ),
            None => DeviceRootRef::legacy(
                parent.root_cid.clone(),
                root.published_at,
                root.dck_generation,
            ),
        };
        if !parent_root.materialized_only {
            return Ok(Some(parent_root));
        }
        root = parent_root;
    }
    Ok(None)
}

pub(crate) fn direct_root_event(key: String, event: &Event) -> Result<DirectRootEvent> {
    Ok(DirectRootEvent {
        key,
        event_id: event.id.to_hex(),
        kind: event.kind.as_u16(),
        json: serde_json::to_string(&event)?,
    })
}

pub(crate) fn direct_root_mesh_stream(owner_pubkey: &str) -> String {
    format!("{DIRECT_ROOT_MESH_STREAM_PREFIX}/{owner_pubkey}")
}

pub(crate) async fn ensure_publishable_root_locally_available(
    config_dir: &Path,
    root_cid_str: &str,
) -> Result<()> {
    let root_cid =
        Cid::parse(root_cid_str).with_context(|| format!("parsing root cid {root_cid_str}"))?;
    let mut last_error: Option<anyhow::Error> = None;
    for delay_ms in
        std::iter::once(0).chain(LOCAL_ROOT_AVAILABILITY_RETRY_DELAYS_MS.iter().copied())
    {
        if delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
        }
        match collect_local_root_hashes(config_dir, &root_cid).await {
            Ok(_) => return Ok(()),
            Err(error)
                if delay_ms < *LOCAL_ROOT_AVAILABILITY_RETRY_DELAYS_MS.last().unwrap_or(&0)
                    && local_root_availability_error_message_is_retryable(&format!(
                        "{error:#}"
                    )) =>
            {
                tracing::warn!(
                    root_cid = root_cid_str,
                    delay_ms,
                    error = %error,
                    "publishable root hit a transient local store read; retrying"
                );
                last_error = Some(error);
            }
            Err(error) => {
                return Err(error).with_context(|| {
                    format!("root {root_cid_str} is not locally readable for publish")
                });
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("root availability check did not run")))
        .with_context(|| format!("root {root_cid_str} is not locally readable for publish"))
}

async fn collect_local_root_hashes(config_dir: &Path, root: &Cid) -> Result<usize> {
    let daemon = Daemon::open(config_dir).context("opening daemon for local root availability")?;
    daemon
        .tree()
        .list_directory(root)
        .await
        .context("reading local root directory")?;
    let hashes = iris_drive_core::block_sync::collect_live_sync_hashes(daemon.tree(), root, 4)
        .await
        .context("walking local live-sync root blocks")?;
    let store = daemon.tree().get_store().clone();
    for hash in &hashes {
        if !store
            .has(hash)
            .await
            .with_context(|| format!("checking local block {}", to_hex(hash)))?
        {
            return Err(anyhow::anyhow!(
                "local store is missing root block {}",
                to_hex(hash)
            ));
        }
    }
    Ok(hashes.len())
}

fn local_root_availability_error_message_is_retryable(message: &str) -> bool {
    let has_store_context = message.contains("Store error") || message.contains("IO error");
    has_store_context
        && (message.contains("os error 2")
            || message.contains("No such file or directory")
            || message.contains("The system cannot find the file specified"))
}

fn direct_root_initial_seq() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().try_into().unwrap_or(u64::MAX - 1))
        .unwrap_or(1)
        .max(1)
}

pub(crate) async fn announce_current_state_direct(
    direct_roots: &mut DirectRootExchange,
    config_dir: &Path,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(state) = config.account.as_ref() else {
        return Ok(());
    };
    direct_roots
        .announce_current_state(config_dir, &config, state, fips_blocks)
        .await
}

pub(crate) async fn upload_tree_to_blossom_with_hashtree(
    config_dir: &std::path::Path,
    config: &AppConfig,
    device: &iris_drive_core::DeviceIdentity,
    root_cid: Cid,
    _previous_root_cid: Option<Cid>,
) -> Result<UploadReport> {
    if config.blossom_servers.is_empty() {
        return Err(anyhow::anyhow!("no blossom servers configured"));
    }

    let bclient =
        iris_drive_core::blossom_sync_client(device.keys().clone(), &config.blossom_servers);
    let daemon = Daemon::open(config_dir).context("opening daemon for blossom upload")?;
    iris_drive_core::blossom_sync::upload_tree(daemon.tree(), &root_cid, &bclient)
        .await
        .context("uploading tree to blossom")
}

pub(crate) async fn maybe_upload_root_to_blossom(
    config_dir: &std::path::Path,
    config: &AppConfig,
    device: &iris_drive_core::DeviceIdentity,
    root_cid_str: &str,
    previous_root_cid: Option<&str>,
) -> Result<(Option<UploadReport>, Option<String>)> {
    if config.blossom_servers.is_empty() {
        return Ok((None, None));
    }

    let root_cid =
        Cid::parse(root_cid_str).with_context(|| format!("parsing root cid {root_cid_str}"))?;
    let previous_root_cid = previous_root_cid
        .map(Cid::parse)
        .transpose()
        .context("parsing previous root cid")?;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(BLOSSOM_UPLOAD_TIMEOUT_SECS),
        upload_tree_to_blossom_with_hashtree(
            config_dir,
            config,
            device,
            root_cid,
            previous_root_cid,
        ),
    )
    .await;
    Ok(match result {
        Ok(Ok(upload)) => (Some(upload), None),
        Ok(Err(error)) => (None, Some(format!("{error:#}"))),
        Err(_) => (
            None,
            Some(format!("timed out after {BLOSSOM_UPLOAD_TIMEOUT_SECS}s")),
        ),
    })
}

pub(crate) async fn start_fips_block_sync(
    config_dir: &std::path::Path,
    config: &AppConfig,
) -> Result<FsFipsBlockSync> {
    let device = iris_drive_core::DeviceIdentity::load(key_path_in(config_dir))
        .context("loading device key")?;
    let daemon = Daemon::open(config_dir).context("opening daemon for direct FIPS sync")?;
    let local = daemon.tree().get_store().clone();
    iris_drive_core::FipsBlockSync::start(&device, local, config)
        .await
        .context("starting direct FIPS block sync")
}

pub(crate) async fn download_roots_over_fips(
    fips: &FsFipsBlockSync,
    root_cid_strs: &[String],
    policy: FipsDownloadPolicy,
) -> Result<DownloadReport> {
    let mut totals = DownloadReport::default();
    for cid_str in root_cid_strs {
        let cid = Cid::parse(cid_str).with_context(|| format!("parsing root cid {cid_str}"))?;
        let report = download_tree_over_fips_with_retry(fips, &cid, policy)
            .await
            .with_context(|| format!("downloading tree over FIPS for {cid_str}"))?;
        add_download_report(&mut totals, report);
    }
    Ok(totals)
}

pub(crate) async fn download_tree_over_fips_with_retry(
    fips: &FsFipsBlockSync,
    root: &Cid,
    policy: FipsDownloadPolicy,
) -> Result<DownloadReport> {
    let mut last_error: Option<anyhow::Error> = None;
    for delay in std::iter::once(0).chain(policy.retry_delays.iter().copied()) {
        if delay > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        }
        match tokio::time::timeout(policy.attempt_timeout, fips.download_tree(root)).await {
            Ok(Ok(report)) => return Ok(report),
            Ok(Err(error)) => last_error = Some(anyhow::Error::from(error)),
            Err(_) => {
                last_error = Some(anyhow::anyhow!(
                    "FIPS download timed out after {}s",
                    policy.attempt_timeout.as_secs()
                ));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("FIPS download failed")))
}

#[derive(Clone, Copy)]
pub(crate) struct FipsDownloadPolicy {
    retry_delays: &'static [u64],
    attempt_timeout: std::time::Duration,
}

pub(crate) fn fips_download_policy(config: &AppConfig) -> FipsDownloadPolicy {
    if config.blossom_servers.is_empty() {
        FipsDownloadPolicy {
            retry_delays: FIPS_DOWNLOAD_RETRY_DELAYS,
            attempt_timeout: std::time::Duration::from_secs(FIPS_DOWNLOAD_ATTEMPT_TIMEOUT_SECS),
        }
    } else {
        FipsDownloadPolicy {
            retry_delays: FIPS_DOWNLOAD_BEFORE_BLOSSOM_RETRY_DELAYS,
            attempt_timeout: std::time::Duration::from_secs(
                FIPS_DOWNLOAD_BEFORE_BLOSSOM_ATTEMPT_TIMEOUT_SECS,
            ),
        }
    }
}

pub(crate) async fn download_roots_over_blossom(
    config_dir: &std::path::Path,
    config: &AppConfig,
    root_cid_strs: &[String],
) -> Result<DownloadReport> {
    if config.blossom_servers.is_empty() {
        return Err(anyhow::anyhow!("no blossom servers configured"));
    }

    let device = iris_drive_core::DeviceIdentity::load(key_path_in(config_dir))
        .context("loading device key")?;
    let daemon = Daemon::open(config_dir).context("opening daemon for Blossom sync")?;
    let local = daemon.tree().get_store().clone();
    let bclient =
        iris_drive_core::blossom_sync_client(device.keys().clone(), &config.blossom_servers);
    let mut totals = DownloadReport::default();
    for cid_str in root_cid_strs {
        let cid = Cid::parse(cid_str).with_context(|| format!("parsing root cid {cid_str}"))?;
        let report = iris_drive_core::blossom_sync::download_tree_with_retry(
            local.clone(),
            &cid,
            bclient.clone(),
            BLOSSOM_DOWNLOAD_RETRY_DELAYS,
        )
        .await
        .with_context(|| format!("downloading tree from Blossom for {cid_str}"))?;
        add_download_report(&mut totals, report);
    }
    Ok(totals)
}

pub(crate) fn add_download_report(total: &mut DownloadReport, report: DownloadReport) {
    total.total_hashes += report.total_hashes;
    total.fetched += report.fetched;
    total.already_local += report.already_local;
}

pub(crate) fn download_report_json(report: &DownloadReport) -> serde_json::Value {
    json!({
        "total_hashes": report.total_hashes,
        "fetched": report.fetched,
        "already_local": report.already_local,
    })
}

pub(crate) fn publish_state_report_json(report: &PublishStateReport) -> serde_json::Value {
    json!({
        "published_app_keys": report.published_app_keys,
        "app_keys_publish_error": report.app_keys_publish_error,
        "published_drive_root": report.published_drive_root,
        "drive_root_publish_error": report.drive_root_publish_error,
        "published_files_root": report.published_files_root,
        "files_root_publish_error": report.files_root_publish_error,
        "root_cid": report.root_cid,
        "blossom_upload_error": report.blossom_upload_error,
        "blossom_upload": report.blossom_upload.as_ref().map(|r| json!({
            "total_hashes": r.total_hashes,
            "uploaded": r.uploaded,
            "already_present": r.already_present,
        })),
    })
}

pub(crate) async fn import_mount_root_and_publish(
    client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    visible_root: Cid,
    tombstone_base_root: Option<Cid>,
    direct_roots: &mut DirectRootExchange,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<()> {
    import_mount_root_and_publish_with_tombstone_paths(
        client,
        config_dir,
        visible_root,
        tombstone_base_root,
        None,
        direct_roots,
        fips_blocks,
    )
    .await
}

pub(crate) async fn import_mount_root_and_publish_with_tombstone_paths(
    client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    visible_root: Cid,
    tombstone_base_root: Option<Cid>,
    tombstone_paths: Option<&BTreeSet<String>>,
    direct_roots: &mut DirectRootExchange,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<()> {
    if !mount_visible_root_has_changed(&visible_root, tombstone_base_root.as_ref()) {
        let payload = json!({
            "event": "mounted_root_unchanged",
            "root_cid": visible_root.to_string(),
            "publish": {"queued": false},
        });
        write_daemon_status(config_dir, payload.clone());
        println!("{payload}");
        return Ok(());
    }

    let mut daemon = Daemon::open(config_dir)
        .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
    let import = daemon
        .import_visible_root_with_tombstone_base_and_paths(
            visible_root,
            tombstone_base_root,
            tombstone_paths,
        )
        .await
        .context("importing mounted root")?;
    let updated_config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(updated_state) = updated_config.account.clone() else {
        return Err(anyhow::anyhow!("missing account after mount import"));
    };
    let direct_root_mesh_error =
        match announce_current_state_direct(direct_roots, config_dir, fips_blocks).await {
            Ok(()) => None,
            Err(error) => Some(format!("{error:#}")),
        };
    spawn_publish_current_state(
        client.clone(),
        config_dir.to_path_buf(),
        updated_config,
        updated_state,
        true,
        "mounted_root_publish_finished",
        json!({"root_cid": import.root_cid.clone()}),
    );
    println!(
        "{}",
        json!({
            "event": "mounted_root",
            "import": {
                "root_cid": import.root_cid,
                "file_count": import.file_count,
                "top_level_entries": import.top_level_entries,
            },
            "direct_root_mesh_error": direct_root_mesh_error,
            "publish": {"queued": true, "upload_blossom": true},
        })
    );
    Ok(())
}

fn mount_visible_root_has_changed(visible_root: &Cid, tombstone_base_root: Option<&Cid>) -> bool {
    !tombstone_base_root.is_some_and(|base| base == visible_root)
}

pub(crate) fn spawn_publish_current_state(
    client: nostr_sdk::Client,
    config_dir: PathBuf,
    config: AppConfig,
    state: AccountState,
    upload_blossom: bool,
    event_name: &'static str,
    context: Value,
) {
    tokio::spawn(async move {
        let started = std::time::Instant::now();
        let payload = match publish_current_state(
            &client,
            &config_dir,
            &config,
            &state,
            upload_blossom,
        )
        .await
        {
            Ok(report) => json!({
                "event": event_name,
                "elapsed_ms": started.elapsed().as_millis(),
                "context": context,
                "publish": publish_state_report_json(&report),
            }),
            Err(error) => json!({
                "event": format!("{event_name}_error"),
                "elapsed_ms": started.elapsed().as_millis(),
                "context": context,
                "error": format!("{error:#}"),
            }),
        };
        write_daemon_status(&config_dir, payload.clone());
        println!("{payload}");
    });
}

pub(crate) async fn publish_current_state(
    client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    config: &AppConfig,
    state: &AccountState,
    upload_blossom: bool,
) -> Result<PublishStateReport> {
    use iris_drive_core::relay_sync;

    let mut report = PublishStateReport::default();
    if state.has_owner_signing_authority
        && let Some(snap) = state.app_keys.as_ref()
    {
        let account = Account::load(state.clone(), config_dir).context("loading account")?;
        let owner_keys = account
            .owner_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("owner key missing on disk"))?
            .keys();
        match relay_publish_with_timeout(relay_sync::publish_app_keys(client, owner_keys, snap))
            .await
        {
            Ok(_) => report.published_app_keys = true,
            Err(error) => report.app_keys_publish_error = Some(error),
        }
    }

    if let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)
        && let Some(root) = publishable_device_root(config_dir, drive, state).await?
    {
        ensure_publishable_root_locally_available(config_dir, &root.root_cid).await?;
        let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
        report.root_cid = Some(root.root_cid.clone());

        if upload_blossom {
            let (blossom_upload, blossom_upload_error) =
                maybe_upload_root_to_blossom(config_dir, config, &device, &root.root_cid, None)
                    .await?;
            report.blossom_upload = blossom_upload;
            report.blossom_upload_error = blossom_upload_error;
        }

        match relay_publish_with_timeout(relay_sync::publish_drive_root(
            client,
            device.keys(),
            &state.owner_pubkey,
            &drive.drive_id,
            &root,
            &authorized_device_pubkeys(state),
        ))
        .await
        {
            Ok(_) => report.published_drive_root = true,
            Err(error) => report.drive_root_publish_error = Some(error),
        }

        if state.has_owner_signing_authority {
            let account = Account::load(state.clone(), config_dir).context("loading account")?;
            let owner_keys = account
                .owner_key
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("owner key missing on disk"))?
                .keys();
            match relay_publish_with_timeout(relay_sync::publish_files_root(
                client,
                owner_keys,
                &drive.drive_id,
                &root,
            ))
            .await
            {
                Ok(_) => report.published_files_root = true,
                Err(error) => {
                    report.files_root_publish_error = Some(error);
                }
            }
        }
    }

    Ok(report)
}

pub(crate) fn spawn_initial_publish(
    client: nostr_sdk::Client,
    config_dir: PathBuf,
    startup_config: AppConfig,
    startup_state: AccountState,
) {
    tokio::spawn(async move {
        match tokio::time::timeout(
            std::time::Duration::from_secs(STARTUP_NETWORK_TIMEOUT_SECS),
            publish_current_state(&client, &config_dir, &startup_config, &startup_state, true),
        )
        .await
        {
            Ok(Ok(report)) => {
                let drive_iris_to_url = report
                    .root_cid
                    .as_ref()
                    .and_then(|_| drive_iris_to_url_for_primary_drive(&startup_config));
                let snapshot_url = report
                    .root_cid
                    .as_deref()
                    .and_then(drive_iris_to_snapshot_url_for_root);
                println!(
                    "{}",
                    json!({
                        "event": "initial_publish",
                        "published_app_keys": report.published_app_keys,
                        "app_keys_publish_error": report.app_keys_publish_error,
                        "published_drive_root": report.published_drive_root,
                        "drive_root_publish_error": report.drive_root_publish_error,
                        "published_files_root": report.published_files_root,
                        "files_root_publish_error": report.files_root_publish_error,
                        "root_cid": report.root_cid,
                        "drive_iris_to_url": drive_iris_to_url,
                        "files_iris_to_url": drive_iris_to_url,
                        "snapshot_url": snapshot_url,
                        "permalink_url": snapshot_url,
                        "blossom_upload_error": report.blossom_upload_error,
                        "blossom_upload": report.blossom_upload.map(|r| json!({
                            "total_hashes": r.total_hashes,
                            "uploaded": r.uploaded,
                            "already_present": r.already_present,
                        })),
                    })
                );
            }
            Ok(Err(error)) => {
                println!(
                    "{}",
                    json!({"event": "initial_publish_error", "error": error.to_string()})
                );
            }
            Err(_) => {
                println!(
                    "{}",
                    json!({
                        "event": "initial_publish_error",
                        "error": format!("timed out after {STARTUP_NETWORK_TIMEOUT_SECS}s"),
                    })
                );
            }
        }
    });
}

pub(crate) fn spawn_daemon_heartbeat(config_dir: PathBuf) {
    let _ = std::thread::Builder::new()
        .name("idrive-status-heartbeat".to_string())
        .spawn(move || {
            loop {
                write_daemon_status(&config_dir, json!({"event": "heartbeat"}));
                std::thread::sleep(std::time::Duration::from_secs(2));
            }
        });
}

pub(crate) async fn relay_publish_with_timeout<T, F>(future: F) -> std::result::Result<T, String>
where
    F: std::future::Future<
            Output = std::result::Result<T, iris_drive_core::relay_sync::RelayError>,
        >,
{
    relay_publish_with_timeout_duration(
        std::time::Duration::from_secs(RELAY_PUBLISH_TIMEOUT_SECS),
        future,
    )
    .await
}

pub(crate) async fn relay_publish_with_timeout_duration<T, F>(
    timeout: std::time::Duration,
    future: F,
) -> std::result::Result<T, String>
where
    F: std::future::Future<
            Output = std::result::Result<T, iris_drive_core::relay_sync::RelayError>,
        >,
{
    match tokio::time::timeout(timeout, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err(format!("timed out after {}s", timeout.as_secs())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn direct_root_mesh_publish_sequence_is_monotonic() {
        let mut exchange = DirectRootExchange::default();

        let first = exchange.next_mesh_publish_seq();
        let second = exchange.next_mesh_publish_seq();
        let third = exchange.next_mesh_publish_seq();

        assert!(first > 0);
        assert_eq!(second, first + 1);
        assert_eq!(third, second + 1);
    }

    #[test]
    fn direct_root_mesh_reuses_cached_event_for_same_logical_root() {
        let mut exchange = DirectRootExchange::default();
        let first = DirectRootEvent {
            key: "drive-root:device:main:7:root".to_string(),
            event_id: "first-event".to_string(),
            kind: 30079,
            json: "{\"id\":\"first\"}".to_string(),
        };
        let rebuilt = DirectRootEvent {
            key: first.key.clone(),
            event_id: "rebuilt-event".to_string(),
            kind: 30079,
            json: "{\"id\":\"rebuilt\"}".to_string(),
        };

        exchange.cache_event(first.clone());
        let event = exchange.event_for_publish(rebuilt);

        assert_eq!(event.event_id, first.event_id);
        assert_eq!(event.json, first.json);
    }

    #[test]
    fn unchanged_mount_visible_root_is_not_publishable() {
        let root = Cid::encrypted([0x11; 32], [0x22; 32]);
        let other = Cid::encrypted([0x33; 32], [0x44; 32]);

        assert!(!mount_visible_root_has_changed(&root, Some(&root)));
        assert!(mount_visible_root_has_changed(&root, Some(&other)));
        assert!(mount_visible_root_has_changed(&root, None));
    }
}
