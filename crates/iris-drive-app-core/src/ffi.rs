use std::sync::{Arc, Mutex};

use crate::actions::NativeAppAction;
use crate::state::{NativeAppState, UiSyncRoot};

#[derive(uniffi::Object, Debug)]
pub struct FfiApp {
    runtime: Mutex<NativeAppRuntime>,
}

#[uniffi::export]
impl FfiApp {
    #[uniffi::constructor]
    #[allow(clippy::needless_pass_by_value)]
    #[must_use]
    pub fn new(data_dir: String, app_version: String) -> Arc<Self> {
        Arc::new(Self {
            runtime: Mutex::new(NativeAppRuntime::new(data_dir, app_version)),
        })
    }

    #[must_use]
    pub fn state(&self) -> NativeAppState {
        self.with_runtime(NativeAppRuntime::state)
    }

    #[must_use]
    pub fn refresh(&self) -> NativeAppState {
        self.dispatch(NativeAppAction::Refresh)
    }

    #[must_use]
    pub fn dispatch(&self, action: NativeAppAction) -> NativeAppState {
        self.with_runtime(|runtime| {
            runtime.dispatch(action);
            runtime.state()
        })
    }
}

impl FfiApp {
    fn with_runtime(
        &self,
        f: impl FnOnce(&mut NativeAppRuntime) -> NativeAppState,
    ) -> NativeAppState {
        match self.runtime.lock() {
            Ok(mut runtime) => f(&mut runtime),
            Err(poisoned) => {
                let mut runtime = poisoned.into_inner();
                "native app state lock was poisoned".clone_into(&mut runtime.state.error);
                f(&mut runtime)
            }
        }
    }
}

#[derive(Debug)]
struct NativeAppRuntime {
    state: NativeAppState,
    data_dir: String,
    app_version: String,
}

impl NativeAppRuntime {
    fn new(data_dir: String, app_version: String) -> Self {
        Self {
            state: NativeAppState::default(),
            data_dir,
            app_version,
        }
    }

    fn state(&mut self) -> NativeAppState {
        let _ = (&self.data_dir, &self.app_version);
        self.state.clone()
    }

    fn dispatch(&mut self, action: NativeAppAction) {
        self.state.error.clear();
        match action {
            NativeAppAction::Refresh => {}
            NativeAppAction::AddRoot { name, local_path } => self.add_root(&name, &local_path),
            NativeAppAction::RemoveRoot { name } => self.remove_root(&name),
        }
    }

    fn add_root(&mut self, name: &str, local_path: &str) {
        let name = name.trim();
        let local_path = local_path.trim();
        if name.is_empty() {
            "root name is required".clone_into(&mut self.state.error);
            return;
        }
        if local_path.is_empty() {
            "root path is required".clone_into(&mut self.state.error);
            return;
        }

        let root = UiSyncRoot {
            name: name.to_owned(),
            local_path: local_path.to_owned(),
            status: "pending provider hookup".to_owned(),
        };
        match self
            .state
            .ui
            .roots
            .iter_mut()
            .find(|existing| existing.name == root.name)
        {
            Some(existing) => *existing = root,
            None => self.state.ui.roots.push(root),
        }
        self.state
            .ui
            .roots
            .sort_by(|left, right| left.name.cmp(&right.name));
    }

    fn remove_root(&mut self, name: &str) {
        let before = self.state.ui.roots.len();
        self.state.ui.roots.retain(|root| root.name != name);
        if before == self.state.ui.roots.len() {
            self.state.error = format!("sync root not found: {name}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FfiApp;
    use crate::NativeAppAction;

    #[test]
    fn dispatch_adds_updates_and_removes_roots() {
        let app = FfiApp::new("/tmp/iris-drive".to_owned(), "test".to_owned());

        let state = app.dispatch(NativeAppAction::AddRoot {
            name: "My Drive".to_owned(),
            local_path: "/virtual/iris".to_owned(),
        });
        assert_eq!(state.ui.roots.len(), 1);
        assert_eq!(state.ui.roots[0].name, "My Drive");
        assert!(state.error.is_empty());

        let state = app.dispatch(NativeAppAction::AddRoot {
            name: "My Drive".to_owned(),
            local_path: "/virtual/iris-renamed".to_owned(),
        });
        assert_eq!(state.ui.roots.len(), 1);
        assert_eq!(state.ui.roots[0].local_path, "/virtual/iris-renamed");

        let state = app.dispatch(NativeAppAction::RemoveRoot {
            name: "My Drive".to_owned(),
        });
        assert!(state.ui.roots.is_empty());
        assert!(state.error.is_empty());
    }

    #[test]
    fn dispatch_rejects_empty_roots() {
        let app = FfiApp::new("/tmp/iris-drive".to_owned(), "test".to_owned());

        let state = app.dispatch(NativeAppAction::AddRoot {
            name: String::new(),
            local_path: "/virtual/iris".to_owned(),
        });

        assert!(state.ui.roots.is_empty());
        assert_eq!(state.error, "root name is required");
    }
}
