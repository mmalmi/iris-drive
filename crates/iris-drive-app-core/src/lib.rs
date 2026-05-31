pub mod actions;
pub mod c_abi;
mod ffi;
#[cfg(not(test))]
mod native_fips;
mod native_provider;
mod provider_metadata;
pub mod state;

pub use actions::NativeAppAction;
pub use ffi::FfiApp;
pub use state::{NativeAppState, UiState};

uniffi::setup_scaffolding!();
