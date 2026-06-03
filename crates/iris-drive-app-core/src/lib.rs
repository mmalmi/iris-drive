pub mod actions;
pub mod c_abi;
mod ffi;
mod native_provider;
mod provider_metadata;
pub mod state;

pub use actions::NativeAppAction;
pub use ffi::{
    FfiApp, LinkInputClassification, RecoverySecretExport, classify_link_input,
    export_recovery_secret, validate_link_input,
};
pub use state::{NativeAppState, UiState};

uniffi::setup_scaffolding!();
