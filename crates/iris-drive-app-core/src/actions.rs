use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum NativeAppAction {
    Refresh,
    AddRoot { name: String, local_path: String },
    RemoveRoot { name: String },
}
