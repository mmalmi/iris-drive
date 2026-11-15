//! Relay subscriptions that follow the folders a device shares, joins, and
//! hands out links to. They change while Drive runs: a folder joined from a
//! link, or a new link, must be heard without restarting.

use std::collections::BTreeMap;

use nostr_sdk::{Client, Filter, SubscriptionId};

use super::{ShareJoinStore, share_join_filters};
use crate::config::AppConfig;
use crate::relay_filters::{drive_root_filter, share_access_snapshot_filter};
use crate::relay_sync::RelayError;

const SHARE_SUBSCRIPTION_PREFIX: &str = "iris-drive-share-";

/// Everything shared folders, their links and pending joins need from relays.
#[must_use]
pub fn share_relay_filters(config: &AppConfig, store: &ShareJoinStore) -> Vec<Filter> {
    let root_scope = config
        .profile
        .as_ref()
        .map(crate::ProfileState::root_scope_id);
    let mut filters = Vec::new();
    for folder in &config.shared_folders {
        let scope = folder.share_id.to_string();
        if root_scope.as_deref() == Some(scope.as_str()) {
            continue;
        }
        filters.push(share_access_snapshot_filter(folder.share_id));
        filters.push(drive_root_filter(&scope, crate::PRIMARY_DRIVE_ID));
    }
    filters.extend(share_join_filters(config, store));
    filters
}

/// Filters already open on the client, keyed by the subscription that owns
/// them; `None` marks one opened elsewhere (at startup) that is left alone.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShareRelaySubscriptionState {
    open: BTreeMap<String, Option<SubscriptionId>>,
}

impl ShareRelaySubscriptionState {
    /// Filters the caller has already subscribed to.
    #[must_use]
    pub fn opened_elsewhere(filters: &[Filter]) -> Self {
        Self {
            open: filters
                .iter()
                .map(|filter| (filter_key(filter), None))
                .collect(),
        }
    }
}

fn filter_key(filter: &Filter) -> String {
    serde_json::to_string(filter).unwrap_or_default()
}

/// Open subscriptions for new filters and close those no longer needed.
/// Returns whether anything changed.
pub async fn refresh_share_relay_subscriptions(
    client: &Client,
    filters: &[Filter],
    state: &mut ShareRelaySubscriptionState,
) -> Result<bool, RelayError> {
    if client.relays().await.is_empty() {
        return Ok(false);
    }
    let wanted = filters
        .iter()
        .map(|filter| (filter_key(filter), filter))
        .collect::<BTreeMap<_, _>>();
    let mut changed = false;
    let unwanted = state
        .open
        .keys()
        .filter(|key| !wanted.contains_key(*key))
        .cloned()
        .collect::<Vec<_>>();
    for key in unwanted {
        if let Some(Some(id)) = state.open.remove(&key) {
            client.unsubscribe(&id).await;
        }
        changed = true;
    }
    for (key, filter) in wanted {
        if state.open.contains_key(&key) {
            continue;
        }
        let digest = hashtree_core::content_hash(key.as_bytes());
        let id = SubscriptionId::new(format!(
            "{SHARE_SUBSCRIPTION_PREFIX}{}",
            hex::encode(&digest[..8])
        ));
        client
            .subscribe_with_id(id.clone(), filter.clone(), None)
            .await
            .map_err(|error| RelayError::Client(error.to_string()))?;
        state.open.insert(key, Some(id));
        changed = true;
    }
    Ok(changed)
}
