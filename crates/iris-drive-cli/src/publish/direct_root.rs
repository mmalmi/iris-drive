use iris_drive_core::{
    DIRECT_ROOT_APP_TOPIC, DIRECT_ROOT_MESH_STREAM_PREFIX, DirectRootFrame, DirectRootHintApply,
    DirectRootHintFrame, DirectRootStateRequestFrame, DirectRootWireFrame, FipsMeshPubsubEvent,
};
const DIRECT_ROOT_STATE_REQUEST_INTERVAL_SECS: u64 = 10;
const DIRECT_ROOT_STATE_REQUEST_REPLY_REPUBLISH_INTERVAL_SECS: u64 = 10;
const DIRECT_ROOT_HINT_REPEAT_INTERVAL_SECS: u64 = 30;
const DIRECT_ROOT_HINT_CACHE_MAX_ENTRIES: usize = 2048;
const DIRECT_ROOT_SEEN_FRAME_RETRY_INTERVAL_SECS: u64 = 30;
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) struct DirectRootAppSendStats {
    pub(crate) selected_peers: usize,
    pub(crate) sent_peers: usize,
    pub(crate) failed_peers: usize,
}
#[derive(Debug, Clone)]
pub(crate) struct DirectRootEvent {
    pub(crate) key: String,
    event_id: String,
    kind: u16,
    json: String,
}
#[derive(Debug, Clone)]
struct DirectRootPublishEvent {
    event: DirectRootEvent,
    source: DirectRootPublishSource,
}
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum DirectRootPublishSource {
    LocalCurrent,
    StateRequestReply,
}
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum DirectRootFrameOutcome {
    Ignored,
    Cached,
    Changed,
}
impl DirectRootFrameOutcome {
    pub(crate) const fn should_log_event(self) -> bool {
        !matches!(self, Self::Ignored)
    }
    pub(crate) const fn should_schedule_announce(self) -> bool {
        matches!(self, Self::Changed)
    }
}
impl DirectRootPublishSource {
    fn as_str(self) -> &'static str {
        match self {
            Self::LocalCurrent => "local_current",
            Self::StateRequestReply => "state_request",
        }
    }
}

#[derive(Default)]
pub(crate) struct DirectRootExchange {
    cached_events: BTreeMap<String, DirectRootEvent>,
    published_keys: BTreeMap<String, std::time::Instant>,
    seen_keys: BTreeSet<String>,
    subscribed_streams: BTreeSet<String>,
    known_mesh_peers: BTreeSet<String>,
    known_publish_peers: BTreeSet<String>,
    known_visible_publish_peers: BTreeSet<String>,
    next_mesh_publish_seq: u64,
    profile_stream_cache: Option<CachedDirectRootProfileStream>,
    current_sync_events_cache: Option<CachedCurrentSyncEvents>,
    hint_config_cache: AppConfigLoadCache,
    state_request_times: BTreeMap<String, std::time::Instant>,
    state_request_reply_times: BTreeMap<String, std::time::Instant>,
    recent_hint_times: BTreeMap<String, std::time::Instant>,
    seen_frame_retry_times: BTreeMap<String, std::time::Instant>,
}
#[derive(Debug, Clone)]
struct CachedDirectRootProfileStream {
    config_fingerprint: ConfigFileFingerprint,
    root_scope_id: Option<String>,
}
#[derive(Debug, Clone)]
struct CachedCurrentSyncEvents {
    config_fingerprint: ConfigFileFingerprint,
    events: Vec<DirectRootEvent>,
}

impl DirectRootExchange {
    pub(crate) fn invalidate_current_sync_events_cache(&mut self) {
        self.current_sync_events_cache = None;
        self.state_request_reply_times.clear();
    }

    async fn subscribe_profile_stream(
        &mut self,
        root_scope_id: &str,
        sync: Option<&FsFipsBlockSync>,
    ) -> bool {
        let Some(sync) = sync else {
            self.subscribed_streams.clear();
            return false;
        };
        let stream = direct_root_mesh_stream(root_scope_id);
        let peers_changed = self.refresh_known_mesh_peers(sync).await;
        let should_subscribe = self.subscribed_streams.insert(stream.clone()) || peers_changed;
        if should_subscribe {
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
        should_subscribe
    }

    async fn announce_current_state(
        &mut self,
        config_dir: &Path,
        _config: &AppConfig,
        state: &ProfileState,
        fips_blocks: Option<&FsFipsBlockSync>,
    ) -> Result<()> {
        let Some(sync) = fips_blocks else {
            return Ok(());
        };
        let root_scope_id = state.root_scope_id();
        self.subscribe_profile_stream(&root_scope_id, Some(sync))
            .await;
        let stream = direct_root_mesh_stream(&root_scope_id);
        let config_fingerprint = config_file_fingerprint(&config_path_in(config_dir))?;
        let expected_root_scope_id = root_scope_id.clone();
        let local_events = self
            .cached_current_sync_events_from_config(config_fingerprint, || {
                build_current_sync_events_from_disk(config_dir, &expected_root_scope_id)
            })
            .await?;
        let events = self.events_for_publish(local_events);
        let now = std::time::Instant::now();
        for publish_event in events {
            self.publish_event(sync, &stream, publish_event, None, now)
                .await?;
        }
        Ok(())
    }

    async fn announce_state_request_reply(
        &mut self,
        config_dir: &Path,
        root_scope_id: &str,
        sync: &FsFipsBlockSync,
        reply_peer: &str,
    ) -> Result<()> {
        self.subscribe_profile_stream(root_scope_id, Some(sync)).await;
        let stream = direct_root_mesh_stream(root_scope_id);
        let local_events = self
            .state_request_current_sync_events(config_dir, root_scope_id)
            .await?;
        let now = std::time::Instant::now();
        for publish_event in self.state_request_events_for_publish(local_events) {
            self.publish_event(sync, &stream, publish_event, Some(reply_peer), now)
                .await?;
        }
        Ok(())
    }

    async fn state_request_current_sync_events(
        &mut self,
        config_dir: &Path,
        root_scope_id: &str,
    ) -> Result<Vec<DirectRootEvent>> {
        let expected_root_scope_id = root_scope_id.to_string();
        build_current_sync_events_from_disk(config_dir, &expected_root_scope_id).await
    }

    async fn publish_event(
        &mut self,
        sync: &FsFipsBlockSync,
        stream: &str,
        publish_event: DirectRootPublishEvent,
        target_peer: Option<&str>,
        now: std::time::Instant,
    ) -> Result<()> {
        let DirectRootPublishEvent { event, source } = publish_event;
        let event = self.event_for_publish(event);
        let should_publish =
            self.should_publish_candidate_key_for_target(&event.key, source, target_peer, now);
        self.cache_event(event.clone());
        if !should_publish {
            return Ok(());
        }
        let frame = DirectRootFrame {
            key: event.key.clone(),
            event_id: event.event_id.clone(),
            event_json: event.json.clone(),
        };
        let bytes = serde_json::to_vec(&frame)?;
        let hint_bytes = should_publish_direct_root_hint(&event.key, source)
            .then(|| iris_drive_core::encode_direct_root_hint_frame(&event.key, &event.event_id))
            .transpose()
            .context("encoding direct-root hint frame")?;
        let attempts = direct_root_publish_attempts_for_source(&event.key, source);
        let publish_targeted_reply_over_mesh =
            should_publish_targeted_direct_root_reply_over_mesh(source);
        for attempt in 0..attempts {
            let publish_full_frame =
                should_publish_direct_root_full_frame(&event.key, source, attempt);
            if let Some(hint_bytes) = hint_bytes.as_ref() {
                let (selected_app_peers, sent_app_peers) = if let Some(target_peer) = target_peer {
                    match sync
                        .send_app_message(target_peer, DIRECT_ROOT_APP_TOPIC, hint_bytes.clone())
                        .await
                    {
                        Ok(()) => (1, 1),
                        Err(error) if publish_targeted_reply_over_mesh => {
                            println!(
                                "{}",
                                json!({
                                    "event": "direct_root_app_hint_publish_error",
                                    "topic": DIRECT_ROOT_APP_TOPIC,
                                    "root_key": event.key.clone(),
                                    "root_event_id": event.event_id.clone(),
                                    "kind": event.kind,
                                    "source": source.as_str(),
                                    "attempt": attempt + 1,
                                    "attempts": attempts,
                                    "target_peer": target_peer,
                                    "error": format!("{error:#}"),
                                })
                            );
                            (1, 0)
                        }
                        Err(error) => return Err(error.into()),
                    }
                } else {
                    let stats =
                        send_direct_root_app_message_to_authorized_peers(sync, hint_bytes.clone())
                            .await;
                    (stats.selected_peers, stats.sent_peers)
                };
                println!(
                    "{}",
                    json!({
                        "event": "direct_root_app_hint_publish",
                        "topic": DIRECT_ROOT_APP_TOPIC,
                        "root_key": event.key.clone(),
                        "root_event_id": event.event_id.clone(),
                        "kind": event.kind,
                        "source": source.as_str(),
                        "attempt": attempt + 1,
                        "attempts": attempts,
                        "target_peer": target_peer,
                        "selected_peers": selected_app_peers,
                        "sent_peers": sent_app_peers,
                        "sent_bytes": hint_bytes.len(),
                    })
                );
            }
            if publish_full_frame {
                let (selected_app_peers, sent_app_peers) = if let Some(target_peer) = target_peer {
                    match sync
                        .send_app_message(target_peer, DIRECT_ROOT_APP_TOPIC, bytes.clone())
                        .await
                    {
                        Ok(()) => (1, 1),
                        Err(error) if publish_targeted_reply_over_mesh => {
                            println!(
                                "{}",
                                json!({
                                    "event": "direct_root_app_publish_error",
                                    "topic": DIRECT_ROOT_APP_TOPIC,
                                    "root_key": event.key.clone(),
                                    "root_event_id": event.event_id.clone(),
                                    "kind": event.kind,
                                    "source": source.as_str(),
                                    "attempt": attempt + 1,
                                    "attempts": attempts,
                                    "target_peer": target_peer,
                                    "error": format!("{error:#}"),
                                })
                            );
                            (1, 0)
                        }
                        Err(error) => return Err(error.into()),
                    }
                } else {
                    let stats =
                        send_direct_root_app_message_to_authorized_peers(sync, bytes.clone()).await;
                    (stats.selected_peers, stats.sent_peers)
                };
                println!(
                    "{}",
                    json!({
                        "event": "direct_root_app_publish",
                        "topic": DIRECT_ROOT_APP_TOPIC,
                        "root_key": event.key.clone(),
                        "root_event_id": event.event_id.clone(),
                        "kind": event.kind,
                        "source": source.as_str(),
                        "attempt": attempt + 1,
                        "attempts": attempts,
                        "target_peer": target_peer,
                        "selected_peers": selected_app_peers,
                        "sent_peers": sent_app_peers,
                        "sent_bytes": bytes.len(),
                    })
                );
            }
            if target_peer.is_some() && !publish_targeted_reply_over_mesh {
                continue;
            }
            if publish_full_frame {
                let seq = self.next_mesh_publish_seq();
                let publish_stats = sync
                    .publish_mesh_pubsub(stream.to_string(), seq, bytes.clone())
                    .await;
                println!(
                    "{}",
                    json!({
                        "event": "direct_root_mesh_publish",
                        "stream": stream,
                        "seq": seq,
                        "root_key": event.key.clone(),
                        "root_event_id": event.event_id.clone(),
                        "kind": event.kind,
                        "source": source.as_str(),
                        "attempt": attempt + 1,
                        "attempts": attempts,
                        "selected_peers": publish_stats.selected_peers,
                        "sent_peers": publish_stats.sent_peers,
                        "sent_bytes": publish_stats.sent_bytes,
                    })
                );
            }
            if let Some(hint_bytes) = hint_bytes.as_ref() {
                let seq = self.next_mesh_publish_seq();
                let publish_stats = sync
                    .publish_mesh_pubsub(stream.to_string(), seq, hint_bytes.clone())
                    .await;
                println!(
                    "{}",
                    json!({
                        "event": "direct_root_mesh_hint_publish",
                        "stream": stream,
                        "seq": seq,
                        "root_key": event.key.clone(),
                        "root_event_id": event.event_id.clone(),
                        "kind": event.kind,
                        "source": source.as_str(),
                        "attempt": attempt + 1,
                        "attempts": attempts,
                        "selected_peers": publish_stats.selected_peers,
                        "sent_peers": publish_stats.sent_peers,
                        "sent_bytes": publish_stats.sent_bytes,
                    })
                );
            }
        }
        Ok(())
    }

    async fn apply_direct_root_frame(
        &mut self,
        client: &nostr_sdk::Client,
        config_dir: &Path,
        sync: Arc<FsFipsBlockSync>,
        mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
        daemon_tasks: &DaemonTaskSet,
        frame: DirectRootFrame,
        source: &'static str,
        source_peer: &str,
    ) -> Result<DirectRootFrameOutcome> {
        if !self.should_cache_event_as_latest(&frame.key) {
            println!(
                "{}",
                json!({
                    "event": "direct_root_frame_ignored",
                    "reason": "not_latest",
                    "root_key": frame.key,
                    "root_event_id": frame.event_id,
                    "frame": "full",
                    "source": source,
                    "source_peer": source_peer,
                })
            );
            return Ok(DirectRootFrameOutcome::Ignored);
        }
        if self.should_skip_seen_direct_root_frame(&frame.key, std::time::Instant::now()) {
            println!(
                "{}",
                json!({
                    "event": "direct_root_frame_ignored",
                    "reason": "seen_recently",
                    "root_key": frame.key,
                    "root_event_id": frame.event_id,
                    "frame": "full",
                    "source": source,
                    "source_peer": source_peer,
                })
            );
            return Ok(DirectRootFrameOutcome::Ignored);
        }
        let event: Event =
            serde_json::from_str(&frame.event_json).context("parsing direct root event")?;
        if event.id.to_hex() != frame.event_id {
            return Err(anyhow::anyhow!("direct root event id mismatch"));
        }
        let direct_event = direct_root_event(frame.key.clone(), &event)?;
        let outcome = match apply_one_event(
            client,
            config_dir,
            &event,
            Some(sync.clone()),
            mount_refresh.clone(),
            daemon_tasks,
        )
        .await
        {
            Ok(outcome) => outcome,
            Err(error) => return Err(error),
        };
        if !outcome.should_cache_direct_root_frame() {
            println!(
                "{}",
                json!({
                    "event": "direct_root_frame_ignored",
                    "reason": "retryable_prerequisite_missing",
                    "root_key": frame.key,
                    "root_event_id": frame.event_id,
                    "frame": "full",
                    "outcome": format!("{outcome:?}"),
                    "source": source,
                    "source_peer": source_peer,
                })
            );
            return Ok(DirectRootFrameOutcome::Ignored);
        }
        self.cache_event(direct_event);
        if outcome.should_announce_current_state() {
            self.invalidate_current_sync_events_cache();
            return Ok(DirectRootFrameOutcome::Changed);
        }
        Ok(DirectRootFrameOutcome::Cached)
    }

    async fn apply_direct_root_hint_frame(
        &mut self,
        config_dir: &Path,
        sync: Arc<FsFipsBlockSync>,
        mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
        daemon_tasks: &DaemonTaskSet,
        frame: DirectRootHintFrame,
        source_peer: &str,
    ) -> Result<DirectRootFrameOutcome> {
        if !frame.hint {
            println!(
                "{}",
                json!({
                    "event": "direct_root_frame_ignored",
                    "reason": "not_hint",
                    "root_key": frame.key,
                    "root_event_id": frame.event_id,
                    "frame": "hint",
                })
            );
            return Ok(DirectRootFrameOutcome::Ignored);
        }
        if !self.should_cache_event_as_latest(&frame.key) {
            println!(
                "{}",
                json!({
                    "event": "direct_root_frame_ignored",
                    "reason": "not_latest",
                    "root_key": frame.key,
                    "root_event_id": frame.event_id,
                    "frame": "hint",
                })
            );
            return Ok(DirectRootFrameOutcome::Ignored);
        }
        if self.should_skip_recent_direct_root_hint(
            source_peer,
            &frame.key,
            std::time::Instant::now(),
        ) {
            println!(
                "{}",
                json!({
                    "event": "direct_root_frame_ignored",
                    "reason": "hint_seen_recently",
                    "root_key": frame.key,
                    "root_event_id": frame.event_id,
                    "frame": "hint",
                    "source_peer": source_peer,
                })
            );
            return Ok(DirectRootFrameOutcome::Ignored);
        }
        let config_lock = ConfigMutationLock::acquire(config_dir).await?;
        let config_path = config_path_in(config_dir);
        let mut config = load_app_config_cached(&config_path, &mut self.hint_config_cache)?;
        let report = iris_drive_core::apply_direct_root_key_hint_to_config(
            &mut config,
            &frame.key,
            source_peer,
            direct_root_hint_published_at(),
        )?;
        let was_applied = matches!(report.outcome, DirectRootHintApply::Applied);
        let already_current = matches!(report.outcome, DirectRootHintApply::AlreadyCurrent);
        let root_cid_to_pull = report
            .root_cid
            .as_ref()
            .filter(|_| was_applied || already_current)
            .cloned();
        let should_refresh_projection = was_applied || already_current;
        println!(
            "{}",
            json!({
                "event": "drive_root_hint",
                "event_id": frame.event_id,
                "source_peer": source_peer,
                "outcome": format!("{:?}", report.outcome),
                "root_key": frame.key,
                "root_cid": root_cid_to_pull.clone(),
            })
        );
        if was_applied {
            config.save(config_path)?;
            self.hint_config_cache.clear();
        }
        drop(config_lock);
        if was_applied {
            sync.refresh_authorized_peers(&config).await;
            self.invalidate_current_sync_events_cache();
        }
        enqueue_root_apply_followup(
            config_dir.to_path_buf(),
            config,
            root_cid_to_pull,
            Some(sync),
            should_refresh_projection,
            "projected_drive_root",
            mount_refresh,
            daemon_tasks,
        );
        Ok(if was_applied {
            DirectRootFrameOutcome::Changed
        } else if already_current {
            DirectRootFrameOutcome::Cached
        } else {
            DirectRootFrameOutcome::Ignored
        })
    }

    async fn handle_state_request_frame(
        &mut self,
        config_dir: &Path,
        sync: Arc<FsFipsBlockSync>,
        frame: DirectRootStateRequestFrame,
        reply_peer: &str,
    ) -> Result<()> {
        if !frame.request {
            return Ok(());
        }
        if reply_peer == sync.endpoint_npub() {
            println!(
                "{}",
                json!({
                    "event": "direct_root_state_request_ignored",
                    "reason": "self",
                    "root_scope_id": frame.root_scope_id,
                    "reply_peer": reply_peer,
                })
            );
            return Ok(());
        }
        let config_path = config_path_in(config_dir);
        let config_fingerprint = config_file_fingerprint(&config_path)?;
        let Some(root_scope_id) = self.cached_profile_stream_root_scope_id_from_config(
            config_fingerprint.clone(),
            || Ok(AppConfig::load_or_default_cached_profile(&config_path)?),
        )?
        else {
            return Ok(());
        };
        if root_scope_id != frame.root_scope_id {
            return Ok(());
        }
        if !self.should_reply_state_request(
            &frame.root_scope_id,
            reply_peer,
            std::time::Instant::now(),
        ) {
            return Ok(());
        }
        println!(
            "{}",
            json!({
                "event": "direct_root_state_request",
                "root_scope_id": frame.root_scope_id,
                "reply_peer": reply_peer,
            })
        );
        self.announce_state_request_reply(
            config_dir,
            &root_scope_id,
            sync.as_ref(),
            reply_peer,
        )
        .await
    }

    fn should_skip_seen_direct_root_frame(&mut self, key: &str, now: std::time::Instant) -> bool {
        if !self.seen_keys.contains(key) {
            return false;
        }
        if direct_root_retry_root_cid(key).is_none() {
            return true;
        }
        let repeat_interval =
            std::time::Duration::from_secs(DIRECT_ROOT_SEEN_FRAME_RETRY_INTERVAL_SECS);
        if self
            .seen_frame_retry_times
            .get(key)
            .is_some_and(|last| now.duration_since(*last) < repeat_interval)
        {
            return true;
        }
        self.seen_frame_retry_times.insert(key.to_string(), now);
        while self.seen_frame_retry_times.len() > DIRECT_ROOT_HINT_CACHE_MAX_ENTRIES {
            let Some(key) = self.seen_frame_retry_times.keys().next().cloned() else {
                break;
            };
            self.seen_frame_retry_times.remove(&key);
        }
        false
    }

    fn should_skip_recent_direct_root_hint(
        &mut self,
        source_peer: &str,
        key: &str,
        now: std::time::Instant,
    ) -> bool {
        let throttle_key = direct_root_hint_throttle_key(source_peer, key);
        let repeat_interval = std::time::Duration::from_secs(DIRECT_ROOT_HINT_REPEAT_INTERVAL_SECS);
        if self
            .recent_hint_times
            .get(&throttle_key)
            .is_some_and(|last| now.duration_since(*last) < repeat_interval)
        {
            return true;
        }
        self.recent_hint_times.insert(throttle_key, now);
        if self.recent_hint_times.len() > DIRECT_ROOT_HINT_CACHE_MAX_ENTRIES {
            self.recent_hint_times
                .retain(|_, last| now.duration_since(*last) < repeat_interval);
        }
        while self.recent_hint_times.len() > DIRECT_ROOT_HINT_CACHE_MAX_ENTRIES {
            let Some(key) = self.recent_hint_times.keys().next().cloned() else {
                break;
            };
            self.recent_hint_times.remove(&key);
        }
        false
    }

    pub(crate) async fn request_roots_from_new_peers(
        &mut self,
        config_dir: &Path,
        sync: Option<&FsFipsBlockSync>,
    ) -> Result<bool> {
        let Some(sync) = sync else {
            self.subscribed_streams.clear();
            return Ok(false);
        };
        let Ok(root_scope_id) = self.cached_profile_stream_root_scope_id(config_dir) else {
            return Ok(false);
        };
        if let Some(root_scope_id) = root_scope_id
            && self
                .subscribe_profile_stream(&root_scope_id, Some(sync))
                .await
        {
            let config = AppConfig::load_or_default_cached_profile(config_path_in(config_dir))?;
            if let Some(state) = config.profile.as_ref() {
                self.announce_current_state(config_dir, &config, state, Some(sync))
                    .await?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub(crate) async fn request_current_state_from_peers(
        &mut self,
        config_dir: &Path,
        sync: Option<&FsFipsBlockSync>,
        trigger: &'static str,
    ) -> Result<()> {
        let Some(sync) = sync else {
            self.subscribed_streams.clear();
            return Ok(());
        };
        let config = AppConfig::load_or_default_cached_profile(config_path_in(config_dir))?;
        let Some(state) = config.profile.as_ref() else {
            return Ok(());
        };
        let root_scope_id = state.root_scope_id();
        self.announce_current_state(config_dir, &config, state, Some(sync))
            .await?;
        let mut visible_peers = sync.connected_peer_ids().await.into_iter().collect::<BTreeSet<_>>();
        visible_peers.extend(sync.mesh_peer_ids().await);
        let now = std::time::Instant::now();
        if !self.should_publish_state_request(
            &root_scope_id,
            visible_peers.iter().map(String::as_str),
            now,
        ) {
            println!(
                "{}",
                json!({
                    "event": "direct_root_state_request_throttled",
                    "trigger": trigger,
                    "root_scope_id": root_scope_id.clone(),
                    "visible_peers": visible_peers.len(),
                })
            );
            return Ok(());
        }
        self.subscribe_profile_stream(&root_scope_id, Some(sync))
            .await;
        let bytes = iris_drive_core::encode_direct_root_state_request_frame(&root_scope_id)
            .context("encoding direct-root state request")?;
        let send_stats = send_direct_root_app_message_to_authorized_peers(sync, bytes.clone()).await;
        println!(
            "{}",
            json!({
                "event": "direct_root_state_request_publish",
                "trigger": trigger,
                "root_scope_id": root_scope_id.clone(),
                "selected_peers": send_stats.selected_peers,
                "visible_peers": visible_peers.len(),
                "sent_peers": send_stats.sent_peers,
                "failed_peers": send_stats.failed_peers,
            })
        );
        let stream = direct_root_mesh_stream(&root_scope_id);
        let seq = self.next_mesh_publish_seq();
        let publish_stats = sync.publish_mesh_pubsub(stream.clone(), seq, bytes).await;
        println!(
            "{}",
            json!({
                "event": "direct_root_state_request_mesh_publish",
                "trigger": trigger,
                "stream": stream,
                "seq": seq,
                "root_scope_id": root_scope_id.clone(),
                "selected_peers": publish_stats.selected_peers,
                "sent_peers": publish_stats.sent_peers,
                "sent_bytes": publish_stats.sent_bytes,
            })
        );
        Ok(())
    }

    fn cached_profile_stream_root_scope_id(&mut self, config_dir: &Path) -> Result<Option<String>> {
        let config_path = config_path_in(config_dir);
        let config_fingerprint = config_file_fingerprint(&config_path)?;
        self.cached_profile_stream_root_scope_id_from_config(config_fingerprint, || {
            Ok(AppConfig::load_or_default_cached_profile(&config_path)?)
        })
    }

    fn cached_profile_stream_root_scope_id_from_config(
        &mut self,
        config_fingerprint: ConfigFileFingerprint,
        load_config: impl FnOnce() -> Result<AppConfig>,
    ) -> Result<Option<String>> {
        if let Some(cached) = self
            .profile_stream_cache
            .as_ref()
            .filter(|cached| cached.config_fingerprint == config_fingerprint)
        {
            return Ok(cached.root_scope_id.clone());
        }
        let config = load_config()?;
        let root_scope_id = config.profile.as_ref().map(ProfileState::root_scope_id);
        self.profile_stream_cache = Some(CachedDirectRootProfileStream {
            config_fingerprint,
            root_scope_id: root_scope_id.clone(),
        });
        Ok(root_scope_id)
    }

    async fn cached_current_sync_events_from_config<F, Fut>(
        &mut self,
        config_fingerprint: ConfigFileFingerprint,
        build: F,
    ) -> Result<Vec<DirectRootEvent>>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<DirectRootEvent>>>,
    {
        if let Some(cached) = self
            .current_sync_events_cache
            .as_ref()
            .filter(|cached| cached.config_fingerprint == config_fingerprint)
        {
            return Ok(cached.events.clone());
        }
        let events = build().await?;
        self.current_sync_events_cache = Some(CachedCurrentSyncEvents {
            config_fingerprint,
            events: events.clone(),
        });
        Ok(events)
    }

    pub(crate) async fn handle_mesh_events(
        &mut self,
        client: &nostr_sdk::Client,
        config_dir: &Path,
        sync: Arc<FsFipsBlockSync>,
        mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
        daemon_tasks: &DaemonTaskSet,
        messages: Vec<FipsMeshPubsubEvent>,
    ) -> Result<bool> {
        let received_messages = messages.len();
        let (messages, skipped_roots) = iris_drive_core::coalesce_direct_root_mesh_events(messages);
        if skipped_roots > 0 {
            println!(
                "{}",
                json!({
                    "event": "direct_root_mesh_coalesced",
                    "received_messages": received_messages,
                    "applied_messages": messages.len(),
                    "skipped_roots": skipped_roots,
                })
            );
        }
        let mut should_announce = false;
        for message in messages {
            should_announce |= self
                .handle_mesh_event(
                    client,
                    config_dir,
                    sync.clone(),
                    mount_refresh.clone(),
                    daemon_tasks,
                    message,
                )
                .await?;
        }
        Ok(should_announce)
    }

    pub(crate) async fn handle_mesh_event(
        &mut self,
        client: &nostr_sdk::Client,
        config_dir: &Path,
        sync: Arc<FsFipsBlockSync>,
        mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
        daemon_tasks: &DaemonTaskSet,
        message: FipsMeshPubsubEvent,
    ) -> Result<bool> {
        if !message
            .stream_id
            .starts_with(DIRECT_ROOT_MESH_STREAM_PREFIX)
        {
            return Ok(false);
        }
        let frame = iris_drive_core::decode_direct_root_wire_frame(&message.payload)
            .context("parsing mesh root frame")?;
        let (root_key, root_event_id, frame_kind) = direct_root_wire_frame_log_fields(&frame);
        let outcome = match frame {
            DirectRootWireFrame::Full(frame) => {
                self.apply_direct_root_frame(
                    client,
                    config_dir,
                    sync,
                    mount_refresh,
                    daemon_tasks,
                    frame,
                    "mesh",
                    &message.origin_peer_id,
                )
                .await?
            }
            DirectRootWireFrame::Hint(frame) => {
                self.apply_direct_root_hint_frame(
                    config_dir,
                    sync,
                    mount_refresh,
                    daemon_tasks,
                    frame,
                    &message.origin_peer_id,
                )
                .await?
            }
            DirectRootWireFrame::Request(frame) => {
                self.handle_state_request_frame(config_dir, sync, frame, &message.origin_peer_id)
                    .await?;
                DirectRootFrameOutcome::Ignored
            }
        };
        if !outcome.should_log_event() {
            return Ok(false);
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
                "frame": frame_kind,
            })
        );
        Ok(outcome.should_schedule_announce())
    }

    pub(crate) async fn handle_app_message(
        &mut self,
        client: &nostr_sdk::Client,
        config_dir: &Path,
        sync: Arc<FsFipsBlockSync>,
        mount_refresh: Option<tokio::sync::mpsc::Sender<&'static str>>,
        daemon_tasks: &DaemonTaskSet,
        message: iris_drive_core::FipsAppMessage,
    ) -> Result<bool> {
        if message.topic != DIRECT_ROOT_APP_TOPIC {
            return Ok(false);
        }
        let frame = iris_drive_core::decode_direct_root_wire_frame(&message.data)
            .context("parsing app root frame")?;
        let (root_key, root_event_id, frame_kind) = direct_root_wire_frame_log_fields(&frame);
        let outcome = match frame {
            DirectRootWireFrame::Full(frame) => {
                self.apply_direct_root_frame(
                    client,
                    config_dir,
                    sync,
                    mount_refresh,
                    daemon_tasks,
                    frame,
                    "app",
                    &message.peer_id,
                )
                .await?
            }
            DirectRootWireFrame::Hint(frame) => {
                self.apply_direct_root_hint_frame(
                    config_dir,
                    sync,
                    mount_refresh,
                    daemon_tasks,
                    frame,
                    &message.peer_id,
                )
                .await?
            }
            DirectRootWireFrame::Request(frame) => {
                self.handle_state_request_frame(config_dir, sync, frame, &message.peer_id)
                    .await?;
                DirectRootFrameOutcome::Ignored
            }
        };
        if !outcome.should_log_event() {
            return Ok(false);
        }
        println!(
            "{}",
            json!({
                "event": "direct_root_app_event",
                "topic": message.topic,
                "peer": message.peer_id,
                "root_key": root_key,
                "root_event_id": root_event_id,
                "frame": frame_kind,
            })
        );
        Ok(outcome.should_schedule_announce())
    }

    fn cache_event(&mut self, event: DirectRootEvent) {
        self.seen_keys.insert(event.key.clone());
        if direct_root_retry_root_cid(&event.key).is_some() {
            self.seen_frame_retry_times
                .insert(event.key.clone(), std::time::Instant::now());
        }
        if !self.should_cache_event_as_latest(&event.key) {
            return;
        }
        for key in self.superseded_cached_event_keys(&event.key) {
            self.cached_events.remove(&key);
            self.published_keys.remove(&key);
            self.seen_frame_retry_times.remove(&key);
        }
        self.cached_events.insert(event.key.clone(), event);
        while self.cached_events.len() > DIRECT_ROOT_EVENT_CACHE_CAP {
            let Some(key) = self.cached_events.keys().next().cloned() else {
                break;
            };
            self.cached_events.remove(&key);
            self.published_keys.remove(&key);
            self.seen_frame_retry_times.remove(&key);
        }
    }

    fn event_for_publish(&self, event: DirectRootEvent) -> DirectRootEvent {
        self.cached_events.get(&event.key).cloned().unwrap_or(event)
    }

    fn events_for_publish(
        &self,
        local_events: Vec<DirectRootEvent>,
    ) -> Vec<DirectRootPublishEvent> {
        local_events
            .into_iter()
            .map(|event| DirectRootPublishEvent {
                event: self.event_for_publish(event),
                source: DirectRootPublishSource::LocalCurrent,
            })
            .collect()
    }

    fn state_request_events_for_publish(
        &self,
        local_events: Vec<DirectRootEvent>,
    ) -> Vec<DirectRootPublishEvent> {
        let mut events = Vec::with_capacity(local_events.len());
        for event in local_events {
            let event = self.event_for_publish(event);
            if direct_root_cache_slot(&event.key).is_some() {
                events.push(DirectRootPublishEvent {
                    event,
                    source: DirectRootPublishSource::StateRequestReply,
                });
            }
        }
        events
    }

    fn should_cache_event_as_latest(&self, incoming_key: &str) -> bool {
        let Some(incoming) = direct_root_cache_slot(incoming_key) else {
            return should_cache_unsequenced_direct_root_key(incoming_key);
        };
        !self.cached_events.keys().any(|key| {
            direct_root_cache_slot(key).is_some_and(|cached| {
                cached.family == incoming.family
                    && direct_root_slot_is_strictly_newer(&cached, &incoming)
            })
        })
    }

    fn superseded_cached_event_keys(&self, incoming_key: &str) -> Vec<String> {
        let Some(incoming) = direct_root_cache_slot(incoming_key) else {
            return Vec::new();
        };
        self.cached_events
            .keys()
            .filter(|key| {
                key.as_str() != incoming_key
                    && direct_root_cache_slot(key).is_some_and(|cached| {
                        cached.family == incoming.family
                            && !direct_root_slot_is_strictly_newer(&cached, &incoming)
                    })
            })
            .cloned()
            .collect()
    }

    #[cfg(test)]
    fn should_publish_key(&mut self, key: &str, now: std::time::Instant) -> bool {
        self.should_publish_candidate_key(key, DirectRootPublishSource::LocalCurrent, now)
    }

    #[cfg(test)]
    fn should_publish_candidate_key(
        &mut self,
        key: &str,
        source: DirectRootPublishSource,
        now: std::time::Instant,
    ) -> bool {
        self.should_publish_candidate_key_for_target(key, source, None, now)
    }

    fn should_publish_candidate_key_for_target(
        &mut self,
        key: &str,
        source: DirectRootPublishSource,
        target_peer: Option<&str>,
        now: std::time::Instant,
    ) -> bool {
        let throttle_key = direct_root_publish_throttle_key(key, source, target_peer);
        if self.published_keys.get(&throttle_key).is_some_and(|last| {
            now.duration_since(*last)
                < std::time::Duration::from_secs(direct_root_republish_interval_secs_for_source(
                    key, source,
                ))
        }) {
            return false;
        }
        self.published_keys.insert(throttle_key, now);
        true
    }

    fn should_publish_state_request<'a>(
        &mut self,
        root_scope_id: &str,
        visible_peers: impl IntoIterator<Item = &'a str>,
        now: std::time::Instant,
    ) -> bool {
        let mut throttle_keys = visible_peers
            .into_iter()
            .filter(|peer| !peer.is_empty())
            .map(|peer| format!("request:{peer}:{root_scope_id}"))
            .collect::<Vec<_>>();
        if throttle_keys.is_empty() {
            throttle_keys.push(format!("request:*:{root_scope_id}"));
        }
        let interval = std::time::Duration::from_secs(DIRECT_ROOT_STATE_REQUEST_INTERVAL_SECS);
        if throttle_keys.iter().all(|key| {
            self.state_request_times
                .get(key)
                .is_some_and(|last| now.duration_since(*last) < interval)
        }) {
            return false;
        }
        for key in throttle_keys {
            self.state_request_times.insert(key, now);
        }
        while self.state_request_times.len() > DIRECT_ROOT_EVENT_CACHE_CAP {
            let Some(key) = self.state_request_times.keys().next().cloned() else {
                break;
            };
            self.state_request_times.remove(&key);
        }
        true
    }

    fn should_reply_state_request(
        &mut self,
        root_scope_id: &str,
        reply_peer: &str,
        now: std::time::Instant,
    ) -> bool {
        let throttle_key = format!("reply:{reply_peer}:{root_scope_id}");
        if self
            .state_request_reply_times
            .get(&throttle_key)
            .is_some_and(|last| {
                now.duration_since(*last)
                    < std::time::Duration::from_secs(DIRECT_ROOT_STATE_REQUEST_INTERVAL_SECS)
            })
        {
            return false;
        }
        self.state_request_reply_times.insert(throttle_key, now);
        while self.state_request_reply_times.len() > DIRECT_ROOT_EVENT_CACHE_CAP {
            let Some(key) = self.state_request_reply_times.keys().next().cloned() else {
                break;
            };
            self.state_request_reply_times.remove(&key);
        }
        true
    }

    async fn refresh_known_mesh_peers(&mut self, sync: &FsFipsBlockSync) -> bool {
        let authorized_peers = sync.authorized_peer_ids().await;
        let mesh_peers = sync.mesh_peer_ids().await;
        let connected_peers = sync.connected_peer_ids().await;
        self.refresh_known_root_peers(authorized_peers, mesh_peers, connected_peers)
    }

    fn refresh_known_root_peers(
        &mut self,
        authorized_peers: impl IntoIterator<Item = String>,
        mesh_peers: impl IntoIterator<Item = String>,
        connected_peers: impl IntoIterator<Item = String>,
    ) -> bool {
        let publish_peers = authorized_peers.into_iter().collect::<BTreeSet<_>>();
        let connected_peers = connected_peers.into_iter().collect::<BTreeSet<_>>();
        let mesh_peers = mesh_peers.into_iter().collect::<BTreeSet<_>>();
        let visible_publish_peers = publish_peers
            .iter()
            .filter(|peer| connected_peers.contains(*peer) || mesh_peers.contains(*peer))
            .cloned()
            .collect::<BTreeSet<_>>();
        let mut root_peers = publish_peers.clone();
        root_peers.extend(mesh_peers.iter().map(|peer| format!("mesh:{peer}")));
        root_peers.extend(
            visible_publish_peers
                .iter()
                .map(|peer| format!("visible:{peer}")),
        );

        let peers_changed = root_peers != self.known_mesh_peers;
        if peers_changed {
            self.known_mesh_peers = root_peers;
        }
        let has_new_publish_peer = publish_peers
            .iter()
            .any(|peer| !self.known_publish_peers.contains(peer));
        let has_new_visible_publish_peer = visible_publish_peers
            .iter()
            .any(|peer| !self.known_visible_publish_peers.contains(peer));
        if has_new_publish_peer || has_new_visible_publish_peer {
            self.published_keys.clear();
        }
        self.known_publish_peers = publish_peers;
        self.known_visible_publish_peers = visible_publish_peers;
        peers_changed
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

pub(crate) async fn send_direct_root_app_message_to_authorized_peers(
    sync: &FsFipsBlockSync,
    bytes: Vec<u8>,
) -> DirectRootAppSendStats {
    let local_peer = sync.endpoint_npub().to_string();
    let peers = sync
        .authorized_peer_ids()
        .await
        .into_iter()
        .filter(|peer| !peer.is_empty() && peer != &local_peer)
        .collect::<Vec<_>>();
    let selected_peers = peers.len();
    let mut sent_peers = 0usize;
    let mut failed_peers = 0usize;
    for peer in peers {
        match sync
            .send_app_message(&peer, DIRECT_ROOT_APP_TOPIC, bytes.clone())
            .await
        {
            Ok(()) => sent_peers += 1,
            Err(_) => failed_peers += 1,
        }
    }
    DirectRootAppSendStats {
        selected_peers,
        sent_peers,
        failed_peers,
    }
}

fn direct_root_republish_interval_secs_for_source(
    key: &str,
    source: DirectRootPublishSource,
) -> u64 {
    if matches!(
        source,
        DirectRootPublishSource::StateRequestReply
    ) && direct_root_cache_slot(key).is_some()
    {
        return DIRECT_ROOT_STATE_REQUEST_REPLY_REPUBLISH_INTERVAL_SECS;
    }
    if direct_root_cache_slot(key).is_some() || key.starts_with("files-root:") {
        DIRECT_ROOT_REPUBLISH_INTERVAL_SECS
    } else {
        DIRECT_ROOT_METADATA_REPUBLISH_INTERVAL_SECS
    }
}

#[cfg(test)]
fn direct_root_publish_attempts(key: &str) -> usize {
    direct_root_publish_attempts_for_source(key, DirectRootPublishSource::LocalCurrent)
}

fn direct_root_publish_attempts_for_source(key: &str, source: DirectRootPublishSource) -> usize {
    if source == DirectRootPublishSource::StateRequestReply && direct_root_cache_slot(key).is_some()
    {
        return 4;
    }
    if source == DirectRootPublishSource::StateRequestReply {
        return 1;
    }
    if direct_root_cache_slot(key).is_some() {
        4
    } else if key.starts_with("files-root:") {
        2
    } else {
        1
    }
}

fn should_publish_direct_root_hint(key: &str, source: DirectRootPublishSource) -> bool {
    matches!(
        source,
        DirectRootPublishSource::LocalCurrent | DirectRootPublishSource::StateRequestReply
    ) && direct_root_cache_slot(key).is_some()
}

fn should_publish_direct_root_full_frame(
    key: &str,
    source: DirectRootPublishSource,
    attempt: usize,
) -> bool {
    if should_publish_direct_root_hint(key, source) {
        return match source {
            DirectRootPublishSource::LocalCurrent | DirectRootPublishSource::StateRequestReply => {
                attempt == 0
            }
        };
    }
    true
}

fn should_publish_targeted_direct_root_reply_over_mesh(source: DirectRootPublishSource) -> bool {
    matches!(source, DirectRootPublishSource::StateRequestReply)
}

fn direct_root_wire_frame_log_fields(
    frame: &DirectRootWireFrame,
) -> (String, String, &'static str) {
    match frame {
        DirectRootWireFrame::Full(frame) => (frame.key.clone(), frame.event_id.clone(), "full"),
        DirectRootWireFrame::Hint(frame) => (frame.key.clone(), frame.event_id.clone(), "hint"),
        DirectRootWireFrame::Request(frame) => {
            (frame.root_scope_id.clone(), String::new(), "request")
        }
    }
}

fn direct_root_hint_published_at() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            duration.as_secs().try_into().unwrap_or(i64::MAX)
        })
}

fn direct_root_hint_throttle_key(source_peer: &str, key: &str) -> String {
    let root_key = direct_root_cache_slot(key).map_or_else(|| key.to_string(), |slot| slot.family);
    format!("{source_peer}:{root_key}")
}

fn direct_root_publish_throttle_key(
    key: &str,
    source: DirectRootPublishSource,
    target_peer: Option<&str>,
) -> String {
    if source == DirectRootPublishSource::StateRequestReply && direct_root_cache_slot(key).is_some()
    {
        return target_peer.map_or_else(
            || format!("state-request:{key}"),
            |peer| format!("state-request:{peer}:{key}"),
        );
    }
    key.to_string()
}

pub(crate) async fn build_current_sync_events(
    config_dir: &Path,
    config: &AppConfig,
    state: &ProfileState,
) -> Result<Vec<DirectRootEvent>> {
    let mut events = Vec::new();

    append_primary_drive_root_events(&mut events, config_dir, config, state).await?;
    append_share_root_events(&mut events, config_dir, config, state).await?;
    append_profile_roster_events(&mut events, state)?;
    append_share_access_snapshot_events(&mut events, config_dir, config, state)?;

    Ok(events)
}

async fn build_current_sync_events_from_disk(
    config_dir: &Path,
    expected_root_scope_id: &str,
) -> Result<Vec<DirectRootEvent>> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    match config.profile.as_ref() {
        Some(state) if state.root_scope_id() == expected_root_scope_id => {
            build_current_sync_events(config_dir, &config, state).await
        }
        _ => Ok(Vec::new()),
    }
}

fn append_profile_roster_events(
    events: &mut Vec<DirectRootEvent>,
    state: &ProfileState,
) -> Result<()> {
    for op in &state.profile_roster_ops {
        let event =
            Event::from_json(&op.event_json).context("parsing NostrIdentity roster op event")?;
        events.push(direct_root_event(
            format!("profile-op:{}:{}", state.profile_id, op.op_id),
            &event,
        )?);
    }
    Ok(())
}

#[derive(Debug, Clone, Eq, PartialEq)]
struct DirectRootCacheSlot {
    family: String,
    seq: u64,
    recipient_count: usize,
}

fn direct_root_cache_slot(key: &str) -> Option<DirectRootCacheSlot> {
    let (prefix, rest) = key.split_once(':')?;
    match prefix {
        "drive-root" => {
            let mut parts = rest.splitn(4, ':');
            let app_key = parts.next()?;
            let drive_id = parts.next()?;
            let seq = parts.next()?.parse().ok()?;
            let root_and_recipients = parts.next()?;
            let (_root_cid, recipients) = root_and_recipients.rsplit_once(':')?;
            Some(DirectRootCacheSlot {
                family: format!("drive-root:{app_key}:{drive_id}"),
                seq,
                recipient_count: direct_root_recipient_count(recipients),
            })
        }
        "share-root" => {
            let mut parts = rest.splitn(4, ':');
            let share_id = parts.next()?;
            let app_key = parts.next()?;
            let seq = parts.next()?.parse().ok()?;
            let root_and_recipients = parts.next()?;
            let (_root_cid, recipients) = root_and_recipients.rsplit_once(':')?;
            Some(DirectRootCacheSlot {
                family: format!("share-root:{share_id}:{app_key}"),
                seq,
                recipient_count: direct_root_recipient_count(recipients),
            })
        }
        _ => None,
    }
}

fn direct_root_retry_root_cid(key: &str) -> Option<String> {
    let (prefix, rest) = key.split_once(':')?;
    match prefix {
        "drive-root" => {
            let mut parts = rest.splitn(4, ':');
            let _app_key = parts.next()?;
            let _drive_id = parts.next()?;
            let _seq = parts.next()?;
            let root_and_recipients = parts.next()?;
            let (root_cid, _recipients) = root_and_recipients.rsplit_once(':')?;
            Some(root_cid.to_string())
        }
        "share-root" => {
            let mut parts = rest.splitn(4, ':');
            let _share_id = parts.next()?;
            let _app_key = parts.next()?;
            let _seq = parts.next()?;
            let root_and_recipients = parts.next()?;
            let (root_cid, _recipients) = root_and_recipients.rsplit_once(':')?;
            Some(root_cid.to_string())
        }
        _ => None,
    }
}

fn direct_root_recipient_count(recipients: &str) -> usize {
    recipients
        .split(',')
        .filter(|recipient| !recipient.is_empty())
        .count()
}

fn direct_root_slot_is_newer(
    candidate: &DirectRootCacheSlot,
    current: &DirectRootCacheSlot,
) -> bool {
    candidate.seq > current.seq
        || (candidate.seq == current.seq && candidate.recipient_count > current.recipient_count)
}

fn direct_root_slot_is_strictly_newer(
    candidate: &DirectRootCacheSlot,
    current: &DirectRootCacheSlot,
) -> bool {
    direct_root_slot_is_newer(candidate, current)
}

fn should_cache_unsequenced_direct_root_key(key: &str) -> bool {
    !key.starts_with("drive-root:")
        && !key.starts_with("share-root:")
        && !key.starts_with("files-root:")
}

fn append_share_access_snapshot_events(
    events: &mut Vec<DirectRootEvent>,
    config_dir: &Path,
    config: &AppConfig,
    state: &ProfileState,
) -> Result<()> {
    let device = iris_drive_core::identity::AppKey::load(key_path_in(config_dir))
        .context("loading app key")?;
    for folder in &config.shared_folders {
        if !iris_drive_core::shared_folder_app_key_can_admin(folder, &state.app_key_pubkey) {
            continue;
        }
        let snapshot = iris_drive_core::sign_share_access_snapshot(
            device.keys(),
            folder,
            folder.access.updated_at,
        )?;
        let event = Event::from_json(&snapshot.event_json)
            .context("parsing share access snapshot event")?;
        events.push(direct_root_event(
            format!(
                "share-access:{}:{}:{}",
                folder.share_id, snapshot.snapshot_id, snapshot.content.updated_at
            ),
            &event,
        )?);
    }
    Ok(())
}

async fn append_primary_drive_root_events(
    events: &mut Vec<DirectRootEvent>,
    config_dir: &Path,
    config: &AppConfig,
    state: &ProfileState,
) -> Result<()> {
    if !state.can_write_roots() {
        return Ok(());
    }
    if let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID) {
        let Some(root) = publishable_app_key_root(config_dir, drive, state).await? else {
            return Ok(());
        };
        let authorized_app_keys =
            iris_drive_core::drive_root_recipient_app_key_pubkeys(state, drive);
        if authorized_app_keys.is_empty() {
            return Ok(());
        }
        ensure_publishable_root_locally_available(config_dir, config, &root.root_cid).await?;
        let device = iris_drive_core::identity::AppKey::load(key_path_in(config_dir))
            .context("loading app key")?;
        let event = iris_drive_core::nostr_events::build_drive_root_publish_event(
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
            let event = iris_drive_core::nostr_events::build_private_hashtree_root_event(
                device.keys(),
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
        if !iris_drive_core::shared_folder_app_key_can_write_roots(folder, &state.app_key_pubkey) {
            continue;
        }
        let Some(root) = folder.app_key_roots.get(&state.app_key_pubkey) else {
            continue;
        };
        if root.local_only {
            continue;
        }
        ensure_publishable_root_locally_available(config_dir, config, &root.root_cid).await?;
        let authorized_recipients = iris_drive_core::shared_folder_key_recipient_pubkeys(folder);
        let event = iris_drive_core::nostr_events::build_drive_root_publish_event(
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
    config: &AppConfig,
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
        match collect_local_root_hashes(config_dir, config, &root_cid).await {
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

async fn collect_local_root_hashes(
    config_dir: &Path,
    config: &AppConfig,
    root: &Cid,
) -> Result<usize> {
    let daemon = Daemon::open_with_config(config_dir, config.clone())
        .context("opening daemon for local root availability")?;
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
