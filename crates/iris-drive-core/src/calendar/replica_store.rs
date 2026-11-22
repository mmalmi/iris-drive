use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{OsRng, rand_core::RngCore};
use hashtree_core::{Cid, HashTree, LinkType, Store};
use serde_json::{Value, json};

use super::projection::{flatten, genesis, materialize};
use super::replica::{
    CHANGES_FILE, Fields, MAX_CLOCK, Registers, Replica, TREE_PREFIX, invalid, valid_writer,
};
use super::{
    CALENDAR_DATA_FILE, CalendarData, CalendarError, MAX_CALENDAR_BYTES, current_calendar_root,
    replica_writer,
};
use crate::config::{AppConfig, Drive, DriveRole};
use crate::{Daemon, ProfileState};

pub(crate) const MAX_REPLICAS: usize = 256;

pub(super) struct Snapshot {
    base: Fields,
    is_new: bool,
    registers: Registers,
    local_replica: Option<Replica>,
}

impl Snapshot {
    pub(super) async fn load<S: Store>(
        tree: &HashTree<S>,
        config: &AppConfig,
        owner: &str,
        local_writer: Option<&str>,
    ) -> Result<Self, CalendarError> {
        let legacy = current_calendar_root(config)?;
        let base = flatten(&load_baseline(tree, legacy.as_ref(), owner).await?)?;
        let mut snapshot = Self {
            base,
            is_new: legacy.is_none(),
            registers: Registers::default(),
            local_replica: None,
        };
        let mut replica_bytes = 0usize;
        if let Some(state) = config.profile.as_ref() {
            let scope = state.root_scope_id();
            for drive in config
                .drives
                .iter()
                .filter(|drive| {
                    drive.root_scope_id == scope && replica_writer(&drive.drive_id).is_some()
                })
                .take(MAX_REPLICAS + 1)
                .enumerate()
            {
                let (index, drive) = drive;
                if index == MAX_REPLICAS {
                    return Err(invalid("too many calendar replicas"));
                }
                let writer = replica_writer(&drive.drive_id).expect("filtered replica tree");
                let Some(reference) = drive.app_key_roots.get(&state.app_key_pubkey) else {
                    continue;
                };
                let cid = Cid::parse(&reference.root_cid).map_err(|e| invalid(e.to_string()))?;
                if cid.key.is_none() {
                    return Err(invalid("calendar replica is not private"));
                }
                let bytes = read_tree_file(tree, &cid, CHANGES_FILE)
                    .await?
                    .ok_or_else(|| invalid("calendar replica changes are missing"))?;
                replica_bytes = replica_bytes.saturating_add(bytes.len());
                if u64::try_from(replica_bytes).unwrap_or(u64::MAX) > MAX_CALENDAR_BYTES * 4 {
                    return Err(invalid("calendar replicas exceed the aggregate size limit"));
                }
                let replica = Replica::decode(&bytes, writer)?;
                snapshot.registers.merge(&replica);
                if local_writer == Some(writer) {
                    snapshot.local_replica = Some(replica);
                }
            }
        }
        Ok(snapshot)
    }

    pub(super) fn view(&self) -> Result<CalendarData, CalendarError> {
        materialize(self.registers.overlay(self.base.clone(), self.is_new))
    }

    pub(super) async fn write(
        &mut self,
        daemon: &mut Daemon,
        writer: String,
        mut edits: Fields,
    ) -> Result<(), CalendarError> {
        if self.is_new && self.registers.initial_timezone().is_none() {
            edits.insert(vec!["initialTimezone".into()], json!("UTC"));
        }
        let clock = self
            .registers
            .clock
            .checked_add(1)
            .filter(|clock| *clock <= MAX_CLOCK)
            .ok_or_else(|| invalid("calendar edit clock is exhausted"))?;
        let mut replica = self
            .local_replica
            .take()
            .unwrap_or_else(|| Replica::new(writer));
        replica.write(clock, edits);
        let bytes = serde_json::to_vec(&replica)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CALENDAR_BYTES {
            return Err(invalid("calendar replica exceeds the size limit"));
        }
        let tree = daemon.tree();
        let base = tree.put_directory(Vec::new()).await.map_err(tree_error)?;
        let (cid, size) = tree.put_file(&bytes).await.map_err(tree_error)?;
        let meta = HashMap::from([
            ("contentType".into(), json!("application/json")),
            ("mimeType".into(), json!("application/json")),
        ]);
        let root = tree
            .set_entry_with_meta(
                &base,
                &[],
                CHANGES_FILE,
                &cid,
                size,
                LinkType::Blob,
                Some(meta),
            )
            .await
            .map_err(tree_error)?;
        daemon
            .import_visible_root_for_drive(&format!("{TREE_PREFIX}{}", replica.writer), root)
            .await?;
        self.registers.merge(&replica);
        self.local_replica = Some(replica);
        Ok(())
    }
}

async fn load_baseline<S: Store>(
    tree: &HashTree<S>,
    root: Option<&Cid>,
    owner: &str,
) -> Result<CalendarData, CalendarError> {
    let Some(root) = root else {
        return Ok(genesis(owner));
    };
    let bytes = read_tree_file(tree, root, CALENDAR_DATA_FILE)
        .await?
        .ok_or_else(|| invalid("calendar file is missing"))?;
    let mut data: CalendarData = serde_json::from_slice(&bytes)?;
    data.owner_npub = owner.to_owned();
    Ok(data)
}

pub(super) async fn read_tree_file<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    filename: &str,
) -> Result<Option<Vec<u8>>, CalendarError> {
    let Some(cid) = tree.resolve(root, filename).await.map_err(tree_error)? else {
        return Ok(None);
    };
    let bytes = tree
        .read_file_range_cid(&cid, 0, Some(MAX_CALENDAR_BYTES + 1))
        .await
        .map_err(tree_error)?
        .ok_or_else(|| invalid("calendar document is unreadable"))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_CALENDAR_BYTES {
        return Err(invalid("calendar document exceeds the size limit"));
    }
    Ok(Some(bytes))
}

fn tree_error(error: impl std::fmt::Display) -> CalendarError {
    CalendarError::Hashtree(error.to_string())
}

fn writer_path(config_dir: &Path, state: &ProfileState) -> PathBuf {
    config_dir.join(format!("calendar-writer-v1-{}", state.app_key_pubkey))
}

/// This must be called while holding the profile config mutation lock.
pub(super) fn ensure_local_writer(daemon: &Daemon) -> Result<String, CalendarError> {
    let state = daemon
        .config()
        .profile
        .as_ref()
        .filter(|state| state.can_write_roots())
        .ok_or_else(|| invalid("profile cannot write Calendar roots"))?;
    if let Some(writer) = read_writer(daemon.config_dir(), state)? {
        return Ok(writer);
    }
    let mut bytes = [0; 16];
    OsRng.fill_bytes(&mut bytes);
    let writer = hex::encode(bytes);
    crate::atomic_file::atomic_write_private(
        &writer_path(daemon.config_dir(), state),
        writer.as_bytes(),
    )
    .map_err(|e| invalid(e.to_string()))?;
    Ok(writer)
}

fn read_writer(config_dir: &Path, state: &ProfileState) -> Result<Option<String>, CalendarError> {
    match std::fs::read_to_string(writer_path(config_dir, state)) {
        Ok(writer) if valid_writer(&writer) => Ok(Some(writer)),
        Ok(_) => Err(invalid("invalid local Calendar writer identity")),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(invalid(error.to_string())),
    }
}

/// A reader can track many writer trees with the same signing key. Only the
/// locally allocated writer slot is a publication candidate.
pub fn local_replica_tree(
    config_dir: &Path,
    state: &ProfileState,
) -> Result<Option<String>, CalendarError> {
    Ok(read_writer(config_dir, state)?.map(|writer| format!("{TREE_PREFIX}{writer}")))
}

pub(crate) fn account_replica_drive(name: &str, scope: String) -> Drive {
    Drive {
        root_scope_id: scope,
        drive_id: name.into(),
        display_name: "Calendar replica".into(),
        role: DriveRole::Owner,
        app_key_roots: Default::default(),
        last_root_cid: None,
        key_hex: None,
    }
}

pub(super) fn mark_updated(edits: &mut Fields, actor: &str, now: i64) {
    for (field, value) in [
        ("updatedAt", json!(now)),
        ("updatedBy", Value::String(actor.into())),
    ] {
        edits.insert(vec![field.into()], value);
    }
}
