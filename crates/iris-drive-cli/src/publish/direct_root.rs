use iris_drive_core::{DIRECT_ROOT_APP_TOPIC, DIRECT_ROOT_MESH_STREAM_PREFIX, DirectRootFrame};

#[derive(Debug, Clone)]
pub(crate) struct DirectRootEvent {
    pub(crate) key: String,
    event_id: String,
    kind: u16,
    json: String,
}

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
    async fn subscribe_profile_stream(&mut self, root_scope_id: &str, sync: Option<&FsFipsBlockSync>) {
        let Some(sync) = sync else {
            self.subscribed_streams.clear();
            return;
        };
        let stream = direct_root_mesh_stream(root_scope_id);
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
        state: &ProfileState,
        fips_blocks: Option<&FsFipsBlockSync>,
    ) -> Result<()> {
        let Some(sync) = fips_blocks else {
            return Ok(());
        };
        let root_scope_id = state.root_scope_id();
        self.subscribe_profile_stream(&root_scope_id, Some(sync)).await;
        let stream = direct_root_mesh_stream(&root_scope_id);
        let events = self.events_for_publish(build_current_sync_events(config_dir, config, state).await?);
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
        let direct_event = direct_root_event(frame.key.clone(), &event)?;
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
        self.cache_event(direct_event);
        let config = AppConfig::load_or_default(config_path_in(config_dir))?;
        if let Some(state) = config.profile.as_ref() {
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
        if let Some(state) = config.profile.as_ref() {
            self.subscribe_profile_stream(&state.root_scope_id(), Some(sync))
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

    fn events_for_publish(&self, local_events: Vec<DirectRootEvent>) -> Vec<DirectRootEvent> {
        let mut events = Vec::with_capacity(local_events.len() + self.cached_events.len());
        let mut keys = BTreeSet::new();
        for event in local_events {
            let event = self.event_for_publish(event);
            keys.insert(event.key.clone());
            events.push(event);
        }
        events.extend(
            self.cached_events
                .values()
                .filter(|event| !keys.contains(&event.key))
                .cloned(),
        );
        events
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
        let authorized_peers = sync.authorized_peer_ids().await;
        let mesh_peers = sync.mesh_peer_ids().await;
        self.refresh_known_root_peers(authorized_peers, mesh_peers)
    }

    fn refresh_known_root_peers(
        &mut self,
        authorized_peers: impl IntoIterator<Item = String>,
        mesh_peers: impl IntoIterator<Item = String>,
    ) -> bool {
        let mut root_peers = authorized_peers.into_iter().collect::<BTreeSet<_>>();
        root_peers.extend(mesh_peers);
        if root_peers != self.known_mesh_peers {
            self.known_mesh_peers = root_peers;
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
    state: &ProfileState,
) -> Result<Vec<DirectRootEvent>> {
    let mut events = Vec::new();

    append_profile_roster_events(&mut events, state)?;
    append_share_roster_events(&mut events, config)?;
    append_primary_drive_root_events(&mut events, config_dir, config, state).await?;
    append_share_root_events(&mut events, config_dir, config, state).await?;

    Ok(events)
}

fn append_profile_roster_events(
    events: &mut Vec<DirectRootEvent>,
    state: &ProfileState,
) -> Result<()> {
    for op in &state.profile_roster_ops {
        let event =
            Event::from_json(&op.event_json).context("parsing IrisProfile roster op event")?;
        events.push(direct_root_event(
            format!("profile-op:{}:{}", state.profile_id, op.op_id),
            &event,
        )?);
    }
    Ok(())
}

fn append_share_roster_events(
    events: &mut Vec<DirectRootEvent>,
    config: &AppConfig,
) -> Result<()> {
    for folder in &config.shared_folders {
        for op in &folder.roster_ops {
            let event =
                Event::from_json(&op.event_json).context("parsing share roster op event")?;
            events.push(direct_root_event(
                format!("share-profile-op:{}:{}", folder.share_id, op.op_id),
                &event,
            )?);
        }
    }
    Ok(())
}

async fn append_primary_drive_root_events(
    events: &mut Vec<DirectRootEvent>,
    config_dir: &Path,
    config: &AppConfig,
    state: &ProfileState,
) -> Result<()> {
    if let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)
        && let Some(root) = publishable_app_key_root(config_dir, drive, state).await?
    {
        ensure_publishable_root_locally_available(config_dir, &root.root_cid).await?;
        let authorized_app_keys = authorized_app_key_pubkeys(state);
        let device = iris_drive_core::identity::AppKey::load(key_path_in(config_dir))
            .context("loading app key")?;
        let event = iris_drive_core::nostr_events::build_drive_root_event(
            device.keys(),
            &state.root_scope_id(),
            &drive.drive_id,
            &root,
            &authorized_app_keys,
        )
        .context("building drive-root event")?;
        events.push(direct_root_event(
            format!(
                "drive-root:{}:{}:{}:{}:{}",
                state.app_key_pubkey,
                drive.drive_id,
                root.app_key_seq,
                root.root_cid,
                authorized_app_keys.join(",")
            ),
            &event,
        )?);

        if state.can_write_roots() {
            let account = Profile::load(state.clone(), config_dir).context("loading profile")?;
            let event = iris_drive_core::nostr_events::build_private_hashtree_root_event(
                account.app_key.keys(),
                &drive.drive_id,
                &root,
            )
            .context("building files-root event")?;
            events.push(direct_root_event(
                format!(
                    "files-root:{}:{}:{}",
                    state.app_key_pubkey, drive.drive_id, root.root_cid
                ),
                &event,
            )?);
        }
    }
    Ok(())
}

async fn append_share_root_events(
    events: &mut Vec<DirectRootEvent>,
    config_dir: &Path,
    config: &AppConfig,
    state: &ProfileState,
) -> Result<()> {
    let device = iris_drive_core::identity::AppKey::load(key_path_in(config_dir))
        .context("loading app key")?;
    for folder in &config.shared_folders {
        let projection = folder.projection();
        if !projection.can_write_roots(&state.app_key_pubkey) {
            continue;
        }
        let Some(root) = folder.app_key_roots.get(&state.app_key_pubkey) else {
            continue;
        };
        if root.local_only {
            continue;
        }
        ensure_publishable_root_locally_available(config_dir, &root.root_cid).await?;
        let authorized_recipients = projection
            .active_facets
            .values()
            .filter(|facet| facet.capabilities.can_receive_key_wraps)
            .map(|facet| facet.pubkey.clone())
            .collect::<Vec<_>>();
        let event = iris_drive_core::nostr_events::build_drive_root_event(
            device.keys(),
            &folder.share_id.to_string(),
            iris_drive_core::PRIMARY_DRIVE_ID,
            root,
            &authorized_recipients,
        )
        .context("building share-root event")?;
        events.push(direct_root_event(
            format!(
                "share-root:{}:{}:{}:{}:{}",
                folder.share_id,
                state.app_key_pubkey,
                root.app_key_seq,
                root.root_cid,
                authorized_recipients.join(",")
            ),
            &event,
        )?);
    }

    Ok(())
}

pub(crate) async fn publishable_app_key_root(
    config_dir: &Path,
    drive: &Drive,
    state: &ProfileState,
) -> Result<Option<AppKeyRootRef>> {
    let Some(root) = drive.app_key_roots.get(&state.app_key_pubkey).cloned() else {
        return Ok(None);
    };
    if !root.local_only {
        return Ok(Some(root));
    }
    publishable_parent_root(config_dir, state, root).await
}

pub(crate) async fn publishable_parent_root(
    config_dir: &Path,
    state: &ProfileState,
    mut root: AppKeyRootRef,
) -> Result<Option<AppKeyRootRef>> {
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
            .find(|parent| parent.app_key_pubkey == state.app_key_pubkey)
        else {
            return Ok(None);
        };
        let parent_cid = Cid::parse(&parent.root_cid)
            .with_context(|| format!("parsing parent root cid {}", parent.root_cid))?;
        let parent_root = match iris_drive_core::indexer::read_root_meta(daemon.tree(), &parent_cid)
            .await
            .with_context(|| format!("reading parent root metadata for {}", parent.root_cid))?
        {
            Some(parent_meta) => AppKeyRootRef::from_meta(
                parent.root_cid.clone(),
                parent_meta.created_at,
                &parent_meta,
            ),
            None => AppKeyRootRef::legacy(
                parent.root_cid.clone(),
                root.published_at,
                root.dck_generation,
            ),
        };
        if !parent_root.local_only {
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

pub(crate) fn direct_root_mesh_stream(root_scope_id: &str) -> String {
    format!("{DIRECT_ROOT_MESH_STREAM_PREFIX}/{root_scope_id}")
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
        .map_or(1, |duration| {
            duration.as_millis().try_into().unwrap_or(u64::MAX - 1)
        })
        .max(1)
}
