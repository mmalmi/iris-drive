pub mod account;
pub mod app_keys;
pub mod blossom_sync;
pub mod config;

/// Convenience constructor: a `BlossomClient` wired with the given
/// signing keys and the given server URLs. Used as both the write and
/// read pool — single account installs typically use the same servers
/// for both.
#[must_use]
pub fn blossom_sync_client(
    keys: nostr_sdk::Keys,
    servers: &[String],
) -> hashtree_blossom::BlossomClient {
    let client = hashtree_blossom::BlossomClient::new(keys);
    let mut read_servers = client.read_servers().to_vec();
    for server in servers {
        if !read_servers.iter().any(|candidate| candidate == server) {
            read_servers.push(server.clone());
        }
    }

    client
        .with_read_servers(read_servers)
        .with_write_servers(servers.to_vec())
}
pub mod conflict;
pub mod daemon;
pub mod gateway;
pub mod history;
pub mod identity;
pub mod indexer;
pub mod merge;
pub mod nostr_events;
pub mod paths;
pub mod relay_sync;
pub mod root_meta;
pub mod sync;
pub mod sync_cache;

#[cfg(test)]
mod tests {
    use nostr_sdk::Keys;

    #[test]
    fn blossom_sync_client_preserves_configured_writes_as_reads_too() {
        let write_server = "https://write.example".to_string();
        let client =
            super::blossom_sync_client(Keys::generate(), std::slice::from_ref(&write_server));

        assert_eq!(client.write_servers(), std::slice::from_ref(&write_server));
        assert!(
            client
                .read_servers()
                .iter()
                .any(|server| server == &write_server)
        );
    }
}

pub use account::{Account, AccountError, AccountState, DeviceAuthorizationState};
pub use app_keys::{AppKeysSnapshot, ApplyDecision, DeviceEntry, apply_snapshot, select_latest};
pub use config::{AppConfig, ConfigError, DeviceRootRef, Drive, DriveRole};
pub use conflict::{
    ConflictDeletedSide, ConflictRecord, ConflictSide, ConflictState, FileSnapshot, SyncAction,
    conflict_filename, conflict_records_from_merge, resolve as resolve_conflict,
};
pub use daemon::{Daemon, DaemonError, ImportReport, PRIMARY_DRIVE_ID};
pub use gateway::{GatewayBind, GatewayError, GatewayServer};
pub use identity::{DeviceIdentity, Identity, IdentityError, OwnerKey};
pub use indexer::{IndexError, index_dir, layer_conflict_records, read_conflict_records};
pub use merge::{
    CONFLICTS_PREFIX, DeviceFileEntry, DeviceSnapshot, DeviceTombstone, META_DIR, MergedConflict,
    MergedConflictFile, MergedConflictKind, MergedConflictTombstone, MergedEntry, MergedView,
    ROOT_META_PATH, TOMBSTONE_PREFIX, WHOLE_FILE_HASH_META_KEY, merge_drives,
    original_path_from_tombstone, tombstone_path,
};
pub use root_meta::{DriveRootMeta, RootObservation, RootParent};
pub use sync::{
    ConflictResolution, SyncBaseState, SyncError, SyncReport, sync as run_sync, sync_with_base,
    sync_with_base_anchor, sync_with_cache,
};
pub use sync_cache::{
    CachedBaseAnchor, CachedBaseState, CachedPathState, CachedRoot, ContentNeed, RetrievalOutcome,
    SOURCE_STATE_AVAILABLE, SOURCE_STATE_MISSING, SOURCE_STATE_POISONED, SOURCE_STATE_UNKNOWN,
    SourceAvailability, SyncCache, SyncCacheError,
};

/// Schema version for the iris-drive config file. Bump when fields are
/// removed or repurposed so older installs can detect incompatible state.
///
/// v2: added optional `AccountState` for the owner/device key split + `AppKeys`.
pub const CONFIG_SCHEMA_VERSION: u32 = 2;
