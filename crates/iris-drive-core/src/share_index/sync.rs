//! Keeping a device's list and folders in step with the profile's other
//! devices. The desktop daemon and the mobile sync both come through here.
//!
//! Every change to the list file happens under the config transaction lock,
//! so the daemon, the apps and the CLI never write over each other.

use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use nostr_sdk::{Client, Event};

use super::event::{
    ShareIndexEventError, current_share_index_events, is_share_index_event, mark_published,
    parse_share_index_event, share_index_d_tag, share_index_event_scope, share_index_filter,
};
use super::{ShareIndex, admit::admit_profile_devices};
use crate::config::AppConfig;
use crate::config_lock::ConfigMutationLock;
use crate::paths::{config_path_in, key_path_in};
use crate::share_join::ShareJoinStore;
use crate::{AppKey, NostrIdentityId};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareIndexApply {
    /// Not a list, or the list of another profile.
    NotOurs,
    /// Signed by a key that is not a device of this profile.
    Unauthorized,
    /// Sent before this device was linked; its sender sends it again.
    KeyUnavailable,
    /// Nothing in it is newer than what this device has.
    Current,
    Applied,
}

fn now_seconds() -> i64 {
    i64::try_from(nostr_sdk::Timestamp::now().as_secs()).unwrap_or(i64::MAX)
}

/// Take in the list another device of this profile sent: merge it, drop the
/// folders the profile removed, and put the others where the list says. A
/// folder this device lacks is added later, from its admin's access list.
pub async fn apply_share_index_event(config_dir: &Path, event: &Event) -> Result<ShareIndexApply> {
    if !is_share_index_event(event) {
        return Ok(ShareIndexApply::NotOurs);
    }
    let lock = ConfigMutationLock::acquire(config_dir).await?;
    let outcome = apply_share_index_event_locked(config_dir, event);
    drop(lock);
    outcome
}

/// As [`apply_share_index_event`], for a caller that already holds the
/// config transaction lock.
pub fn apply_share_index_event_locked(config_dir: &Path, event: &Event) -> Result<ShareIndexApply> {
    if !is_share_index_event(event) {
        return Ok(ShareIndexApply::NotOurs);
    }
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config.profile.clone().context("Profile required")?;
    let scope = state.root_scope_id();
    // Any part of the list, however its sender split it.
    let first_part = share_index_d_tag(&scope, 0);
    if share_index_event_scope(event) != Some(scope.as_str())
        || !event.tags.identifier().is_some_and(|d_tag| {
            d_tag == first_part
                || d_tag
                    .strip_prefix(first_part.as_str())
                    .is_some_and(|part| part.starts_with('/'))
        })
    {
        return Ok(ShareIndexApply::NotOurs);
    }
    let author = event.pubkey.to_hex();
    if author == state.app_key_pubkey {
        return Ok(ShareIndexApply::Current);
    }
    if !state.can_write_roots_for_app_key(&author) {
        return Ok(ShareIndexApply::Unauthorized);
    }
    let app_key = AppKey::load(key_path_in(config_dir)).context("loading app key")?;
    let entries = match parse_share_index_event(event, app_key.keys()) {
        Ok(entries) => entries,
        Err(ShareIndexEventError::KeyUnavailable) => return Ok(ShareIndexApply::KeyUnavailable),
        Err(error) => return Err(error.into()),
    };
    let mut index = ShareIndex::load_for(config_dir, &config)?;
    // Where this device's folders sit first, so its own move is neither lost
    // nor mistaken for the other device's.
    let mut index_changed = index.capture_local(&config, now_seconds());
    let removed_before = removed_share_ids(&index);
    index_changed |= index.merge(entries);
    let config_changed = index.apply_to_config(&mut config);
    // A folder the profile just removed is neither waited for nor answered
    // for. One removed earlier is left alone: this device may be joining it
    // again.
    let mut store = ShareJoinStore::load(config_dir)?;
    let mut store_changed = false;
    for share_id in removed_share_ids(&index).difference(&removed_before) {
        store_changed |= store.finish_pending(*share_id);
        store_changed |= store.disable_links(*share_id);
    }
    if config_changed {
        config.save(config_path_in(config_dir))?;
    }
    if store_changed {
        store.save(config_dir)?;
    }
    if index_changed || config_changed {
        index.save(config_dir)?;
    }
    Ok(if index_changed || config_changed {
        ShareIndexApply::Applied
    } else {
        ShareIndexApply::Current
    })
}

fn removed_share_ids(index: &ShareIndex) -> BTreeSet<NostrIdentityId> {
    index
        .entries
        .values()
        .filter(|entry| entry.removed)
        .map(|entry| entry.share_id)
        .collect()
}

/// Send the parts of this device's list that relays do not have yet (or all
/// of them again, with `force`). Returns whether anything was sent.
pub async fn publish_share_index(client: &Client, config_dir: &Path, force: bool) -> Result<bool> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let events = current_share_index_events(config_dir, &config)?;
    if events.is_empty() || client.relays().await.is_empty() {
        return Ok(false);
    }
    let mut sent = false;
    for (event, published) in &events {
        if *published && !force {
            continue;
        }
        client
            .send_event(event)
            .await
            .map_err(|error| anyhow::anyhow!("sending shared folder list: {error}"))?;
        mark_published(config_dir, event)?;
        sent = true;
    }
    Ok(sent)
}

/// What one reconciliation did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShareIndexReconcile {
    /// The list or this device's folders changed to agree with each other.
    pub captured: bool,
    /// Folders the profile's newly linked devices were let into.
    pub admitted: Vec<NostrIdentityId>,
    /// The list was sent to relays.
    pub published: bool,
}

impl ShareIndexReconcile {
    /// Whether the profile's other devices have something new to hear.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.captured || !self.admitted.is_empty()
    }
}

/// Bring this device's list and folders into agreement, and let the
/// profile's new devices into the folders it answers for.
async fn capture_and_admit(config_dir: &Path) -> Result<ShareIndexReconcile> {
    let lock = ConfigMutationLock::acquire(config_dir).await?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    if config.profile.is_none() {
        return Ok(ShareIndexReconcile::default());
    }
    let mut index = ShareIndex::load_for(config_dir, &config)?;
    let now = now_seconds();
    let captured = index.capture_local(&config, now);
    // A folder the profile left that is back in the config was put there by
    // a stale copy of it, not by the person: it goes again.
    let healed = index.apply_to_config(&mut config);
    let admitted = if config.shared_folders.is_empty() {
        Vec::new()
    } else {
        let app_key = AppKey::load(key_path_in(config_dir)).context("loading app key")?;
        admit_profile_devices(&mut config, &index, app_key.keys(), now)?
    };
    if healed || !admitted.is_empty() {
        config.save(config_path_in(config_dir))?;
    }
    if captured || healed {
        index.save(config_dir)?;
    }
    drop(lock);
    Ok(ShareIndexReconcile {
        captured: captured || healed,
        admitted,
        published: false,
    })
}

/// Bring the list up to date with this device and the profile's devices,
/// then publish what changed: the folders new devices were let into, and
/// the list itself.
pub async fn reconcile_share_index(
    config_dir: &Path,
    client: &Client,
    force_publish: bool,
) -> Result<ShareIndexReconcile> {
    let mut report = capture_and_admit(config_dir).await?;
    for share_id in &report.admitted {
        crate::shared_content::prepare_shared_folder_roots(config_dir).await?;
        crate::share_actions::publish_share_content_to(config_dir, *share_id, client).await?;
    }
    report.published = publish_share_index(client, config_dir, force_publish).await?;
    Ok(report)
}

/// One pass for apps without a long-running daemon: take in the lists of the
/// profile's other devices, then reconcile and publish this device's own.
pub async fn sync_share_index_once(
    config_dir: &Path,
    client: &Client,
    timeout: Duration,
) -> Result<usize> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config.profile.as_ref().context("Profile required")?;
    let events = crate::relay_sync::fetch_events(
        client,
        vec![share_index_filter(&state.root_scope_id())],
        timeout,
    )
    .await?;
    let mut applied = 0;
    for event in &events {
        // A list that does not check out is skipped, never fatal to sync.
        if matches!(
            apply_share_index_event(config_dir, event).await,
            Ok(ShareIndexApply::Applied)
        ) {
            applied += 1;
        }
    }
    reconcile_share_index(config_dir, client, false).await?;
    Ok(applied)
}
