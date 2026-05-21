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
    hashtree_blossom::BlossomClient::new(keys).with_servers(servers.to_vec())
}
pub mod conflict;
pub mod daemon;
pub mod history;
pub mod identity;
pub mod indexer;
pub mod merge;
pub mod nostr_events;
pub mod paths;
pub mod relay_sync;
pub mod root_meta;
pub mod sync;

pub use account::{Account, AccountError, AccountState, DeviceAuthorizationState};
pub use app_keys::{AppKeysSnapshot, ApplyDecision, DeviceEntry, apply_snapshot, select_latest};
pub use config::{AppConfig, ConfigError, DeviceRootRef, Drive, DriveRole};
pub use conflict::{
    ConflictRecord, ConflictSide, ConflictState, FileSnapshot, SyncAction, conflict_filename,
    resolve as resolve_conflict,
};
pub use daemon::{Daemon, DaemonError, ImportReport, PRIMARY_DRIVE_ID};
pub use identity::{DeviceIdentity, Identity, IdentityError, OwnerKey};
pub use indexer::{IndexError, index_dir, layer_conflict_records, read_conflict_records};
pub use merge::{
    CONFLICTS_PREFIX, DeviceFileEntry, DeviceSnapshot, DeviceTombstone, META_DIR, MergedEntry,
    MergedView, ROOT_META_PATH, TOMBSTONE_PREFIX, merge_drives, original_path_from_tombstone,
    tombstone_path,
};
pub use root_meta::{DriveRootMeta, RootObservation, RootParent};
pub use sync::{ConflictResolution, SyncError, SyncReport, sync as run_sync};

/// Schema version for the iris-drive config file. Bump when fields are
/// removed or repurposed so older installs can detect incompatible state.
///
/// v2: added optional `AccountState` for the owner/device key split + `AppKeys`.
pub const CONFIG_SCHEMA_VERSION: u32 = 2;
