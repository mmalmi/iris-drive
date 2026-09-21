//! Private, explicit friend backup contacts and storage commitments.

mod config;
mod exchange;
mod export;
mod identity;
pub mod recovery;
mod routes;
pub(crate) mod runtime;
mod selection;
pub(crate) mod service_lease;
pub mod status;
pub mod storage;

pub use config::{
    FriendBackupConfig, FriendBackupPeer, MAX_FRIEND_LABEL_BYTES, MAX_FRIENDS, remove_friend,
    set_capacity, upsert_friend,
};
pub use identity::{
    BACKUP_INVITE_PREFIX, backup_keys, backup_npub, encode_backup_invite, parse_backup_contact,
};

#[cfg(test)]
mod tests;

#[cfg(test)]
mod runtime_tests;
