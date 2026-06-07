pub mod actions;
pub mod c_abi;
mod ffi;
mod native_provider;
mod provider_metadata;
pub mod state;
pub mod update_policy;

pub use actions::NativeAppAction;
pub use ffi::{
    DriveLinkForCid, FfiApp, GeneratedRecoveryKey, LinkInputClassification, RecoverySecretExport,
    classify_link_input, drive_link_for_cid, export_recovery_secret, generate_recovery_key,
    recovery_pubkey_for_phrase, validate_link_input,
};
pub use state::{NativeAppState, UiState};
pub use update_policy::UpdateAutoCheckPolicy;

uniffi::setup_scaffolding!();
