pub mod actions;
pub mod c_abi;
mod ffi;
pub mod state;

pub use actions::NativeAppAction;
pub use ffi::FfiApp;
pub use state::{NativeAppState, UiState};

uniffi::setup_scaffolding!();
