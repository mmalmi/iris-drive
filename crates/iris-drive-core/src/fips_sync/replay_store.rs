//! Peer history Drive serves from local stores instead of its bounded live
//! cache: cached peer announcements and the newest signed release head.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use nostr_pubsub::{
    EventBus, EventRetentionPolicy, EventSource, FileEventBus, Filter, PublishReport, PubsubError,
    QueryEvent, QueryOptions, QueryReport, RetainedEvents, SOURCE_PRIORITY_LOCAL_INDEX,
    VerifiedEvent,
};
use nostr_sdk::{Kind, Timestamp};

/// Hashtree mutable roots, including signed release heads.
const HASHTREE_ROOT_KIND: Kind = Kind::Custom(30_064);
/// Trusted release trees a device subscribes to; each keeps only its newest head.
const MAX_RELEASE_TREES: usize = 8;

/// The newest head for each author-scoped hashtree root filter this device
/// subscribes to. Live traffic cannot evict it, so any peer's request is
/// answered for as long as this device runs. The update exchange validates
/// each head and republishes it from disk after a restart.
pub(super) struct ReleaseHeads {
    heads: Mutex<RetainedEvents>,
}

impl ReleaseHeads {
    pub(super) fn for_subscriptions(filters: &[Filter]) -> Option<Self> {
        let filters = filters
            .iter()
            .filter(|filter| {
                filter.authors.as_ref().is_some_and(|a| !a.is_empty())
                    && filter
                        .kinds
                        .as_ref()
                        .is_some_and(|kinds| kinds.contains(&HASHTREE_ROOT_KIND))
            })
            .cloned()
            .collect::<Vec<_>>();
        let policy = EventRetentionPolicy::new(MAX_RELEASE_TREES, filters);
        RetainedEvents::new(policy).ok().map(|heads| Self {
            heads: Mutex::new(heads),
        })
    }

    /// Keep the event if it is a newer head for a subscribed release tree.
    pub(super) fn retain(&self, event: &VerifiedEvent) -> bool {
        self.heads
            .lock()
            .is_ok_and(|mut heads| heads.insert(event.clone(), Timestamp::now().as_secs()))
    }

    fn query(&self, filters: &[Filter], options: QueryOptions) -> Vec<VerifiedEvent> {
        self.heads
            .lock()
            .map(|heads| heads.query(filters, options, Timestamp::now().as_secs()))
            .unwrap_or_default()
    }
}

/// Read-only replay source for the FIPS pubsub client. The runtime owns
/// ingestion into each store.
pub(super) struct DriveReplaySource {
    pub(super) announcements: Option<Arc<FileEventBus>>,
    pub(super) release_heads: Option<Arc<ReleaseHeads>>,
}

#[async_trait]
impl EventBus for DriveReplaySource {
    async fn publish(
        &self,
        _: VerifiedEvent,
        _: EventSource,
    ) -> nostr_pubsub::Result<PublishReport> {
        Err(PubsubError::Validation(
            "Drive replay source is read-only".into(),
        ))
    }

    async fn query(
        &self,
        filters: Vec<Filter>,
        options: QueryOptions,
    ) -> nostr_pubsub::Result<QueryReport> {
        let mut report = match &self.announcements {
            Some(store) => store.query(filters.clone(), options).await?,
            None => QueryReport::default(),
        };
        if let Some(heads) = &self.release_heads {
            report
                .events
                .extend(
                    heads
                        .query(&filters, options)
                        .into_iter()
                        .map(|event| QueryEvent {
                            event,
                            source: EventSource::local_index("release-heads"),
                            priority: SOURCE_PRIORITY_LOCAL_INDEX,
                        }),
                );
        }
        Ok(report)
    }
}
