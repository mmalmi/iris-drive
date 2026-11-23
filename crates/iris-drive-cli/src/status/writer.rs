use super::{
    DAEMON_STATUS_SCHEMA, daemon_status_path, normalize_daemon_status_for_clients, unix_now,
};
use serde_json::{Map, Value, json};
use std::{fs, io, io::Write, path::Path};
use tempfile::NamedTempFile;

pub(crate) fn write_daemon_status(config_dir: &Path, payload: Value) -> Value {
    update_status(config_dir, payload, |mut previous, payload| {
        let Some(current) = payload.as_object_mut() else {
            return;
        };
        for key in [
            "last_block_sync",
            "block_sync_by_root",
            "block_sync_error_by_root",
            "relays",
            "current_app_key_npub",
            "summary",
            "hashtree",
            "provider_update_mode",
            "watch_debounce_ms",
            "mount",
            "relay_statuses",
            "embedded_hashtree",
            "browser_gateway",
            "fips_block_sync",
            "fips_block_sync_error",
            "fips",
        ] {
            if !current.contains_key(key)
                && let Some(value) = previous.get_mut(key)
            {
                current.insert(key.to_string(), value.take());
            }
        }
    })
}

pub(crate) fn merge_daemon_status(config_dir: &Path, update: impl FnOnce(&mut Map<String, Value>)) {
    update_status(config_dir, Value::Null, |previous, payload| {
        let mut current = match previous {
            Value::Object(object) => object,
            _ => Map::new(),
        };
        update(&mut current);
        *payload = Value::Object(current);
    });
}

fn update_status(
    config_dir: &Path,
    mut payload: Value,
    update: impl FnOnce(Value, &mut Value),
) -> Value {
    let persist = || -> io::Result<()> {
        fs::create_dir_all(config_dir)?;
        // The status file itself is replaced on each write. Keep the lock on a stable inode
        // so heartbeats, block downloads and other CLI processes serialize the whole update.
        let guard = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(config_dir.join("daemon-status-write.lock"))?;
        guard.lock()?;
        let path = daemon_status_path(config_dir);
        let previous = match fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or(Value::Null),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Value::Null,
            Err(error) => return Err(error),
        };
        update(previous, &mut payload);
        stamp_status(config_dir, &mut payload);
        // Readers need one complete snapshot. Status is transient and is rebuilt on startup.
        let mut snapshot = NamedTempFile::new_in(config_dir)?;
        snapshot.write_all(&serde_json::to_vec_pretty(&payload)?)?;
        snapshot.persist(path)?;
        Ok(())
    };
    if let Err(error) = persist() {
        tracing::warn!(%error, "could not persist daemon status");
    }
    payload
}

fn stamp_status(config_dir: &Path, payload: &mut Value) {
    let heartbeat = payload.get("event").and_then(Value::as_str) == Some("heartbeat");
    if let Some(object) = payload.as_object_mut() {
        object.insert("schema".to_string(), json!(DAEMON_STATUS_SCHEMA));
        object.insert("pid".to_string(), json!(std::process::id()));
        object.insert("running".to_string(), json!(true));
        object.insert("fresh".to_string(), json!(true));
        object.insert("updated_at".to_string(), json!(unix_now()));
        object.insert(
            "binary_version".to_string(),
            json!(env!("CARGO_PKG_VERSION")),
        );
    }
    if !heartbeat {
        normalize_daemon_status_for_clients(config_dir, payload);
    }
}
