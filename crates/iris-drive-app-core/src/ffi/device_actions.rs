use std::path::Path;

use iris_drive_core::Profile;
use iris_drive_core::paths::config_path_in;

use super::{NativeAppRuntime, normalize_pubkey};

impl NativeAppRuntime {
    pub(super) fn rename_device(&mut self, app_key_pubkey: &str, label: &str) {
        let app_key_pubkey = match normalize_pubkey(app_key_pubkey) {
            Ok(pubkey) => pubkey,
            Err(error) => {
                self.state.error = error;
                return;
            }
        };
        let mut config = match self.load_config() {
            Ok(config) => config,
            Err(error) => {
                self.state.error = error;
                return;
            }
        };
        let Some(state) = config.profile.clone() else {
            "admin profile is required to rename devices".clone_into(&mut self.state.error);
            return;
        };
        let mut account = match Profile::load(state, Path::new(&self.data_dir)) {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("loading profile: {error}");
                return;
            }
        };
        if let Err(error) = account.rename_app_key(&app_key_pubkey, label.to_owned()) {
            self.state.error = format!("renaming device: {error}");
            return;
        }
        config.profile = Some(account.state);
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::NativeAppAction;

    use super::super::FfiApp;

    #[test]
    fn rename_device_action_updates_linked_device_label() {
        let owner_dir = tempfile::tempdir().unwrap();
        let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
        let owner = app.dispatch(NativeAppAction::CreateProfile {
            app_key_label: "Mac".to_owned(),
        });
        let linked_dir = tempfile::tempdir().unwrap();
        let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
        let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
            link_target: owner.ui.profile.unwrap().app_key_link_invite,
            app_key_label: "Phone".to_owned(),
        });
        let linked_account = linked.ui.profile.unwrap();
        let linked_device = linked_account.current_app_key_npub;
        let approved = app.dispatch(NativeAppAction::ApproveDevice {
            request: linked_account.app_key_link_request,
            label: "Phone".to_owned(),
        });
        assert!(approved.error.is_empty(), "{}", approved.error);

        let renamed = app.dispatch(NativeAppAction::RenameDevice {
            app_key_pubkey: linked_device.clone(),
            label: "iPhone 16".to_owned(),
        });

        assert!(renamed.error.is_empty(), "{}", renamed.error);
        assert!(renamed.ui.app_actors.iter().any(|device| {
            device.pubkey == linked_device && device.label == "iPhone 16" && device.role == "member"
        }));
    }
}
