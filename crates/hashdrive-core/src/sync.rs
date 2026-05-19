use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SyncState {
    Idle,
    Indexing,
    Uploading,
    Downloading,
    UpToDate,
}
