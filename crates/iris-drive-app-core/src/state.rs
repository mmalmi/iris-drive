use serde::{Deserialize, Serialize};

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiState {
    pub roots: Vec<UiSyncRoot>,
}

#[derive(uniffi::Record, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct UiSyncRoot {
    pub name: String,
    pub local_path: String,
    pub status: String,
}

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NativeAppState {
    pub ui: UiState,
    pub error: String,
}
