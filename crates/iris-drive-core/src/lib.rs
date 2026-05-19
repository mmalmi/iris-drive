pub mod config;
pub mod conflict;
pub mod identity;
pub mod indexer;
pub mod paths;
pub mod sync;

pub use config::{AppConfig, ConfigError, Drive, DriveRole};
pub use conflict::{conflict_filename, resolve as resolve_conflict, FileSnapshot, SyncAction};
pub use identity::{Identity, IdentityError};
pub use indexer::{index_dir, IndexError};
pub use sync::{sync as run_sync, ConflictResolution, SyncError, SyncReport};

/// Schema version for the iris-drive config file. Bump when fields are
/// removed or repurposed so older installs can detect incompatible state.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;
