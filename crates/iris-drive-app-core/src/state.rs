use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UiState {
    pub roots: Vec<UiSyncRoot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSyncRoot {
    pub name: String,
    pub local_path: String,
    pub status: String,
}

#[derive(Debug, Clone, Default)]
pub struct NativeAppState {
    pub ui: UiState,
}
