use std::collections::HashMap;

use hashtree_core::{Cid, HashTree, LinkType, Store};
use serde_json::{Value, json};

use super::{
    CALENDAR_DATA_FILE, CALENDAR_TREE_NAME, CalendarData, CalendarError, CalendarEvent,
    event_from_ics, event_href_id, now_millis, projection, replica, replica_store,
};
use crate::{AppConfig, Daemon};

pub async fn load_calendar_data<S: Store>(
    tree: &HashTree<S>,
    config: &AppConfig,
    owner_npub: &str,
) -> Result<CalendarData, CalendarError> {
    let owner = config.profile.as_ref().map_or_else(
        || owner_npub.to_owned(),
        |state| crate::app_key_summary::pubkey_npub(&state.app_key_pubkey),
    );
    replica_store::Snapshot::load(tree, config, &owner, None)
        .await?
        .view()
}

pub async fn put_calendar_event_ics(
    daemon: &mut Daemon,
    resource_id: &str,
    body: &[u8],
) -> Result<CalendarEvent, CalendarError> {
    put_calendar_event_ics_checked(daemon, resource_id, body, |_| Ok(()))
        .await
        .map(|(event, _)| event)
}

pub(crate) async fn put_calendar_event_ics_checked(
    daemon: &mut Daemon,
    resource_id: &str,
    body: &[u8],
    check: impl FnOnce(Option<&CalendarEvent>) -> Result<(), CalendarError>,
) -> Result<(CalendarEvent, bool), CalendarError> {
    let _lock = calendar_mutation_lock(daemon).await?;
    let actor = calendar_actor(daemon.config());
    let writer = replica_store::ensure_local_writer(daemon)?;
    let mut snapshot =
        replica_store::Snapshot::load(daemon.tree(), daemon.config(), &actor, Some(&writer))
            .await?;
    let data = snapshot.view()?;
    let now = now_millis();
    let ics = std::str::from_utf8(body).map_err(|e| CalendarError::Data(e.to_string()))?;
    let mut event = event_from_ics(ics, resource_id, now)?;
    let previous = data.events.iter().find(|existing| {
        existing.id == resource_id
            || event_href_id(&existing.id) == resource_id
            || existing.id == event.id
    });
    check(previous)?;
    let mut before = replica::Fields::new();
    if let Some(previous) = previous {
        event.id.clone_from(&previous.id);
        event.color.clone_from(&previous.color);
        event
            .recurrence_source_id
            .clone_from(&previous.recurrence_source_id);
        event.created_at = previous.created_at;
        event.updated_at = previous.updated_at;
        let mut previous_fields = previous.extra.clone();
        previous_fields.remove("day");
        previous_fields.remove("days");
        event.extra.append(&mut previous_fields);
        before = projection::event_fields(previous)?;
    }
    let mut edits = projection::differences(before, projection::event_fields(&event)?);
    if edits.is_empty() {
        return Ok((event, previous.is_some()));
    }
    event.updated_at = now;
    edits.insert(
        vec!["events".into(), event.id.clone(), "updatedAt".into()],
        json!(now),
    );
    replica_store::mark_updated(&mut edits, &actor, now);
    snapshot.write(daemon, writer, edits).await?;
    let event = snapshot
        .view()?
        .events
        .into_iter()
        .find(|item| item.id == event.id)
        .ok_or_else(|| CalendarError::Data("CalDAV event was not visible after write".into()))?;
    Ok((event, previous.is_some()))
}

pub async fn delete_calendar_event(
    daemon: &mut Daemon,
    resource_id: &str,
) -> Result<bool, CalendarError> {
    delete_calendar_event_checked(daemon, resource_id, |_| Ok(())).await
}

pub(crate) async fn delete_calendar_event_checked(
    daemon: &mut Daemon,
    resource_id: &str,
    check: impl FnOnce(Option<&CalendarEvent>) -> Result<(), CalendarError>,
) -> Result<bool, CalendarError> {
    let _lock = calendar_mutation_lock(daemon).await?;
    let actor = calendar_actor(daemon.config());
    let writer = replica_store::ensure_local_writer(daemon)?;
    let mut snapshot =
        replica_store::Snapshot::load(daemon.tree(), daemon.config(), &actor, Some(&writer))
            .await?;
    let data = snapshot.view()?;
    let event = data
        .events
        .iter()
        .find(|event| event.id == resource_id || event_href_id(&event.id) == resource_id);
    check(event)?;
    let Some(event) = event else {
        return Ok(false);
    };
    let mut edits = replica::Fields::from([(vec!["events".into(), event.id.clone()], Value::Null)]);
    replica_store::mark_updated(&mut edits, &actor, now_millis());
    snapshot.write(daemon, writer, edits).await?;
    Ok(true)
}

async fn calendar_mutation_lock(
    daemon: &mut Daemon,
) -> Result<crate::config_lock::ConfigMutationLock, CalendarError> {
    let lock = crate::config_lock::ConfigMutationLock::acquire(daemon.config_dir())
        .await
        .map_err(|e| CalendarError::Data(e.to_string()))?;
    *daemon = Daemon::open(daemon.config_dir())?;
    Ok(lock)
}

/// Store a historical Calendar snapshot for import and bridge fixtures.
pub async fn save_calendar_data(
    daemon: &mut Daemon,
    data: &CalendarData,
) -> Result<(), CalendarError> {
    let bytes = serde_json::to_vec_pretty(data)?;
    let mut bytes = bytes;
    bytes.push(b'\n');
    let tree = daemon.tree();
    let base_root = if let Some(root) = current_calendar_root(daemon.config())? {
        root
    } else {
        tree.put_directory(Vec::new())
            .await
            .map_err(|e| CalendarError::Hashtree(e.to_string()))?
    };
    let (file_cid, size) = tree
        .put_file(&bytes)
        .await
        .map_err(|e| CalendarError::Hashtree(e.to_string()))?;
    let mut meta = HashMap::new();
    meta.insert(
        "contentType".into(),
        Value::String("application/json".into()),
    );
    meta.insert("mimeType".into(), Value::String("application/json".into()));
    let visible_root = tree
        .set_entry_with_meta(
            &base_root,
            &[],
            CALENDAR_DATA_FILE,
            &file_cid,
            size,
            LinkType::Blob,
            Some(meta),
        )
        .await
        .map_err(|e| CalendarError::Hashtree(e.to_string()))?;
    daemon
        .import_visible_root_for_drive(CALENDAR_TREE_NAME, visible_root)
        .await?;
    Ok(())
}

pub(super) fn current_calendar_root(config: &AppConfig) -> Result<Option<Cid>, CalendarError> {
    let Some(state) = config.profile.as_ref() else {
        return Ok(None);
    };
    let scope = state.root_scope_id();
    let Some(drive) = config
        .drive(CALENDAR_TREE_NAME)
        .filter(|drive| drive.root_scope_id == scope)
    else {
        return Ok(None);
    };
    let root = drive
        .app_key_roots
        .get(&state.app_key_pubkey)
        .map(|root| root.root_cid.as_str())
        .or_else(|| {
            drive
                .app_key_roots
                .is_empty()
                .then_some(drive.last_root_cid.as_deref())
                .flatten()
        });
    root.map(Cid::parse)
        .transpose()
        .map_err(|e| CalendarError::Data(e.to_string()))
}

fn calendar_actor(config: &AppConfig) -> String {
    config.profile.as_ref().map_or_else(
        || "iris-caldav".into(),
        |profile| crate::app_key_summary::pubkey_npub(&profile.app_key_pubkey),
    )
}
