pub mod actions;
pub mod state;

pub use actions::NativeAppAction;
pub use state::{NativeAppState, UiState};

uniffi::setup_scaffolding!();
