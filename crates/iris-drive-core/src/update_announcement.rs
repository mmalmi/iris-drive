//! Signed product-update announcements over the existing Iris Drive FIPS mesh.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::{Duration, Instant};

use hashtree_core::Store;
use hashtree_updater::{UpdateEventCache, UpdateRef};
use nostr_sdk::{Event, JsonUtil};

use crate::atomic_file::atomic_write;
use crate::paths::update_announcement_path_in;
use crate::{FipsBlockSync, FipsNostrPubsubEvent};

const UPDATE_REPUBLISH_INTERVAL: Duration = Duration::from_secs(30);

/// Bridges the updater's verified event cache to Iris Drive's existing FIPS
/// mesh. It owns no endpoint and deliberately has no legacy-manifest path.
pub struct UpdateAnnouncementExchange {
    cache: UpdateEventCache,
    known_peers: BTreeSet<String>,
    last_publish: Option<Instant>,
}

impl UpdateAnnouncementExchange {
    pub fn load(config_dir: &Path) -> Result<Self, String> {
        let reference = crate::updater::product_update_reference(None)
            .map_err(|error| format!("parsing update reference: {error:#}"))?;
        let cache = load_update_event_cache(config_dir, &reference).unwrap_or_else(|error| {
            tracing::warn!(error, "ignoring invalid cached update announcement");
            UpdateEventCache::new(&reference).expect("already validated update reference")
        });
        Ok(Self::from_cache(cache))
    }

    #[cfg(test)]
    pub(crate) fn load_for_reference(
        config_dir: &Path,
        reference: &UpdateRef,
    ) -> Result<Self, String> {
        Ok(Self::from_cache(load_update_event_cache(
            config_dir, reference,
        )?))
    }

    fn from_cache(cache: UpdateEventCache) -> Self {
        Self {
            cache,
            known_peers: BTreeSet::new(),
            last_publish: None,
        }
    }

    /// Refresh the local subscription and replay the newest signed root when
    /// a peer appears or periodically, covering peers that subscribe late.
    pub async fn sync_with_peers<L>(&mut self, config_dir: &Path, sync: &FipsBlockSync<L>) -> bool
    where
        L: Store + Send + Sync + 'static,
    {
        let disk_advanced = self.refresh_from_disk(config_dir).unwrap_or_else(|error| {
            tracing::warn!(error, "ignoring invalid cached update announcement");
            false
        });
        let peers = sync.connected_peer_ids().await.into_iter().collect();
        let peers_changed = peers != self.known_peers;
        self.known_peers = peers;

        let periodic_replay = self
            .last_publish
            .is_none_or(|last| last.elapsed() >= UPDATE_REPUBLISH_INTERVAL);
        if disk_advanced || peers_changed || periodic_replay {
            return self.publish_cached(sync).await;
        }
        false
    }

    /// Accept only a valid, newer release-root event and persist exactly that
    /// event for updater checks and late-peer replay.
    pub fn handle_nostr_event(
        &mut self,
        config_dir: &Path,
        message: &FipsNostrPubsubEvent,
    ) -> Result<bool, String> {
        self.ingest_event(config_dir, message.event.clone())
    }

    pub(crate) fn ingest_event(&mut self, config_dir: &Path, event: Event) -> Result<bool, String> {
        let advanced = self
            .cache
            .ingest_event(event)
            .map_err(|error| error.to_string())?;
        if advanced {
            persist_update_event_cache(config_dir, &self.cache)?;
        }
        Ok(advanced)
    }

    #[cfg(test)]
    pub(crate) fn latest_event(&self) -> Option<&Event> {
        self.cache
            .latest()
            .map(nostr_pubsub::VerifiedEvent::as_event)
    }

    async fn publish_cached<L>(&mut self, sync: &FipsBlockSync<L>) -> bool
    where
        L: Store + Send + Sync + 'static,
    {
        let Some(event) = self.cache.latest() else {
            return false;
        };
        if let Err(error) = sync.publish_nostr_event(event.as_event().clone()).await {
            tracing::debug!(%error, "Drive update announcement had no FIPS pubsub route");
        }
        self.last_publish = Some(Instant::now());
        true
    }

    fn refresh_from_disk(&mut self, config_dir: &Path) -> Result<bool, String> {
        let path = update_announcement_path_in(config_dir);
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(format!("reading {}: {error}", path.display())),
        };
        let event = Event::from_json(bytes)
            .map_err(|error| format!("parsing {}: {error}", path.display()))?;
        self.cache
            .ingest_event(event)
            .map_err(|error| format!("validating {}: {error}", path.display()))
    }
}

pub(crate) fn load_update_event_cache(
    config_dir: &Path,
    reference: &UpdateRef,
) -> Result<UpdateEventCache, String> {
    let mut cache = UpdateEventCache::new(reference).map_err(|error| error.to_string())?;
    let path = update_announcement_path_in(config_dir);
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(cache),
        Err(error) => return Err(format!("reading {}: {error}", path.display())),
    };
    let event =
        Event::from_json(bytes).map_err(|error| format!("parsing {}: {error}", path.display()))?;
    cache
        .ingest_event(event)
        .map_err(|error| format!("validating {}: {error}", path.display()))?;
    Ok(cache)
}

pub(crate) fn persist_update_event_cache(
    config_dir: &Path,
    cache: &UpdateEventCache,
) -> Result<(), String> {
    let Some(event) = cache.latest() else {
        return Ok(());
    };
    let path = update_announcement_path_in(config_dir);
    atomic_write(&path, event.as_event().as_json().as_bytes())
        .map_err(|error| format!("writing {}: {error}", path.display()))
}
