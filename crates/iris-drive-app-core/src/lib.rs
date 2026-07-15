pub mod actions;
pub mod c_abi;
mod ffi;
mod native_link_input;
mod native_provider;
mod provider_metadata;
pub mod state;

pub use actions::NativeAppAction;
pub use ffi::{
    DriveLinkForCid, FfiApp, GeneratedRecoveryKey, RecoverySecretExport, drive_link_for_cid,
    export_recovery_secret, generate_recovery_key, recovery_pubkey_for_phrase,
};
pub use iris_drive_core::updater::UpdateAutoCheckPolicy;
pub use native_link_input::{
    LinkInputClassification, classify_link_input, validate_device_approval_input,
    validate_device_invite_input, validate_link_input,
};
pub use state::{NativeAppState, UiState};

uniffi::setup_scaffolding!();
