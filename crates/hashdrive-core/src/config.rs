use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub roots: Vec<SyncRoot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncRoot {
    pub name: String,
    pub local_path: String,
}
