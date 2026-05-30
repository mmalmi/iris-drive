use std::path::Path;
use std::sync::{Arc, Mutex};

use iris_drive_core::config::{DEFAULT_BLOSSOM_SERVERS, DEFAULT_RELAYS};
use iris_drive_core::paths::{config_path_in, key_path_in};
use iris_drive_core::{Account, AppConfig, DeviceAuthorizationState, DeviceRole, Drive};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::{FromBech32, ToBech32};

use crate::actions::NativeAppAction;
use crate::state::{
    NativeAppState, UiAccount, UiBackup, UiDevice, UiDeviceLinkRequest, UiPaths, UiState,
    UiSyncRoot, UiSyncStatus,
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
            NativeAppAction::Logout => self.logout(),
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
        let target = match resolve_device_link_target(owner_pubkey) {
            Ok(target) => target,
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
            target.owner_hex,
            label_option(device_label),
        ) {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("linking device: {error}");
                return;
            }
        };
        let admin_device = target
            .admin_device_hex
            .unwrap_or_else(|| account.state.owner_pubkey.clone());
        let link_secret = if target.link_secret.trim().is_empty() {
            account.state.device_link_secret.clone()
        } else {
            target.link_secret
        };
        if let Err(error) = account.state.queue_outbound_device_link_request(
            admin_device,
            &link_secret,
            unix_now_seconds(),
        ) {
            self.state.error = format!("queueing device link request: {error}");
            return;
        }
        if let Err(error) = self.finish_account_init(&account) {
            self.state.error = error;
        }
    }

    fn logout(&mut self) {
        match iris_drive_core::logout_local_account(Path::new(&self.data_dir)) {
            Ok(_) => {
                self.state.ui.roots.clear();
                self.state.ui.devices.clear();
                self.set_sync_running(false);
            }
            Err(error) => {
                self.state.error = format!("logging out: {error}");
            }
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
            device_link_request: device_link_request_url(account),
            device_link_invite: device_link_invite_url(account),
            inbound_device_link_requests: inbound_device_link_requests(account),
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceLinkTarget {
    owner_hex: String,
    admin_device_hex: Option<String>,
    link_secret: String,
}

fn device_link_request_url(state: &iris_drive_core::AccountState) -> String {
    if state.can_manage_devices()
        || state.authorization_state != DeviceAuthorizationState::AwaitingApproval
    {
        return String::new();
    }
    encode_device_approval_request(
        &state.owner_pubkey,
        &state.device_pubkey,
        state
            .outbound_device_link_request
            .as_ref()
            .and_then(|request| {
                (!request.link_secret.trim().is_empty()).then_some(request.link_secret.as_str())
            })
            .unwrap_or(state.device_link_secret.as_str()),
        state.device_label.as_deref(),
    )
}

fn device_link_invite_url(state: &iris_drive_core::AccountState) -> String {
    if !state.can_manage_devices() {
        return String::new();
    }
    iris_drive_core::device_link_invite::encode_device_link_invite(
        &state.owner_pubkey,
        &state.device_pubkey,
        &state.device_link_secret,
    )
    .unwrap_or_default()
}

fn inbound_device_link_requests(state: &iris_drive_core::AccountState) -> Vec<UiDeviceLinkRequest> {
    if !state.can_manage_devices() {
        return Vec::new();
    }
    state
        .inbound_device_link_requests
        .iter()
        .map(|request| UiDeviceLinkRequest {
            device_pubkey: account_npub(&request.device_pubkey),
            label: request.label.clone().unwrap_or_default(),
            requested_at: request.requested_at,
            request_link: encode_device_approval_request(
                &state.owner_pubkey,
                &request.device_pubkey,
                &request.link_secret,
                request.label.as_deref(),
            ),
        })
        .collect()
}

fn encode_device_approval_request(
    owner_hex: &str,
    device_hex: &str,
    link_secret: &str,
    label: Option<&str>,
) -> String {
    let mut url = format!(
        "iris-drive://device-link?owner={}&device={}",
        account_npub(owner_hex),
        account_npub(device_hex)
    );
    if !link_secret.trim().is_empty() {
        url.push_str("&secret=");
        url.push_str(&percent_encode_component(link_secret.trim()));
    }
    if let Some(label) = label.and_then(label_option) {
        url.push_str("&label=");
        url.push_str(&percent_encode_component(&label));
    }
    url
}

fn resolve_device_link_target(input: &str) -> Result<DeviceLinkTarget, String> {
    if let Some(target) = decode_device_link_invite(input)? {
        return Ok(target);
    }
    Ok(DeviceLinkTarget {
        owner_hex: normalize_pubkey(input)?,
        admin_device_hex: None,
        link_secret: String::new(),
    })
}

fn decode_device_link_invite(request: &str) -> Result<Option<DeviceLinkTarget>, String> {
    iris_drive_core::device_link_invite::parse_device_link_invite(request)
        .map(|target| {
            target.map(|target| DeviceLinkTarget {
                owner_hex: target.owner_hex,
                admin_device_hex: Some(target.admin_device_hex),
                link_secret: target.link_secret,
            })
        })
        .map_err(|error| error.to_string())
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
        let label = query_value(query, "label");
        return Ok((normalize_pubkey(&owner)?, normalize_pubkey(&device)?, label));
    }
    let device = normalize_pubkey(request)?;
    Ok((String::new(), device, None))
}

fn query_value(query: &str, name: &str) -> Option<String> {
    query.split('&').find_map(|part| {
        let (key, value) = part.split_once('=')?;
        (key == name && !value.is_empty()).then(|| percent_decode_component(value))
    })
}

fn percent_encode_component(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0f));
        }
    }
    encoded
}

fn percent_decode_component(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let Some(hi) = bytes.get(index + 1).copied().and_then(hex_value) else {
                output.push(bytes[index]);
                index += 1;
                continue;
            };
            let Some(lo) = bytes.get(index + 2).copied().and_then(hex_value) else {
                output.push(bytes[index]);
                index += 1;
                continue;
            };
            output.push((hi << 4) | lo);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(output).unwrap_or_default()
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'A' + value - 10) as char,
        _ => '0',
    }
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
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
mod tests;
