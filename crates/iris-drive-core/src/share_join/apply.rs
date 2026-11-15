//! Acting on join requests (as the folder's admin) and on the access lists
//! that answer them (as the person who asked). Both the desktop daemon and
//! the mobile sync pass relay events through here.

use std::path::Path;

use anyhow::{Context, Result};
use nostr_sdk::{Alphabet, Client, Event, Filter, Kind, SingleLetterTag};

use super::store::ShareJoinStore;
use super::{
    ShareJoinLink, build_share_join_request, is_share_join_request_event,
    join_request_needs_approval, share_join_request_filter, share_recipient_from_join_request,
    shared_folder_from_access_event_for_join,
};
use crate::config::AppConfig;
use crate::paths::{config_path_in, key_path_in};
use crate::sharing::{
    invite_shared_folder_resolved_recipient, share_access_snapshot_d_tag,
    share_recipient_profile_evidence_for_app_key, shared_folder_app_key_can_admin,
};
use crate::{
    AppKey, KIND_SHARE_ACCESS_SNAPSHOT, NostrIdentityId, Profile, SHARE_ACCESS_LABEL,
    is_share_access_snapshot_event_coordinate,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareJoinEventOutcome {
    /// Someone who asked with one of this device's links was let in.
    Approved(NostrIdentityId),
    /// A folder this device asked to join let it in.
    Joined(NostrIdentityId),
    Ignored,
}

fn now_seconds() -> i64 {
    i64::try_from(nostr_sdk::Timestamp::now().as_secs()).unwrap_or(i64::MAX)
}

/// Let in the person behind a request made with one of this device's links.
pub fn approve_share_join_request(
    config_dir: &Path,
    config: &mut AppConfig,
    store: &ShareJoinStore,
    event: &Event,
    now: i64,
) -> Result<Option<NostrIdentityId>> {
    let Some(link) = store.link_signed_by(&event.pubkey.to_hex()) else {
        return Ok(None);
    };
    let app_key = AppKey::load(key_path_in(config_dir)).context("loading app key")?;
    let admin = app_key.keys().public_key().to_hex();
    let Some(folder) = config
        .shared_folders
        .iter_mut()
        .find(|folder| folder.share_id == link.share_id)
    else {
        return Ok(None);
    };
    if link.admin != admin || !shared_folder_app_key_can_admin(folder, &admin) {
        return Ok(None);
    }
    let (resolved, requester) = share_recipient_from_join_request(link, event)?;
    if !join_request_needs_approval(folder, resolved.profile_id, &requester) {
        return Ok(None);
    }
    invite_shared_folder_resolved_recipient(folder, app_key.keys(), &resolved, link.role, now)?;
    Ok(Some(folder.share_id))
}

/// Keep a folder this device asked to join, once its admin's access list
/// includes this device.
pub fn complete_share_join(
    config: &mut AppConfig,
    store: &mut ShareJoinStore,
    app_pubkey: &str,
    event: &Event,
) -> Result<Option<NostrIdentityId>> {
    let Some(pending) = store.pending.iter().find(|pending| {
        pending.link.admin == event.pubkey.to_hex()
            && event.tags.identifier()
                == Some(share_access_snapshot_d_tag(pending.link.share_id).as_str())
    }) else {
        return Ok(None);
    };
    let share_id = pending.link.share_id;
    if config.shared_folder(share_id).is_some() {
        store.finish_pending(share_id);
        return Ok(None);
    }
    let Some(folder) = shared_folder_from_access_event_for_join(&pending.link, event, app_pubkey)
    else {
        return Ok(None);
    };
    let name = folder.display_name.clone();
    config.upsert_shared_folder(folder);
    crate::share_actions::ensure_default_share_shortcut(config, share_id, &name)?;
    store.finish_pending(share_id);
    Ok(Some(share_id))
}

/// Whether the event is one this module acts on.
#[must_use]
pub fn is_share_join_event(event: &Event, store: &ShareJoinStore) -> bool {
    if is_share_join_request_event(event) {
        return store.link_signed_by(&event.pubkey.to_hex()).is_some();
    }
    is_share_access_snapshot_event_coordinate(event)
        && store.pending.iter().any(|pending| {
            event.tags.identifier()
                == Some(share_access_snapshot_d_tag(pending.link.share_id).as_str())
        })
}

/// Apply one relay event under the config lock. After `Approved`, the caller
/// publishes the folder with [`publish_approved_share`].
pub async fn apply_share_join_event(
    config_dir: &Path,
    event: &Event,
) -> Result<ShareJoinEventOutcome> {
    let lock = crate::config_lock::ConfigMutationLock::acquire(config_dir).await?;
    let mut store = ShareJoinStore::load(config_dir)?;
    if !is_share_join_event(event, &store) {
        return Ok(ShareJoinEventOutcome::Ignored);
    }
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let outcome = if is_share_join_request_event(event) {
        approve_share_join_request(config_dir, &mut config, &store, event, now_seconds())?.map_or(
            ShareJoinEventOutcome::Ignored,
            ShareJoinEventOutcome::Approved,
        )
    } else {
        let app_pubkey = config
            .profile
            .as_ref()
            .context("Profile required")?
            .app_key_pubkey
            .clone();
        let joined = complete_share_join(&mut config, &mut store, &app_pubkey, event)?;
        store.save(config_dir)?;
        joined.map_or(
            ShareJoinEventOutcome::Ignored,
            ShareJoinEventOutcome::Joined,
        )
    };
    if outcome != ShareJoinEventOutcome::Ignored {
        config.save(config_path_in(config_dir))?;
    }
    drop(lock);
    Ok(outcome)
}

/// Publish a folder's new access list and its files, wrapped for everyone
/// now in it, so the person just let in can open it.
pub async fn publish_approved_share(config_dir: &Path, share_id: NostrIdentityId) -> Result<()> {
    crate::share_actions::publish_share_content(config_dir, share_id).await
}

/// Relay filters for requests made with this device's links and for the
/// access lists of folders it asked to join.
#[must_use]
pub fn share_join_filters(config: &AppConfig, store: &ShareJoinStore) -> Vec<Filter> {
    let mut filters = Vec::new();
    if let Some(profile) = config.profile.as_ref() {
        let link_pubkeys = store
            .links
            .iter()
            .filter(|link| link.admin == profile.app_key_pubkey)
            .filter(|link| config.shared_folder(link.share_id).is_some())
            .filter_map(|link| link.pubkey().ok())
            .collect::<Vec<_>>();
        filters.extend(share_join_request_filter(&link_pubkeys));
    }
    let pending = store
        .pending
        .iter()
        .filter(|pending| config.shared_folder(pending.link.share_id).is_none())
        .collect::<Vec<_>>();
    if !pending.is_empty() {
        let admins = pending
            .iter()
            .filter_map(|pending| nostr_sdk::PublicKey::from_hex(&pending.link.admin).ok())
            .collect::<Vec<_>>();
        filters.push(
            Filter::new()
                .kind(Kind::from(KIND_SHARE_ACCESS_SNAPSHOT))
                .authors(admins)
                .custom_tag(SingleLetterTag::lowercase(Alphabet::L), SHARE_ACCESS_LABEL)
                .custom_tags(
                    SingleLetterTag::lowercase(Alphabet::D),
                    pending
                        .iter()
                        .map(|pending| share_access_snapshot_d_tag(pending.link.share_id)),
                ),
        );
    }
    filters
}

/// Ask to join with `link`, or note that this device is already in.
pub fn record_share_join(
    config_dir: &Path,
    config: &AppConfig,
    link: ShareJoinLink,
    now: i64,
) -> Result<bool> {
    if config.shared_folder(link.share_id).is_some() {
        return Ok(false);
    }
    let mut store = ShareJoinStore::load(config_dir)?;
    store.record_pending(link, now);
    store.save(config_dir)?;
    Ok(true)
}

/// Signed requests for every folder this device is still waiting to join.
/// Sending one again replaces the earlier copy on relays.
pub fn pending_share_join_requests(config_dir: &Path) -> Result<Vec<Event>> {
    let store = ShareJoinStore::load(config_dir)?;
    if store.pending.is_empty() {
        return Ok(Vec::new());
    }
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config.profile.clone().context("Profile required")?;
    let display_name = config
        .user_profile
        .as_ref()
        .map(|profile| profile.username.trim().to_owned())
        .filter(|name| !name.is_empty())
        .or_else(|| state.app_key_label.clone());
    let profile = Profile::load(state, config_dir)?;
    let mut events = Vec::new();
    for pending in &store.pending {
        if config.shared_folder(pending.link.share_id).is_some() {
            continue;
        }
        let evidence = share_recipient_profile_evidence_for_app_key(
            profile.state.profile_id,
            &profile.state.profile_roster_ops,
            profile.app_key.keys(),
            display_name.clone(),
            pending.requested_at,
        )?;
        events.push(build_share_join_request(
            &pending.link,
            &evidence,
            pending.requested_at,
        )?);
    }
    Ok(events)
}

/// Send the pending requests. Relays keep them, so an admin who is away sees
/// them when their Drive comes back.
pub async fn publish_pending_share_join_requests(
    client: &Client,
    config_dir: &Path,
) -> Result<usize> {
    let events = pending_share_join_requests(config_dir)?;
    for event in &events {
        client
            .send_event(event)
            .await
            .map_err(|error| anyhow::anyhow!("sending join request: {error}"))?;
    }
    Ok(events.len())
}

/// What one pass over join events did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShareJoinSyncReport {
    pub requests_sent: usize,
    pub approved: Vec<NostrIdentityId>,
    pub joined: Vec<NostrIdentityId>,
}

/// One pass for apps without a long-running daemon (mobile sync): send
/// pending requests, let in whoever asked with this device's links, and keep
/// folders whose admin let this device in.
pub async fn sync_share_joins_once(
    config_dir: &Path,
    client: &Client,
    timeout: std::time::Duration,
) -> Result<ShareJoinSyncReport> {
    let mut report = ShareJoinSyncReport {
        requests_sent: publish_pending_share_join_requests(client, config_dir).await?,
        ..ShareJoinSyncReport::default()
    };
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let filters = share_join_filters(&config, &ShareJoinStore::load(config_dir)?);
    if filters.is_empty() {
        return Ok(report);
    }
    let events = crate::relay_sync::fetch_events(client, filters, timeout).await?;
    for event in &events {
        // A request that does not check out is skipped, never fatal to sync.
        match apply_share_join_event(config_dir, event).await {
            Ok(ShareJoinEventOutcome::Approved(share_id)) => report.approved.push(share_id),
            Ok(ShareJoinEventOutcome::Joined(share_id)) => report.joined.push(share_id),
            Ok(ShareJoinEventOutcome::Ignored) | Err(_) => {}
        }
    }
    report.approved.sort();
    report.approved.dedup();
    for share_id in &report.approved {
        publish_approved_share(config_dir, *share_id).await?;
    }
    Ok(report)
}
