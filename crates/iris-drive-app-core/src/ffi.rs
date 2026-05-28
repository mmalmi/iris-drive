use std::path::Path;
use std::sync::{Arc, Mutex};

use iris_drive_core::config::{DEFAULT_BLOSSOM_SERVERS, DEFAULT_RELAYS};
use iris_drive_core::paths::{config_path_in, key_path_in};
use iris_drive_core::{Account, AppConfig, DeviceAuthorizationState, DeviceRole, Drive};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::{FromBech32, ToBech32};

use crate::actions::NativeAppAction;
use crate::state::{
    NativeAppState, UiAccount, UiBackup, UiDevice, UiPaths, UiState, UiSyncRoot, UiSyncStatus,
};

const DEFAULT_ROOT_STATUS: &str = "SAF provider root";

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
        state.ui.paths = paths_for(&data_dir);
        state.ui.sync = UiSyncStatus {
            running: false,
            status: "paused".to_owned(),
        };

        let mut runtime = Self {
            state,
            data_dir,
            app_version,
        };
        runtime.reload_from_disk();
        runtime
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
            NativeAppAction::ResetRelays => self.reset_relays(),
            NativeAppAction::StartSync | NativeAppAction::RestartSync => {
                self.set_sync_running(true);
            }
            NativeAppAction::StopSync => self.set_sync_running(false),
            NativeAppAction::AddRoot { name, local_path } => self.add_root(&name, &local_path),
            NativeAppAction::RemoveRoot { name } => self.remove_root(&name),
        }
        self.reload_from_disk_preserving_error();
    }

    fn create_profile(&mut self, device_label: &str) {
        if self.initialized() {
            "already initialized".clone_into(&mut self.state.error);
            return;
        }
        let account = match Account::create(Path::new(&self.data_dir), label_option(device_label)) {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("creating account: {error}");
                return;
            }
        };
        if let Err(error) = self.finish_account_init(&account) {
            self.state.error = error;
        }
    }

    fn restore_profile(&mut self, secret: &str, device_label: &str) {
        if secret.trim().is_empty() {
            "owner secret is required".clone_into(&mut self.state.error);
            return;
        }
        if self.initialized() {
            "already initialized".clone_into(&mut self.state.error);
            return;
        }
        let account = match Account::restore(
            Path::new(&self.data_dir),
            secret.trim(),
            label_option(device_label),
        ) {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("restoring account: {error}");
                return;
            }
        };
        if let Err(error) = self.finish_account_init(&account) {
            self.state.error = error;
        }
    }

    fn link_device(&mut self, owner_pubkey: &str, device_label: &str) {
        let owner_pubkey = match normalize_pubkey(owner_pubkey) {
            Ok(owner) => owner,
            Err(error) => {
                self.state.error = error;
                return;
            }
        };
        if self.initialized() {
            "already initialized".clone_into(&mut self.state.error);
            return;
        }
        let mut account = match Account::link(
            Path::new(&self.data_dir),
            owner_pubkey,
            label_option(device_label),
        ) {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("linking device: {error}");
                return;
            }
        };
        let admin_device = account.state.owner_pubkey.clone();
        if let Err(error) = account
            .state
            .queue_outbound_device_link_request(admin_device, unix_now_seconds())
        {
            self.state.error = format!("queueing device link request: {error}");
            return;
        }
        if let Err(error) = self.finish_account_init(&account) {
            self.state.error = error;
        }
    }

    fn approve_device(&mut self, request: &str, label: &str) {
        let request = request.trim();
        if request.is_empty() {
            "device request is required".clone_into(&mut self.state.error);
            return;
        }
        let (owner_hex, device_hex, request_label) = match decode_device_approval_request(request) {
            Ok(value) => value,
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
        let Some(state) = config.account.clone() else {
            "owner profile is required to approve devices".clone_into(&mut self.state.error);
            return;
        };
        if !state.can_manage_devices() {
            "owner profile is required to approve devices".clone_into(&mut self.state.error);
            return;
        }
        if !owner_hex.is_empty() && state.owner_pubkey != owner_hex {
            "device request is for a different owner".clone_into(&mut self.state.error);
            return;
        }
        let label = label_option(label).or(request_label);
        let mut account = match Account::load(state, Path::new(&self.data_dir)) {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("loading account: {error}");
                return;
            }
        };
        if let Err(error) = account.approve_device(&device_hex, label) {
            self.state.error = format!("approving device: {error}");
            return;
        }
        config.account = Some(account.state);
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
        }
    }

    fn revoke_device(&mut self, device_pubkey: &str) {
        let device_pubkey = match normalize_pubkey(device_pubkey) {
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
        let Some(state) = config.account.clone() else {
            "owner profile is required to revoke devices".clone_into(&mut self.state.error);
            return;
        };
        if state.device_pubkey == device_pubkey {
            "cannot revoke this device from itself".clone_into(&mut self.state.error);
            return;
        }
        let mut account = match Account::load(state, Path::new(&self.data_dir)) {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("loading account: {error}");
                return;
            }
        };
        if let Err(error) = account.revoke_device(&device_pubkey) {
            self.state.error = format!("revoking device: {error}");
            return;
        }
        config.account = Some(account.state);
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
        }
    }

    fn set_device_admin_role(&mut self, device_pubkey: &str, make_admin: bool) {
        let device_pubkey = match normalize_pubkey(device_pubkey) {
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
        let Some(state) = config.account.clone() else {
            "admin profile is required to manage device admins".clone_into(&mut self.state.error);
            return;
        };
        let mut account = match Account::load(state, Path::new(&self.data_dir)) {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("loading account: {error}");
                return;
            }
        };
        let result = if make_admin {
            account.appoint_admin(&device_pubkey)
        } else {
            account.demote_admin(&device_pubkey)
        };
        if let Err(error) = result {
            self.state.error = format!("updating device role: {error}");
            return;
        }
        config.account = Some(account.state);
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
        }
    }

    fn add_relay(&mut self, url: &str) {
        let url = url.trim();
        if url.is_empty() {
            "relay URL is required".clone_into(&mut self.state.error);
            return;
        }
        let mut config = match self.load_config() {
            Ok(config) => config,
            Err(error) => {
                self.state.error = error;
                return;
            }
        };
        if !config.relays.iter().any(|existing| existing == url) {
            config.relays.push(url.to_owned());
        }
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
        }
    }

    fn remove_relay(&mut self, url: &str) {
        let url = url.trim();
        let mut config = match self.load_config() {
            Ok(config) => config,
            Err(error) => {
                self.state.error = error;
                return;
            }
        };
        let before = config.relays.len();
        config.relays.retain(|relay| relay != url);
        if before == config.relays.len() {
            self.state.error = format!("relay not found: {url}");
            return;
        }
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
        }
    }

    fn reset_relays(&mut self) {
        let mut config = match self.load_config() {
            Ok(config) => config,
            Err(error) => {
                self.state.error = error;
                return;
            }
        };
        config.relays = default_relays();
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
        }
    }

    fn initialized(&self) -> bool {
        key_path_in(Path::new(&self.data_dir)).exists()
            && self
                .load_config()
                .ok()
                .and_then(|config| config.account)
                .is_some()
    }

    fn load_config(&self) -> Result<AppConfig, String> {
        AppConfig::load_or_default(config_path_in(Path::new(&self.data_dir)))
            .map_err(|error| format!("loading config: {error}"))
    }

    fn finish_account_init(&self, account: &Account) -> Result<(), String> {
        let mut config = self.load_config()?;
        config.account = Some(account.state.clone());
        if config.drive(iris_drive_core::PRIMARY_DRIVE_ID).is_none() {
            config.upsert_drive(Drive::primary(&account.state.owner_pubkey));
        }
        config
            .save(config_path_in(Path::new(&self.data_dir)))
            .map_err(|error| format!("saving config: {error}"))
    }

    fn reload_from_disk_preserving_error(&mut self) {
        let error = self.state.error.clone();
        self.reload_from_disk();
        self.state.error = error;
    }

    fn reload_from_disk(&mut self) {
        let paths = paths_for(&self.data_dir);
        let sync = self.state.ui.sync.clone();
        let previous_roots = self.state.ui.roots.clone();
        self.state.ui = UiState {
            relays: default_relays(),
            backups: default_backups(),
            paths,
            sync,
            snapshot_link: "https://drive.iris.to/snapshot/local".to_owned(),
            ..UiState::default()
        };

        let Ok(config) = self.load_config() else {
            return;
        };
        self.state.ui.relays = if config.relays.is_empty() {
            default_relays()
        } else {
            config.relays.clone()
        };
        self.state.ui.backups = config
            .blossom_servers
            .iter()
            .map(|server| UiBackup {
                label: "Blossom fallback".to_owned(),
                state: "configured".to_owned(),
                detail: server.clone(),
            })
            .collect();
        self.state.ui.roots = if config.drives.is_empty() {
            previous_roots
        } else {
            config
                .drives
                .iter()
                .map(|drive| UiSyncRoot {
                    name: drive.display_name.clone(),
                    local_path: self.data_dir.clone(),
                    status: DEFAULT_ROOT_STATUS.to_owned(),
                })
                .collect()
        };

        let Some(account) = config.account.as_ref() else {
            update_snapshot_link(&mut self.state);
            return;
        };
        self.state.ui.account = Some(UiAccount {
            owner_pubkey: account_npub(&account.owner_pubkey),
            device_pubkey: account_npub(&account.device_pubkey),
            device_label: account.device_label.clone().unwrap_or_default(),
            authorization_state: authorization_state_label(account).to_owned(),
            has_owner_signing_authority: account.has_owner_signing_authority,
            device_link_request: encode_device_approval_request(
                &account.owner_pubkey,
                &account.device_pubkey,
                account.device_label.as_deref(),
            ),
        });
        self.state.ui.devices = devices_from_account(account, self.state.ui.sync.running);
        self.refresh_device_actions();
        update_snapshot_link(&mut self.state);
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
            device.is_current_device = is_current;
        }
    }

    fn set_sync_running(&mut self, running: bool) {
        self.state.ui.sync = UiSyncStatus {
            running,
            status: if running { "running" } else { "paused" }.to_owned(),
        };
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

fn label_option(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn normalize_pubkey(input: &str) -> Result<String, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("public key is required".to_owned());
    }
    if trimmed.starts_with("npub1") {
        return PublicKey::from_bech32(trimmed)
            .map(|pubkey| pubkey.to_hex())
            .map_err(|error| format!("parsing npub: {error}"));
    }
    if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(trimmed.to_owned());
    }
    Err(format!(
        "expected npub1... or 64-char hex pubkey, got {trimmed}"
    ))
}

fn account_npub(hex: &str) -> String {
    PublicKey::from_hex(hex)
        .ok()
        .and_then(|pubkey| pubkey.to_bech32().ok())
        .unwrap_or_else(|| hex.to_owned())
}

fn authorization_state_label(state: &iris_drive_core::AccountState) -> &'static str {
    match state.authorization_state {
        DeviceAuthorizationState::Authorized => "authorized",
        DeviceAuthorizationState::AwaitingApproval => "awaiting_approval",
        DeviceAuthorizationState::Revoked => "revoked",
    }
}

fn device_role_label(role: DeviceRole) -> &'static str {
    match role {
        DeviceRole::Admin => "admin",
        DeviceRole::Member => "member",
    }
}

fn devices_from_account(
    state: &iris_drive_core::AccountState,
    sync_running: bool,
) -> Vec<UiDevice> {
    let Some(app_keys) = state.app_keys.as_ref() else {
        return vec![UiDevice {
            pubkey: account_npub(&state.device_pubkey),
            label: state.device_label.clone().unwrap_or_default(),
            role: "member".to_owned(),
            state: authorization_state_label(state).to_owned(),
            detail: account_npub(&state.device_pubkey),
            is_current_device: true,
            is_online: sync_running,
            can_revoke: false,
            can_appoint_admin: false,
            can_demote_admin: false,
        }];
    };

    app_keys
        .devices
        .iter()
        .map(|device| {
            let is_current = device.pubkey == state.device_pubkey;
            let role = device_role_label(device.role).to_owned();
            UiDevice {
                pubkey: account_npub(&device.pubkey),
                label: device.label.clone().unwrap_or_default(),
                state: if role == "admin" {
                    "Admin"
                } else {
                    "Authorized"
                }
                .to_owned(),
                role,
                detail: account_npub(&device.pubkey),
                is_current_device: is_current,
                is_online: sync_running,
                can_revoke: false,
                can_appoint_admin: false,
                can_demote_admin: false,
            }
        })
        .collect()
}

fn encode_device_approval_request(
    owner_hex: &str,
    device_hex: &str,
    label: Option<&str>,
) -> String {
    let mut url = format!(
        "iris-drive://device-link?owner={}&device={}",
        account_npub(owner_hex),
        account_npub(device_hex)
    );
    if let Some(label) = label.and_then(label_option) {
        url.push_str("&label=");
        url.push_str(&label.replace(' ', "%20"));
    }
    url
}

fn decode_device_approval_request(
    request: &str,
) -> Result<(String, String, Option<String>), String> {
    if request.starts_with("iris-drive://device-link")
        || request.starts_with("https://drive.iris.to/device-link")
    {
        let query = request.split_once('?').map_or("", |(_, query)| query);
        let owner = query_value(query, "owner")
            .ok_or_else(|| "device request is missing owner".to_owned())?;
        let device = query_value(query, "device")
            .ok_or_else(|| "device request is missing device".to_owned())?;
        let label = query_value(query, "label").map(|label| label.replace("%20", " "));
        return Ok((normalize_pubkey(&owner)?, normalize_pubkey(&device)?, label));
    }
    let device = normalize_pubkey(request)?;
    Ok((String::new(), device, None))
}

fn query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == name && !value.is_empty()).then(|| value.to_owned())
    })
}

fn unix_now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
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
        let dir = tempfile::tempdir().unwrap();
        let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

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
        let dir = tempfile::tempdir().unwrap();
        let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

        let state = app.dispatch(NativeAppAction::AddRoot {
            name: String::new(),
            local_path: "/virtual/iris".to_owned(),
        });

        assert!(state.ui.roots.is_empty());
        assert_eq!(state.error, "root name is required");
    }

    #[test]
    fn profile_actions_populate_mobile_parity_state() {
        let dir = tempfile::tempdir().unwrap();
        let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

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
        assert_eq!(state.ui.paths.data_dir, dir.path().display().to_string());

        let state = app.dispatch(NativeAppAction::StartSync);
        assert!(state.ui.sync.running);
        assert_eq!(state.ui.sync.status, "running");

        let state = app.dispatch(NativeAppAction::StopSync);
        assert!(!state.ui.sync.running);
        assert_eq!(state.ui.sync.status, "paused");
    }

    #[test]
    fn link_action_tracks_pending_approval() {
        let owner_dir = tempfile::tempdir().unwrap();
        let owner_app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());
        let owner = owner_app.dispatch(NativeAppAction::CreateProfile {
            device_label: "Owner".to_owned(),
        });
        let owner_npub = owner.ui.account.unwrap().owner_pubkey;

        let dir = tempfile::tempdir().unwrap();
        let app = FfiApp::new(dir.path().display().to_string(), "test".to_owned());

        let state = app.dispatch(NativeAppAction::LinkDevice {
            owner_pubkey: owner_npub.clone(),
            device_label: "iPhone".to_owned(),
        });

        let account = state.ui.account.expect("account exists");
        assert_eq!(account.owner_pubkey, owner_npub);
        assert_eq!(account.device_label, "iPhone");
        assert_eq!(account.authorization_state, "awaiting_approval");
        assert!(!account.has_owner_signing_authority);
        assert!(account.device_link_request.contains("device="));
        assert_eq!(state.ui.devices[0].role, "member");
    }

    #[test]
    fn owner_can_approve_and_revoke_linked_devices() {
        let owner_dir = tempfile::tempdir().unwrap();
        let app = FfiApp::new(owner_dir.path().display().to_string(), "test".to_owned());

        let owner = app.dispatch(NativeAppAction::CreateProfile {
            device_label: "Mac".to_owned(),
        });
        let owner_npub = owner.ui.account.unwrap().owner_pubkey;
        let linked_dir = tempfile::tempdir().unwrap();
        let linked_app = FfiApp::new(linked_dir.path().display().to_string(), "test".to_owned());
        let linked = linked_app.dispatch(NativeAppAction::LinkDevice {
            owner_pubkey: owner_npub,
            device_label: "Phone".to_owned(),
        });
        let request = linked.ui.account.unwrap().device_link_request;
        let linked_device = linked.ui.devices[0].pubkey.clone();
        let state = app.dispatch(NativeAppAction::ApproveDevice {
            request,
            label: "Phone".to_owned(),
        });

        assert!(state.ui.devices.iter().any(|device| {
            device.pubkey == linked_device
                && device.label == "Phone"
                && device.role == "member"
                && device.can_revoke
                && device.can_appoint_admin
        }));

        let state = app.dispatch(NativeAppAction::AppointAdmin {
            device_pubkey: linked_device.clone(),
        });
        assert!(state.ui.devices.iter().any(|device| {
            device.pubkey == linked_device
                && device.role == "admin"
                && device.can_demote_admin
                && !device.can_appoint_admin
        }));

        let state = app.dispatch(NativeAppAction::DemoteAdmin {
            device_pubkey: linked_device.clone(),
        });
        assert!(state.ui.devices.iter().any(|device| {
            device.pubkey == linked_device
                && device.role == "member"
                && !device.can_demote_admin
                && device.can_appoint_admin
        }));

        let state = app.dispatch(NativeAppAction::RevokeDevice {
            device_pubkey: linked_device.clone(),
        });

        assert!(
            !state
                .ui
                .devices
                .iter()
                .any(|device| device.pubkey == linked_device)
        );
        assert!(state.error.is_empty());
    }
}
