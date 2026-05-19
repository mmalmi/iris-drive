pub mod config;
pub mod htree_client;
pub mod paths;
pub mod share;
pub mod sync;

/// Schema version for the hashdrive config file. Bump when fields are
/// removed or repurposed so older installs can detect incompatible state.
pub const CONFIG_SCHEMA_VERSION: u32 = 1;
