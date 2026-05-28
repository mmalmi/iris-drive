use serde::{Deserialize, Serialize};

#[derive(uniffi::Enum, Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum NativeAppAction {
    Refresh,
    AddRoot { name: String, local_path: String },
    RemoveRoot { name: String },
}
