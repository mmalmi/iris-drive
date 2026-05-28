use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use iris_drive_core::config::{DEFAULT_BLOSSOM_SERVERS, DEFAULT_RELAYS};

use crate::actions::NativeAppAction;
use crate::state::{
    NativeAppState, UiAccount, UiBackup, UiDevice, UiPaths, UiSyncRoot, UiSyncStatus,
};

const DEFAULT_DEVICE_LABEL: &str = "This device";
const DEFAULT_ROOT_STATUS: &str = "SAF provider root";
static NEXT_SYNTHETIC_PUBKEY: AtomicU64 = AtomicU64::new(1);

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
        let mut state = NativeAppState::default();
        state.ui.relays = default_relays();
        state.ui.backups = default_backups();
        state.ui.paths = paths_for(&data_dir);
        state.ui.sync = UiSyncStatus {
            running: false,
            status: "paused".to_owned(),
        };
        update_snapshot_link(&mut state);

        Self {
            state,
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
            NativeAppAction::CreateProfile { device_label } => {
                self.create_profile(&device_label);
            }
            NativeAppAction::RestoreProfile {
                secret,
                device_label,
            } => {
                self.restore_profile(&secret, &device_label);
            }
            NativeAppAction::LinkDevice {
                owner_pubkey,
                device_label,
            } => {
                self.link_device(&owner_pubkey, &device_label);
            }
            NativeAppAction::ApproveDevice { request, label } => {
                self.approve_device(&request, &label);
            }
            NativeAppAction::RevokeDevice { device_pubkey } => {
                self.revoke_device(&device_pubkey);
            }
            NativeAppAction::AppointAdmin { device_pubkey } => {
                self.set_device_admin_role(&device_pubkey, true);
            }
            NativeAppAction::DemoteAdmin { device_pubkey } => {
                self.set_device_admin_role(&device_pubkey, false);
            }
            NativeAppAction::AddRelay { url } => self.add_relay(&url),
            NativeAppAction::RemoveRelay { url } => self.remove_relay(&url),
            NativeAppAction::ResetRelays => self.state.ui.relays = default_relays(),
            NativeAppAction::StartSync | NativeAppAction::RestartSync => {
                self.set_sync_running(true);
            }
            NativeAppAction::StopSync => self.set_sync_running(false),
            NativeAppAction::AddRoot { name, local_path } => self.add_root(&name, &local_path),
            NativeAppAction::RemoveRoot { name } => self.remove_root(&name),
        }
        update_snapshot_link(&mut self.state);
    }

    fn create_profile(&mut self, device_label: &str) {
        let owner_pubkey = generated_pubkey();
        let device_pubkey = generated_pubkey();
        let device_label = label_or_default(device_label);
        self.state.ui.account = Some(UiAccount {
            owner_pubkey: owner_pubkey.clone(),
            device_pubkey: device_pubkey.clone(),
            device_label: device_label.clone(),
            authorization_state: "authorized".to_owned(),
            has_owner_signing_authority: true,
            device_link_request: format!(
                "iris-drive://device-link?owner={owner_pubkey}&device={device_pubkey}"
            ),
        });
        self.state.ui.devices = vec![local_device(device_pubkey, device_label, "Admin", "admin")];
        self.refresh_device_actions();
    }

    fn restore_profile(&mut self, secret: &str, device_label: &str) {
        if secret.trim().is_empty() {
            "owner secret is required".clone_into(&mut self.state.error);
            return;
        }

        let owner_pubkey = generated_pubkey();
        let device_pubkey = generated_pubkey();
        let device_label = label_or_default(device_label);
        self.state.ui.account = Some(UiAccount {
            owner_pubkey: owner_pubkey.clone(),
            device_pubkey: device_pubkey.clone(),
            device_label: device_label.clone(),
            authorization_state: "authorized".to_owned(),
            has_owner_signing_authority: true,
            device_link_request: format!(
                "iris-drive://device-link?owner={owner_pubkey}&device={device_pubkey}"
            ),
        });
        self.state.ui.devices = vec![local_device(device_pubkey, device_label, "Admin", "admin")];
        self.refresh_device_actions();
    }

    fn link_device(&mut self, owner_pubkey: &str, device_label: &str) {
        let owner_pubkey = owner_pubkey.trim();
        if owner_pubkey.is_empty() {
            "owner public key is required".clone_into(&mut self.state.error);
            return;
        }

        let device_pubkey = generated_pubkey();
        let device_label = label_or_default(device_label);
        self.state.ui.account = Some(UiAccount {
            owner_pubkey: owner_pubkey.to_owned(),
            device_pubkey: device_pubkey.clone(),
            device_label: device_label.clone(),
            authorization_state: "awaiting_approval".to_owned(),
            has_owner_signing_authority: false,
            device_link_request: format!(
                "iris-drive://device-link?owner={owner_pubkey}&device={device_pubkey}"
            ),
        });
        self.state.ui.devices = vec![local_device(
            device_pubkey,
            device_label,
            "Awaiting approval",
            "member",
        )];
        self.refresh_device_actions();
    }

    fn approve_device(&mut self, request: &str, label: &str) {
        if !self.can_manage_devices() {
            "owner profile is required to approve devices".clone_into(&mut self.state.error);
            return;
        }

        let request = request.trim();
        if request.is_empty() {
            "device request is required".clone_into(&mut self.state.error);
            return;
        }

        let device_pubkey = request_device_pubkey(request);
        let label = label_or(label, "Linked device");
        let device = UiDevice {
            pubkey: device_pubkey.clone(),
            label,
            role: "member".to_owned(),
            state: "Authorized".to_owned(),
            detail: request.to_owned(),
            is_online: self.state.ui.sync.running,
            can_revoke: true,
            can_appoint_admin: false,
            can_demote_admin: false,
        };
        match self
            .state
            .ui
            .devices
            .iter_mut()
            .find(|existing| existing.pubkey == device_pubkey)
        {
            Some(existing) => *existing = device,
            None => self.state.ui.devices.push(device),
        }
        self.refresh_device_actions();
    }

    fn revoke_device(&mut self, device_pubkey: &str) {
        let device_pubkey = device_pubkey.trim();
        if device_pubkey.is_empty() {
            "device public key is required".clone_into(&mut self.state.error);
            return;
        }
        if self
            .state
            .ui
            .account
            .as_ref()
            .is_some_and(|account| account.device_pubkey == device_pubkey)
        {
            "cannot revoke this device from itself".clone_into(&mut self.state.error);
            return;
        }

        let before = self.state.ui.devices.len();
        self.state
            .ui
            .devices
            .retain(|device| device.pubkey != device_pubkey);
        if before == self.state.ui.devices.len() {
            self.state.error = format!("device not found: {device_pubkey}");
        }
        self.refresh_device_actions();
    }

    fn set_device_admin_role(&mut self, device_pubkey: &str, make_admin: bool) {
        if !self.can_manage_devices() {
            "admin profile is required to manage device admins".clone_into(&mut self.state.error);
            return;
        }

        let device_pubkey = device_pubkey.trim();
        if device_pubkey.is_empty() {
            "device public key is required".clone_into(&mut self.state.error);
            return;
        }

        let admin_count = self
            .state
            .ui
            .devices
            .iter()
            .filter(|device| device.role == "admin")
            .count();
        let Some(device) = self
            .state
            .ui
            .devices
            .iter_mut()
            .find(|device| device.pubkey == device_pubkey)
        else {
            self.state.error = format!("device not found: {device_pubkey}");
            return;
        };

        if make_admin {
            device.role = "admin".to_owned();
            device.state = "Admin".to_owned();
        } else {
            if device.role == "admin" && admin_count <= 1 {
                "cannot remove the last admin".clone_into(&mut self.state.error);
                return;
            }
            device.role = "member".to_owned();
            device.state = "Authorized".to_owned();
        }
        self.refresh_device_actions();
    }

    fn add_relay(&mut self, url: &str) {
        let url = url.trim();
        if url.is_empty() {
            "relay URL is required".clone_into(&mut self.state.error);
            return;
        }
        if !self.state.ui.relays.iter().any(|existing| existing == url) {
            self.state.ui.relays.push(url.to_owned());
        }
    }

    fn remove_relay(&mut self, url: &str) {
        let url = url.trim();
        let before = self.state.ui.relays.len();
        self.state.ui.relays.retain(|relay| relay != url);
        if before == self.state.ui.relays.len() {
            self.state.error = format!("relay not found: {url}");
        }
    }

    fn set_sync_running(&mut self, running: bool) {
        self.state.ui.sync = UiSyncStatus {
            running,
            status: if running { "running" } else { "paused" }.to_owned(),
        };
        for device in &mut self.state.ui.devices {
            if !device.can_revoke {
                device.is_online = running;
            }
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
            status: DEFAULT_ROOT_STATUS.to_owned(),
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

    fn can_manage_devices(&self) -> bool {
        self.state
            .ui
            .account
            .as_ref()
            .is_some_and(|account| account.has_owner_signing_authority)
    }

    fn refresh_device_actions(&mut self) {
        let can_manage = self.can_manage_devices();
        let current_device = self
            .state
            .ui
            .account
            .as_ref()
            .map(|account| account.device_pubkey.clone());
        let admin_count = self
            .state
            .ui
            .devices
            .iter()
            .filter(|device| device.role == "admin")
            .count();
        for device in &mut self.state.ui.devices {
            let is_current = current_device
                .as_deref()
                .is_some_and(|current| current == device.pubkey);
            let is_admin = device.role == "admin";
            device.can_revoke = can_manage && !is_current;
            device.can_appoint_admin = can_manage && !is_current && !is_admin;
            device.can_demote_admin = can_manage && !is_current && is_admin && admin_count > 1;
        }
    }
}

fn paths_for(data_dir: &str) -> UiPaths {
    UiPaths {
        data_dir: data_dir.to_owned(),
        config_path: path_join(data_dir, "config.toml"),
        blocks_dir: path_join(data_dir, "blocks"),
    }
}

fn path_join(data_dir: &str, child: &str) -> String {
    if data_dir.is_empty() {
        child.to_owned()
    } else {
        Path::new(data_dir).join(child).display().to_string()
    }
}

fn default_relays() -> Vec<String> {
    DEFAULT_RELAYS
        .iter()
        .map(|relay| (*relay).to_owned())
        .collect()
}

fn default_backups() -> Vec<UiBackup> {
    DEFAULT_BLOSSOM_SERVERS
        .iter()
        .map(|server| UiBackup {
            label: "Blossom fallback".to_owned(),
            state: "configured".to_owned(),
            detail: (*server).to_owned(),
        })
        .collect()
}

fn generated_pubkey() -> String {
    let next = NEXT_SYNTHETIC_PUBKEY.fetch_add(1, Ordering::Relaxed);
    format!("pubkey-{next:016x}")
}

fn local_device(pubkey: String, label: String, state: &str, role: &str) -> UiDevice {
    UiDevice {
        pubkey: pubkey.clone(),
        label,
        role: role.to_owned(),
        state: state.to_owned(),
        detail: pubkey,
        is_online: false,
        can_revoke: false,
        can_appoint_admin: false,
        can_demote_admin: false,
    }
}

fn label_or_default(value: &str) -> String {
    label_or(value, DEFAULT_DEVICE_LABEL)
}

fn label_or(value: &str, fallback: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn request_device_pubkey(request: &str) -> String {
    request
        .split(['&', '?'])
        .find_map(|part| part.strip_prefix("device="))
        .unwrap_or(request)
        .to_owned()
}

fn update_snapshot_link(state: &mut NativeAppState) {
    let owner = state
        .ui
        .account
        .as_ref()
        .map_or("local", |account| account.owner_pubkey.as_str());
    state.ui.snapshot_link = format!("https://drive.iris.to/snapshot/{owner}");
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

    #[test]
    fn profile_actions_populate_mobile_parity_state() {
        let app = FfiApp::new("/tmp/iris-drive".to_owned(), "test".to_owned());

        let state = app.dispatch(NativeAppAction::CreateProfile {
            device_label: "Pixel".to_owned(),
        });

        let account = state.ui.account.as_ref().expect("account exists");
        assert_eq!(account.device_label, "Pixel");
        assert_eq!(account.authorization_state, "authorized");
        assert!(account.has_owner_signing_authority);
        assert_eq!(state.ui.devices.len(), 1);
        assert_eq!(state.ui.devices[0].label, "Pixel");
        assert_eq!(state.ui.devices[0].role, "admin");
        assert!(state.ui.snapshot_link.contains(&account.owner_pubkey));
        assert!(!state.ui.relays.is_empty());
        assert!(!state.ui.backups.is_empty());
        assert_eq!(state.ui.paths.data_dir, "/tmp/iris-drive");

        let state = app.dispatch(NativeAppAction::StartSync);
        assert!(state.ui.sync.running);
        assert_eq!(state.ui.sync.status, "running");

        let state = app.dispatch(NativeAppAction::StopSync);
        assert!(!state.ui.sync.running);
        assert_eq!(state.ui.sync.status, "paused");
    }

    #[test]
    fn link_action_tracks_pending_approval() {
        let app = FfiApp::new("/tmp/iris-drive".to_owned(), "test".to_owned());

        let state = app.dispatch(NativeAppAction::LinkDevice {
            owner_pubkey: "owner-pubkey".to_owned(),
            device_label: "iPhone".to_owned(),
        });

        let account = state.ui.account.expect("account exists");
        assert_eq!(account.owner_pubkey, "owner-pubkey");
        assert_eq!(account.device_label, "iPhone");
        assert_eq!(account.authorization_state, "awaiting_approval");
        assert!(!account.has_owner_signing_authority);
        assert!(account.device_link_request.contains("device="));
        assert_eq!(state.ui.devices[0].role, "member");
    }

    #[test]
    fn owner_can_approve_and_revoke_linked_devices() {
        let app = FfiApp::new("/tmp/iris-drive".to_owned(), "test".to_owned());

        let _ = app.dispatch(NativeAppAction::CreateProfile {
            device_label: "Mac".to_owned(),
        });
        let state = app.dispatch(NativeAppAction::ApproveDevice {
            request: "iris-drive://device-link?owner=owner&device=device-b".to_owned(),
            label: "Phone".to_owned(),
        });

        assert!(state.ui.devices.iter().any(|device| {
            device.pubkey == "device-b"
                && device.label == "Phone"
                && device.role == "member"
                && device.can_revoke
                && device.can_appoint_admin
        }));

        let state = app.dispatch(NativeAppAction::AppointAdmin {
            device_pubkey: "device-b".to_owned(),
        });
        assert!(state.ui.devices.iter().any(|device| {
            device.pubkey == "device-b"
                && device.role == "admin"
                && device.can_demote_admin
                && !device.can_appoint_admin
        }));

        let state = app.dispatch(NativeAppAction::DemoteAdmin {
            device_pubkey: "device-b".to_owned(),
        });
        assert!(state.ui.devices.iter().any(|device| {
            device.pubkey == "device-b"
                && device.role == "member"
                && !device.can_demote_admin
                && device.can_appoint_admin
        }));

        let state = app.dispatch(NativeAppAction::RevokeDevice {
            device_pubkey: "device-b".to_owned(),
        });

        assert!(
            !state
                .ui
                .devices
                .iter()
                .any(|device| device.pubkey == "device-b")
        );
        assert!(state.error.is_empty());
    }
}
