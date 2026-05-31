use std::collections::BTreeMap;
#[cfg(not(test))]
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[cfg(not(test))]
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use hashtree_core::{Cid, NHashData, nhash_encode_full};
use hashtree_provider::{HashTreeProviderFs, ItemKind, ProviderFs};
use iris_drive_core::config::{DEFAULT_BLOSSOM_SERVERS, DEFAULT_RELAYS};
#[cfg(not(test))]
use iris_drive_core::device_link_transport::{
    DEVICE_LINK_REQUEST_APP_TOPIC, DEVICE_LINK_ROSTER_ACK_APP_TOPIC, DEVICE_LINK_ROSTER_APP_TOPIC,
    DeviceLinkRequestFrame, DeviceLinkRosterAckFrame, DeviceLinkRosterFrame,
    pending_device_link_request_frame,
};
use iris_drive_core::paths::{config_path_in, key_path_in};
use iris_drive_core::{Account, AppConfig, DeviceAuthorizationState, DeviceRole, Drive};
#[cfg(not(test))]
use nostr_sdk::JsonUtil;
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::{FromBech32, ToBech32};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::actions::NativeAppAction;
use crate::state::{
    NativeAppState, UiAccount, UiBackup, UiDevice, UiDeviceLinkRequest, UiPaths, UiState,
    UiSyncRoot, UiSyncStatus,
};

#[cfg(target_os = "android")]
#[path = "ffi_android_test_support.rs"]
mod android_test_support;
#[cfg(target_os = "android")]
pub(crate) use android_test_support::native_apply_owner_snapshot_for_test_json;

const DEFAULT_ROOT_STATUS: &str = "SAF provider root";
const NATIVE_FIPS_STATUS_FILE_NAME: &str = "native-fips-status.json";
const NATIVE_FIPS_STATUS_FRESH_SECS: u64 = 20;
const MIN_PROVIDER_DISPLAY_UNIX_SECS: i64 = 946_684_800;
const PROVIDER_IMPORT_RETRY_DELAYS_MS: &[u64] = &[25, 50, 100, 200, 400];
const NATIVE_SYNC_RELAY_TIMEOUT_SECS: u64 = 10;
#[cfg(not(test))]
const DEVICE_LINK_REQUEST_RETRY_SECS: u64 = 10;
#[cfg(not(test))]
const DEVICE_LINK_ROSTER_RETRY_SECS: u64 = 2;
#[cfg(not(test))]
const DEVICE_LINK_EXCHANGE_TICK_SECS: u64 = 1;

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
        install_rustls_crypto_provider();
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
    #[cfg(not(test))]
    device_link_exchange_running: Arc<AtomicBool>,
}

impl NativeAppRuntime {
    fn new(data_dir: String, app_version: String) -> Self {
        let mut state = NativeAppState::default();
        state.ui.paths = paths_for(&data_dir);
        state.ui.sync = UiSyncStatus {
            running: true,
            status: "running".to_owned(),
        };

        let mut runtime = Self {
            state,
            data_dir,
            app_version,
            #[cfg(not(test))]
            device_link_exchange_running: Arc::new(AtomicBool::new(false)),
        };
        runtime.reload_from_disk();
        runtime.start_device_link_exchange_if_needed();
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
            NativeAppAction::ResetInvite => self.reset_invite(),
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
            NativeAppAction::StartSync | NativeAppAction::RestartSync => self.start_sync(),
            NativeAppAction::StopSync => self.set_sync_running(false),
            NativeAppAction::AddRoot { name, local_path } => self.add_root(&name, &local_path),
            NativeAppAction::RemoveRoot { name } => self.remove_root(&name),
            NativeAppAction::ImportFile {
                display_name,
                source_path,
            } => {
                self.import_file(&display_name, &source_path);
            }
        }
        self.reload_from_disk_preserving_error();
        self.start_device_link_exchange_if_needed();
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
        } else {
            self.set_sync_running(true);
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
        } else {
            self.set_sync_running(true);
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
        if self.initialized() && !self.current_device_is_revoked() {
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
        } else {
            self.set_sync_running(true);
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

    fn reset_invite(&mut self) {
        let mut config = match self.load_config() {
            Ok(config) => config,
            Err(error) => {
                self.state.error = error;
                return;
            }
        };
        let Some(state) = config.account.as_mut() else {
            "owner profile is required to reset invites".clone_into(&mut self.state.error);
            return;
        };
        if !state.can_manage_devices() {
            "owner profile is required to reset invites".clone_into(&mut self.state.error);
            return;
        }
        state.reset_device_link_secret();
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

    fn current_authorization_state(&self) -> Option<DeviceAuthorizationState> {
        let mut account = self.load_config().ok()?.account?;
        account.recompute_authorization();
        Some(account.authorization_state)
    }

    fn current_device_is_revoked(&self) -> bool {
        self.state
            .ui
            .account
            .as_ref()
            .is_some_and(|account| account.authorization_state == "revoked")
            || self.current_authorization_state() == Some(DeviceAuthorizationState::Revoked)
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

    fn start_device_link_exchange_if_needed(&mut self) {
        #[cfg(not(test))]
        {
            let Ok(config) = self.load_config() else {
                return;
            };
            if config.account.is_none() {
                return;
            }
            if self
                .device_link_exchange_running
                .swap(true, Ordering::AcqRel)
            {
                return;
            }

            let data_dir = self.data_dir.clone();
            let running = self.device_link_exchange_running.clone();
            std::thread::spawn(move || {
                if let Err(error) = run_device_link_exchange(&data_dir) {
                    tracing::warn!(error = %error, "native device-link FIPS exchange stopped");
                }
                running.store(false, Ordering::Release);
            });
        }
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
            snapshot_link: String::new(),
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
                label: "Blossom remote".to_owned(),
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

        let Some(raw_account) = config.account.as_ref() else {
            return;
        };
        let mut account = raw_account.clone();
        account.recompute_authorization();
        self.state.ui.account = Some(UiAccount {
            owner_pubkey: account_npub(&account.owner_pubkey),
            device_pubkey: account_npub(&account.device_pubkey),
            device_label: account.device_label.clone().unwrap_or_default(),
            authorization_state: authorization_state_label(&account).to_owned(),
            has_owner_signing_authority: account.has_owner_signing_authority,
            device_link_request: device_link_request_url(&account),
            device_link_invite: device_link_invite_url(&account),
            inbound_device_link_requests: inbound_device_link_requests(&account),
        });
        if account.authorization_state == DeviceAuthorizationState::Revoked {
            self.set_sync_running(false);
            self.state.ui.roots.clear();
            self.state.ui.devices.clear();
            self.state.ui.snapshot_link.clear();
            return;
        }
        let fips_status = load_native_fips_status(Path::new(&self.data_dir));
        self.state.ui.devices = devices_from_account(&account, fips_status.as_ref());
        self.refresh_device_actions();
        update_snapshot_link(&mut self.state, &config);
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

    fn start_sync(&mut self) {
        self.set_sync_running(true);
        match run_native_sync_once(&self.data_dir) {
            Ok(report) => {
                self.state.ui.sync.status = native_sync_status_label(&report).to_owned();
            }
            Err(error) => {
                self.state.ui.sync.status = "sync error".to_owned();
                self.state.error = format!("syncing drive: {error:#}");
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

    fn import_file(&mut self, display_name: &str, source_path: &str) {
        if !self.initialized() {
            "profile is required before importing files".clone_into(&mut self.state.error);
            return;
        }
        if source_path.trim().is_empty() {
            "source file is required".clone_into(&mut self.state.error);
            return;
        }
        if let Err(error) =
            native_provider_import_shared_file(&self.data_dir, display_name, source_path)
        {
            self.state.error = format!("importing shared file: {error:#}");
        }
    }
}

#[derive(Debug, Serialize)]
struct ProviderListEntry {
    path: String,
    kind: &'static str,
    size: u64,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified_at: Option<i64>,
}

pub(crate) fn native_provider_list_json(data_dir: &str) -> serde_json::Value {
    match run_native_provider_list(data_dir) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn native_provider_read_json(
    data_dir: &str,
    path: &str,
    output_path: &str,
) -> serde_json::Value {
    match run_native_provider_read(data_dir, path, output_path) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn native_provider_write_json(
    data_dir: &str,
    path: &str,
    source_path: &str,
) -> serde_json::Value {
    match run_native_provider_write(data_dir, path, source_path) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn native_provider_mkdir_json(data_dir: &str, path: &str) -> serde_json::Value {
    match run_native_provider_mkdir(data_dir, path) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn native_provider_delete_json(data_dir: &str, path: &str) -> serde_json::Value {
    match run_native_provider_delete(data_dir, path) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn native_provider_rename_json(
    data_dir: &str,
    old_path: &str,
    new_path: &str,
) -> serde_json::Value {
    match run_native_provider_rename(data_dir, old_path, new_path) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

pub(crate) fn native_provider_import_shared_file_json(
    data_dir: &str,
    display_name: &str,
    source_path: &str,
) -> serde_json::Value {
    match native_provider_import_shared_file(data_dir, display_name, source_path) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

fn native_provider_import_shared_file(
    data_dir: &str,
    display_name: &str,
    source_path: &str,
) -> anyhow::Result<serde_json::Value> {
    let display_name = sanitized_provider_file_name(display_name);
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        let modified_at_by_path = BTreeMap::new();
        let entries = provider_entries(&provider, &modified_at_by_path).await?;
        let path = unique_provider_path(&entries, &display_name);
        let bytes = std::fs::read(source_path)
            .with_context(|| format!("reading {}", Path::new(source_path).display()))?;
        write_provider_file(&provider, &path, &bytes).await?;
        import_provider_mutation(&mut daemon, &provider, &path, Some(visible_root)).await
    })
}

fn run_native_provider_list(data_dir: &str) -> anyhow::Result<serde_json::Value> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let daemon = iris_drive_core::Daemon::open(data_dir)
            .with_context(|| format!("opening daemon at {}", Path::new(data_dir).display()))?;
        let visible_view = iris_drive_core::primary_merged_view(daemon.tree(), daemon.config())
            .await
            .context("building provider view")?;
        let modified_at_by_path = provider_modified_at_index(&visible_view);
        let visible_root = iris_drive_core::primary_merged_root(daemon.tree(), daemon.config())
            .await
            .context("building provider root")?;
        let provider =
            HashTreeProviderFs::open(daemon.tree_handle(), visible_root.root_cid.clone())
                .await
                .context("opening provider root")?;
        let entries = provider_entries(&provider, &modified_at_by_path).await?;
        Ok(json!({
            "anchor": provider.anchor().await.as_str(),
            "root_cid": visible_root.root_cid.to_string(),
            "entries": entries,
        }))
    })
}

fn run_native_provider_read(
    data_dir: &str,
    path: &str,
    output_path: &str,
) -> anyhow::Result<serde_json::Value> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let path = normalize_provider_path(path)?;
        let (_daemon, provider, _visible_root) = native_provider(data_dir).await?;
        let item = provider.item(&path).await?;
        if item.kind == ItemKind::Directory {
            anyhow::bail!("cannot read directory: {path}");
        }
        let bytes = provider.read(&path, 0, item.size).await?;
        let output = PathBuf::from(output_path);
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::write(&output, bytes).with_context(|| format!("writing {}", output.display()))?;
        Ok(json!({
            "path": path,
            "output": output.display().to_string(),
            "size": item.size,
        }))
    })
}

fn run_native_provider_write(
    data_dir: &str,
    path: &str,
    source_path: &str,
) -> anyhow::Result<serde_json::Value> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let path = normalize_provider_path(path)?;
        let bytes = std::fs::read(source_path)
            .with_context(|| format!("reading {}", Path::new(source_path).display()))?;
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        write_provider_file(&provider, &path, &bytes).await?;
        import_provider_mutation(&mut daemon, &provider, &path, Some(visible_root)).await
    })
}

fn run_native_provider_mkdir(data_dir: &str, path: &str) -> anyhow::Result<serde_json::Value> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let path = normalize_provider_path(path)?;
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        create_provider_dir(&provider, &path).await?;
        import_provider_mutation(&mut daemon, &provider, &path, Some(visible_root)).await
    })
}

fn run_native_provider_delete(data_dir: &str, path: &str) -> anyhow::Result<serde_json::Value> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let path = normalize_provider_path(path)?;
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        delete_provider_path(&provider, &path).await?;
        import_provider_mutation(&mut daemon, &provider, &path, Some(visible_root)).await
    })
}

fn run_native_provider_rename(
    data_dir: &str,
    old_path: &str,
    new_path: &str,
) -> anyhow::Result<serde_json::Value> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let old_path = normalize_provider_path(old_path)?;
        let new_path = normalize_provider_path(new_path)?;
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        rename_provider_path(&provider, &old_path, &new_path).await?;
        import_provider_mutation(&mut daemon, &provider, &new_path, Some(visible_root)).await
    })
}

fn native_provider_runtime() -> anyhow::Result<tokio::runtime::Runtime> {
    install_rustls_crypto_provider();
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building native provider runtime")
}

fn install_rustls_crypto_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

async fn native_provider(
    data_dir: &str,
) -> anyhow::Result<(
    iris_drive_core::Daemon,
    HashTreeProviderFs<hashtree_fs::FsBlobStore>,
    hashtree_core::Cid,
)> {
    let daemon = iris_drive_core::Daemon::open(data_dir)
        .with_context(|| format!("opening daemon at {}", Path::new(data_dir).display()))?;
    let visible = iris_drive_core::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .context("building provider root")?;
    let provider = HashTreeProviderFs::open(daemon.tree_handle(), visible.root_cid.clone())
        .await
        .context("opening provider root")?;
    Ok((daemon, provider, visible.root_cid))
}

async fn provider_entries<P>(
    provider: &P,
    modified_at_by_path: &BTreeMap<String, i64>,
) -> anyhow::Result<Vec<ProviderListEntry>>
where
    P: ProviderFs<ItemId = String>,
{
    let mut entries = Vec::new();
    let mut stack = vec![String::new()];
    while let Some(parent) = stack.pop() {
        let mut children = provider.read_dir(&parent).await?;
        children.sort_by(|left, right| left.name.cmp(&right.name));
        for child in children {
            let item = provider.item(&child.id).await?;
            let kind = match item.kind {
                ItemKind::Directory => {
                    stack.push(child.id.clone());
                    "directory"
                }
                ItemKind::File => "file",
            };
            let modified_at = modified_at_by_path.get(&child.id).copied();
            entries.push(ProviderListEntry {
                path: child.id,
                kind,
                size: item.size,
                version: provider.anchor().await.as_str().to_owned(),
                modified_at,
            });
        }
    }
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn provider_modified_at_index(
    view: &iris_drive_core::projection::PrimaryMergedView,
) -> BTreeMap<String, i64> {
    let mut index = BTreeMap::new();
    for entry in &view.view.files {
        remember_provider_modified_at(&mut index, &entry.path, entry.published_at);
        let mut path = entry.path.as_str();
        while let Some((parent, _name)) = path.rsplit_once('/') {
            remember_provider_modified_at(&mut index, parent, entry.published_at);
            path = parent;
        }
    }
    index
}

fn remember_provider_modified_at(index: &mut BTreeMap<String, i64>, path: &str, modified_at: i64) {
    if path.is_empty() || modified_at < MIN_PROVIDER_DISPLAY_UNIX_SECS {
        return;
    }
    index
        .entry(path.to_owned())
        .and_modify(|existing| *existing = (*existing).max(modified_at))
        .or_insert(modified_at);
}

async fn import_provider_mutation<P>(
    daemon: &mut iris_drive_core::Daemon,
    provider: &P,
    changed_path: &str,
    tombstone_base_root: Option<hashtree_core::Cid>,
) -> anyhow::Result<serde_json::Value>
where
    P: ProviderFs<ItemId = String>,
{
    let root = hashtree_core::Cid::parse(provider.anchor().await.as_str())
        .context("reading provider root CID")?;
    let report = import_provider_root_with_retry(daemon, root, tombstone_base_root).await?;
    let publish = publish_current_device_root_best_effort(daemon.config_dir()).await;
    Ok(json!({
        "path": changed_path,
        "root_cid": report.root_cid,
        "file_count": report.file_count,
        "top_level_entries": report.top_level_entries,
        "publish": publish,
    }))
}

async fn import_provider_root_with_retry(
    daemon: &mut iris_drive_core::Daemon,
    root: hashtree_core::Cid,
    tombstone_base_root: Option<hashtree_core::Cid>,
) -> anyhow::Result<iris_drive_core::ImportReport> {
    let mut attempt = 0;
    loop {
        match daemon
            .import_visible_root_with_tombstone_base(root.clone(), tombstone_base_root.clone())
            .await
        {
            Ok(report) => return Ok(report),
            Err(error)
                if attempt < PROVIDER_IMPORT_RETRY_DELAYS_MS.len()
                    && provider_import_error_message_is_retryable(&error.to_string()) =>
            {
                let delay_ms = PROVIDER_IMPORT_RETRY_DELAYS_MS[attempt];
                attempt += 1;
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
}

async fn publish_current_device_root_best_effort(config_dir: &Path) -> serde_json::Value {
    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        publish_current_device_root(config_dir),
    )
    .await
    {
        Ok(Ok(published)) => published,
        Ok(Err(error)) => json!({"published_drive_root": false, "error": format!("{error:#}")}),
        Err(_) => json!({"published_drive_root": false, "error": "publish timed out"}),
    }
}

async fn publish_current_device_root(config_dir: &Path) -> anyhow::Result<serde_json::Value> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(account) = config.account.as_ref() else {
        return Ok(json!({"published_drive_root": false, "error": "account missing"}));
    };
    let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID) else {
        return Ok(json!({"published_drive_root": false, "error": "primary drive missing"}));
    };
    let Some(root) = drive.device_roots.get(&account.device_pubkey) else {
        return Ok(json!({"published_drive_root": false, "error": "device root missing"}));
    };
    let loaded_account =
        Account::load(account.clone(), config_dir).context("loading account keys")?;

    let relays = if config.relays.is_empty() {
        default_relays()
    } else {
        config.relays.clone()
    };
    let client = iris_drive_core::relay_sync::connect(&relays).await?;
    let authorized_devices = authorized_device_pubkeys(account);
    let result = iris_drive_core::relay_sync::publish_drive_root(
        &client,
        loaded_account.device.keys(),
        &account.owner_pubkey,
        &drive.drive_id,
        root,
        &authorized_devices,
    )
    .await;
    let _ = client.disconnect().await;
    let event_id = result?;
    Ok(json!({
        "published_drive_root": true,
        "drive_root_event_id": event_id.to_hex(),
    }))
}

fn run_native_sync_once(data_dir: &str) -> anyhow::Result<iris_drive_core::NetworkSyncReport> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(iris_drive_core::network_sync_once(
        Path::new(data_dir),
        &[],
        std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
    ))
}

#[cfg(test)]
fn run_native_sync_once_with_drive_root_events_for_test(
    config_dir: &Path,
    events: &[nostr_sdk::Event],
) -> anyhow::Result<iris_drive_core::DriveRootEventApplyReport> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
        let report = iris_drive_core::apply_drive_root_events(config_dir, &mut config, events)?;
        config.save(config_path_in(config_dir))?;
        Ok(report)
    })
}

fn native_sync_status_label(report: &iris_drive_core::NetworkSyncReport) -> &'static str {
    if report.fips_download.is_some() || report.blossom_download.is_some() {
        "synced"
    } else if report.drive_root_events_applied > 0 || report.files_root_event_outcome == "applied" {
        "root synced"
    } else {
        "up to date"
    }
}

fn authorized_device_pubkeys(state: &iris_drive_core::AccountState) -> Vec<String> {
    let mut devices: Vec<String> = state
        .app_keys
        .as_ref()
        .map(|snap| {
            snap.devices
                .iter()
                .map(|device| device.pubkey.clone())
                .collect()
        })
        .unwrap_or_default();
    if !devices.contains(&state.device_pubkey) {
        devices.push(state.device_pubkey.clone());
    }
    devices
}

async fn write_provider_file<P>(provider: &P, path: &str, bytes: &[u8]) -> anyhow::Result<()>
where
    P: ProviderFs<ItemId = String>,
{
    let (parent, name) = split_provider_path(path)?;
    ensure_provider_dirs(provider, &parent).await?;
    match provider.item(&path.to_owned()).await {
        Ok(item) if item.kind == ItemKind::Directory => {
            delete_provider_path(provider, path).await?;
            provider.create_file(&parent, &name).await?;
        }
        Ok(_) => {
            provider.truncate(&path.to_owned(), 0).await?;
        }
        Err(_) => {
            provider.create_file(&parent, &name).await?;
        }
    }
    if !bytes.is_empty() {
        provider.write(&path.to_owned(), 0, bytes).await?;
    }
    Ok(())
}

async fn create_provider_dir<P>(provider: &P, path: &str) -> anyhow::Result<()>
where
    P: ProviderFs<ItemId = String>,
{
    let (parent, name) = split_provider_path(path)?;
    ensure_provider_dirs(provider, &parent).await?;
    match provider.item(&path.to_owned()).await {
        Ok(item) if item.kind == ItemKind::Directory => Ok(()),
        Ok(_) => {
            provider.remove(&parent, &name).await?;
            provider.create_dir(&parent, &name).await?;
            Ok(())
        }
        Err(_) => {
            provider.create_dir(&parent, &name).await?;
            Ok(())
        }
    }
}

async fn ensure_provider_dirs<P>(provider: &P, parent: &str) -> anyhow::Result<()>
where
    P: ProviderFs<ItemId = String>,
{
    let mut current = String::new();
    for segment in parent.split('/').filter(|segment| !segment.is_empty()) {
        let next = if current.is_empty() {
            segment.to_owned()
        } else {
            format!("{current}/{segment}")
        };
        match provider.item(&next).await {
            Ok(item) if item.kind == ItemKind::Directory => {}
            Ok(_) => {
                provider.remove(&current, segment).await?;
                provider.create_dir(&current, segment).await?;
            }
            Err(_) => {
                provider.create_dir(&current, segment).await?;
            }
        }
        current = next;
    }
    Ok(())
}

async fn delete_provider_path<P>(provider: &P, path: &str) -> anyhow::Result<()>
where
    P: ProviderFs<ItemId = String>,
{
    let mut directories = Vec::new();
    let mut stack = vec![path.to_owned()];
    while let Some(current) = stack.pop() {
        let item = match provider.item(&current).await {
            Ok(item) => item,
            Err(hashtree_provider::ProviderError::NotFound) => continue,
            Err(error) => return Err(error.into()),
        };
        if item.kind == ItemKind::Directory {
            directories.push(current.clone());
            for child in provider.read_dir(&current).await? {
                stack.push(child.id);
            }
        } else {
            let (parent, name) = split_provider_path(&current)?;
            match provider.remove(&parent, &name).await {
                Ok(()) | Err(hashtree_provider::ProviderError::NotFound) => {}
                Err(error) => return Err(error.into()),
            }
        }
    }
    for directory in directories.into_iter().rev() {
        let (parent, name) = split_provider_path(&directory)?;
        match provider.remove(&parent, &name).await {
            Ok(()) | Err(hashtree_provider::ProviderError::NotFound) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

async fn rename_provider_path<P>(provider: &P, old_path: &str, new_path: &str) -> anyhow::Result<()>
where
    P: ProviderFs<ItemId = String>,
{
    let (old_parent, old_name) = split_provider_path(old_path)?;
    let (new_parent, new_name) = split_provider_path(new_path)?;
    ensure_provider_dirs(provider, &new_parent).await?;
    if provider.item(&new_path.to_owned()).await.is_ok() {
        delete_provider_path(provider, new_path).await?;
    }
    provider
        .rename(&old_parent, &old_name, &new_parent, &new_name)
        .await?;
    Ok(())
}

fn split_provider_path(path: &str) -> anyhow::Result<(String, String)> {
    let path = normalize_provider_path(path)?;
    let Some((parent, name)) = path.rsplit_once('/') else {
        return Ok((String::new(), path));
    };
    Ok((parent.to_owned(), name.to_owned()))
}

fn normalize_provider_path(path: &str) -> anyhow::Result<String> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        anyhow::bail!("provider path is required");
    }
    let mut segments = Vec::new();
    for segment in trimmed.split('/') {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.contains('\\')
            || segment.contains(':')
        {
            anyhow::bail!("unsafe provider path: {path}");
        }
        segments.push(segment);
    }
    Ok(segments.join("/"))
}

fn sanitized_provider_file_name(display_name: &str) -> String {
    let mut name = display_name
        .split(['/', ':', '\\'])
        .map(str::trim)
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .collect::<Vec<_>>()
        .join("_");
    if name.is_empty() {
        "Shared file".clone_into(&mut name);
    }
    name
}

fn unique_provider_path(entries: &[ProviderListEntry], name: &str) -> String {
    let existing = entries
        .iter()
        .map(|entry| entry.path.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut candidate = name.to_owned();
    if !existing.contains(candidate.as_str()) {
        return candidate;
    }

    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Shared file");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let mut index = 2;
    while existing.contains(candidate.as_str()) {
        candidate = format!("{stem} ({index}){extension}");
        index += 1;
    }
    candidate
}

fn provider_import_error_message_is_retryable(message: &str) -> bool {
    message.contains("block not found")
        || message.contains("missing block")
        || message.contains("No such file or directory")
}

#[cfg(not(test))]
fn run_device_link_exchange(data_dir: &str) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| format!("building device-link exchange runtime: {error}"))?;
    let result = runtime.block_on(run_device_link_exchange_async(data_dir));
    if let Err(error) = &result {
        write_native_fips_error(Path::new(data_dir), error);
    }
    result
}

#[cfg(not(test))]
async fn run_device_link_exchange_async(data_dir: &str) -> Result<(), String> {
    let config_dir = Path::new(data_dir);
    let startup_config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    if startup_config.account.is_none() {
        return Ok(());
    }

    let device = iris_drive_core::DeviceIdentity::load(key_path_in(config_dir))
        .map_err(|error| format!("loading device key: {error}"))?;
    let daemon = iris_drive_core::Daemon::open(config_dir)
        .map_err(|error| format!("opening block store: {error}"))?;
    let local = daemon.tree().get_store().clone();
    let sync = iris_drive_core::FipsBlockSync::start(&device, local, &startup_config)
        .await
        .map_err(|error| format!("starting FIPS device-link exchange: {error}"))?;
    if let Err(error) = write_native_fips_status(config_dir, &sync, None).await {
        tracing::warn!(error = %error, "writing native FIPS status failed");
    }
    let mut app_messages = sync.subscribe_app_messages();
    let mut sent_requests = BTreeMap::new();
    let mut sent_rosters = BTreeMap::new();
    let mut acked_rosters = BTreeSet::new();
    let mut direct_roots = iris_drive_core::DirectRootExchange::default();
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(
        DEVICE_LINK_EXCHANGE_TICK_SECS,
    ));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    loop {
        tokio::select! {
            _ = tick.tick() => {
                let _ = drive_device_link_exchange_tick(
                    config_dir,
                    &sync,
                    &mut sent_requests,
                    &mut sent_rosters,
                    &acked_rosters,
                ).await?;
                if let Err(error) = direct_roots.announce_current_state(config_dir, &sync).await {
                    tracing::warn!(error = %error, "native direct-root FIPS exchange failed");
                }
                if let Err(error) = direct_roots.drain_mesh_events(config_dir, &sync).await {
                    tracing::warn!(error = %error, "native direct-root FIPS mesh drain failed");
                }
            }
            message = app_messages.recv() => {
                match message {
                    Ok(message) => {
                        if let Err(error) = handle_native_device_link_app_message(
                            config_dir,
                            &sync,
                            &message,
                            &mut acked_rosters,
                        ).await {
                            tracing::warn!(error = %error, topic = message.topic, "handling native device-link FIPS message failed");
                            continue;
                        }
                        if let Err(error) = direct_roots.handle_app_message(
                            config_dir,
                            &sync,
                            &message,
                        ).await {
                            tracing::warn!(error = %error, topic = message.topic, "handling native direct-root FIPS message failed");
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "native device-link FIPS receiver lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    Ok(())
}

#[cfg(not(test))]
async fn drive_device_link_exchange_tick(
    config_dir: &Path,
    sync: &iris_drive_core::FsFipsBlockSync,
    sent_requests: &mut BTreeMap<String, std::time::Instant>,
    sent_rosters: &mut BTreeMap<String, std::time::Instant>,
    acked_rosters: &BTreeSet<String>,
) -> Result<bool, String> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    let Some(state) = config.account.as_ref() else {
        return Ok(false);
    };

    sync.refresh_authorized_peers(&config).await;
    send_native_pending_device_link_request(sync, state, sent_requests).await?;
    send_native_authorized_device_link_rosters(
        config_dir,
        sync,
        state,
        sent_rosters,
        acked_rosters,
    )
    .await?;
    if let Err(error) = write_native_fips_status(config_dir, sync, None).await {
        tracing::warn!(error = %error, "writing native FIPS status failed");
    }
    Ok(true)
}

#[cfg(not(test))]
async fn send_native_pending_device_link_request(
    sync: &iris_drive_core::FsFipsBlockSync,
    state: &iris_drive_core::AccountState,
    sent_requests: &mut BTreeMap<String, std::time::Instant>,
) -> Result<(), String> {
    let Some(frame) = pending_device_link_request_frame(state) else {
        return Ok(());
    };
    let Some(pending) = state.outbound_device_link_request.as_ref() else {
        return Ok(());
    };
    let fingerprint = format!(
        "{}:{}:{}",
        pending.admin_device_pubkey, state.device_pubkey, pending.requested_at
    );
    let now = std::time::Instant::now();
    if sent_requests.get(&fingerprint).is_some_and(|last_sent| {
        now.duration_since(*last_sent)
            < std::time::Duration::from_secs(DEVICE_LINK_REQUEST_RETRY_SECS)
    }) {
        return Ok(());
    }
    let admin_npub = account_npub(&pending.admin_device_pubkey);
    let bytes = serde_json::to_vec(&frame)
        .map_err(|error| format!("encoding device link request: {error}"))?;
    match sync
        .send_app_message(&admin_npub, DEVICE_LINK_REQUEST_APP_TOPIC, bytes)
        .await
    {
        Ok(()) => {
            sent_requests.insert(fingerprint, now);
            tracing::debug!(
                admin_npub,
                requested_at = frame.requested_at,
                "sent native device-link request over FIPS"
            );
        }
        Err(error) => tracing::warn!(
            admin_npub,
            error = %error,
            "sending native device-link request over FIPS failed"
        ),
    }
    Ok(())
}

#[cfg(not(test))]
async fn send_native_authorized_device_link_rosters(
    config_dir: &Path,
    sync: &iris_drive_core::FsFipsBlockSync,
    state: &iris_drive_core::AccountState,
    sent_rosters: &mut BTreeMap<String, std::time::Instant>,
    acked_rosters: &BTreeSet<String>,
) -> Result<(), String> {
    if !state.can_manage_devices() {
        return Ok(());
    }
    let Some(app_keys) = state.app_keys.as_ref() else {
        return Ok(());
    };
    if !app_keys.contains(&state.device_pubkey) {
        return Ok(());
    }

    let now = std::time::Instant::now();
    let due_devices = app_keys
        .devices
        .iter()
        .filter(|device| device.pubkey != state.device_pubkey)
        .filter(|device| {
            let fingerprint = device_link_roster_fingerprint(device.pubkey.as_str(), app_keys);
            if acked_rosters.contains(&fingerprint) {
                return false;
            }
            !sent_rosters.get(&fingerprint).is_some_and(|last_sent| {
                now.duration_since(*last_sent)
                    < std::time::Duration::from_secs(DEVICE_LINK_ROSTER_RETRY_SECS)
            })
        })
        .map(|device| device.pubkey.clone())
        .collect::<Vec<_>>();
    if due_devices.is_empty() {
        return Ok(());
    }

    let (event_id, event_json) = signed_roster_event_for_native_state(config_dir, state, app_keys)?;
    let frame = DeviceLinkRosterFrame {
        schema: 1,
        owner_pubkey: state.owner_pubkey.clone(),
        admin_device_pubkey: state.device_pubkey.clone(),
        app_keys: app_keys.clone(),
        app_keys_event_id: event_id,
        app_keys_event_json: event_json,
        sent_at: unix_now_seconds(),
    };
    let bytes = serde_json::to_vec(&frame)
        .map_err(|error| format!("encoding device link roster: {error}"))?;
    for device_pubkey in due_devices {
        let recipient_npub = account_npub(&device_pubkey);
        match sync
            .send_app_message(&recipient_npub, DEVICE_LINK_ROSTER_APP_TOPIC, bytes.clone())
            .await
        {
            Ok(()) => {
                sent_rosters.insert(
                    device_link_roster_fingerprint(&device_pubkey, app_keys),
                    now,
                );
                tracing::debug!(
                    recipient_npub,
                    dck_generation = app_keys.dck_generation,
                    "sent native device-link roster over FIPS"
                );
            }
            Err(error) => tracing::warn!(
                recipient_npub,
                error = %error,
                "sending native device-link roster over FIPS failed"
            ),
        }
    }
    Ok(())
}

#[cfg(not(test))]
fn signed_roster_event_for_native_state(
    config_dir: &Path,
    state: &iris_drive_core::AccountState,
    app_keys: &iris_drive_core::AppKeysSnapshot,
) -> Result<(String, String), String> {
    if let Some(record) = state.app_keys_event.as_ref()
        && record.signer_pubkey == app_keys.signer_pubkey()
    {
        return Ok((record.event_id.clone(), record.event_json.clone()));
    }
    if app_keys.signer_pubkey() != state.device_pubkey {
        return Err("cannot send roster: missing signed event from this admin device".to_owned());
    }
    let account = Account::load(state.clone(), config_dir)
        .map_err(|error| format!("loading account for roster signing: {error}"))?;
    let event =
        iris_drive_core::nostr_events::build_app_keys_event(account.device.keys(), app_keys)
            .map_err(|error| format!("building device-link roster event: {error}"))?;
    Ok((event.id.to_hex(), event.as_json()))
}

#[cfg(not(test))]
async fn handle_native_device_link_app_message(
    config_dir: &Path,
    sync: &iris_drive_core::FsFipsBlockSync,
    message: &iris_drive_core::FipsAppMessage,
    acked_rosters: &mut BTreeSet<String>,
) -> Result<bool, String> {
    match message.topic.as_str() {
        DEVICE_LINK_REQUEST_APP_TOPIC => handle_native_device_link_request(config_dir, message),
        DEVICE_LINK_ROSTER_APP_TOPIC => {
            handle_native_device_link_roster(config_dir, sync, message).await
        }
        DEVICE_LINK_ROSTER_ACK_APP_TOPIC => {
            handle_native_device_link_roster_ack(config_dir, message, acked_rosters)
        }
        _ => Ok(false),
    }
}

#[cfg(not(test))]
fn handle_native_device_link_request(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
) -> Result<bool, String> {
    let frame: DeviceLinkRequestFrame = serde_json::from_slice(&message.data)
        .map_err(|error| format!("parsing device link request frame: {error}"))?;
    if frame.schema != 1 {
        return Err(format!(
            "unsupported device link request schema {}",
            frame.schema
        ));
    }
    let owner_hex = normalize_pubkey(&frame.owner_pubkey)?;
    let device_hex = normalize_pubkey(&frame.device_pubkey)?;
    let link_secret = if frame.link_secret.trim().is_empty() {
        device_approval_link_secret(&frame.url)
    } else {
        frame.link_secret
    };

    let mut config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    let Some(state) = config.account.as_mut() else {
        return Ok(true);
    };
    let changed = state
        .record_inbound_device_link_request(
            &owner_hex,
            &device_hex,
            frame.label,
            &link_secret,
            frame.requested_at,
        )
        .map_err(|error| format!("recording inbound device link request: {error}"))?;
    if changed {
        config
            .save(config_path_in(config_dir))
            .map_err(|error| format!("saving config: {error}"))?;
        tracing::debug!(
            peer = message.peer_id,
            device_npub = account_npub(&device_hex),
            requested_at = frame.requested_at,
            "received native device-link request over FIPS"
        );
    }
    Ok(true)
}

#[cfg(not(test))]
async fn handle_native_device_link_roster(
    config_dir: &Path,
    sync: &iris_drive_core::FsFipsBlockSync,
    message: &iris_drive_core::FipsAppMessage,
) -> Result<bool, String> {
    let frame: DeviceLinkRosterFrame = serde_json::from_slice(&message.data)
        .map_err(|error| format!("parsing device link roster frame: {error}"))?;
    if frame.schema != 1 {
        return Err(format!(
            "unsupported device link roster schema {}",
            frame.schema
        ));
    }
    let owner_hex = normalize_pubkey(&frame.owner_pubkey)?;
    let admin_device_hex = normalize_pubkey(&frame.admin_device_pubkey)?;
    let sender_hex = normalize_pubkey(&message.peer_id).ok();

    let mut config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    let Some(state) = config.account.as_ref() else {
        return Ok(true);
    };
    if state.can_manage_devices() || state.owner_pubkey != owner_hex {
        return Ok(true);
    }
    if sender_hex.as_deref() != Some(admin_device_hex.as_str()) {
        return Ok(true);
    }
    if frame.app_keys_event_json.is_empty() || frame.app_keys_event_id.is_empty() {
        return Ok(true);
    }
    let roster_event = nostr_sdk::Event::from_json(&frame.app_keys_event_json)
        .map_err(|error| format!("parsing signed device-link roster event: {error}"))?;
    if roster_event.id.to_hex() != frame.app_keys_event_id {
        return Ok(true);
    }
    let parsed = iris_drive_core::nostr_events::parse_app_keys_event(&roster_event)
        .map_err(|error| format!("parsing signed device-link AppKeys: {error}"))?;
    if roster_event.pubkey.to_hex() != admin_device_hex
        || parsed.owner_pubkey != state.owner_pubkey
        || !parsed.is_admin(&admin_device_hex)
    {
        return Ok(true);
    }

    let outcome = iris_drive_core::relay_sync::apply_device_link_roster_event(
        &mut config,
        &roster_event,
        &admin_device_hex,
    )
    .map_err(|error| format!("applying signed device-link roster event: {error}"))?;
    let accepted = match outcome {
        iris_drive_core::relay_sync::DeviceLinkRosterApply::Current => true,
        iris_drive_core::relay_sync::DeviceLinkRosterApply::Applied(decision) => {
            decision != iris_drive_core::ApplyDecision::Rejected
        }
        iris_drive_core::relay_sync::DeviceLinkRosterApply::Ignored => false,
    };
    let changed = matches!(
        outcome,
        iris_drive_core::relay_sync::DeviceLinkRosterApply::Applied(decision)
            if decision != iris_drive_core::ApplyDecision::Rejected
    );
    let state = config.account.as_ref().expect("account still present");
    let ack_data = if accepted {
        Some((
            state.device_pubkey.clone(),
            state
                .app_keys
                .as_ref()
                .expect("accepted/current app keys")
                .clone(),
            roster_event.id.to_hex(),
        ))
    } else {
        None
    };
    if changed {
        config
            .save(config_path_in(config_dir))
            .map_err(|error| format!("saving config: {error}"))?;
        tracing::debug!(
            peer = message.peer_id,
            admin_device_npub = account_npub(&admin_device_hex),
            apply_outcome = ?outcome,
            "accepted native device-link roster over FIPS"
        );
    }
    if let Some((device_pubkey, app_keys, app_keys_event_id)) = ack_data {
        send_native_device_link_roster_ack(
            sync,
            &admin_device_hex,
            &owner_hex,
            &device_pubkey,
            &app_keys_event_id,
            &app_keys,
        )
        .await?;
    }
    let should_sync_roots = changed
        && config
            .account
            .as_ref()
            .is_some_and(iris_drive_core::AccountState::is_authorized);
    if should_sync_roots {
        match iris_drive_core::sync_once_with_fips(
            config_dir,
            &[],
            std::time::Duration::from_secs(NATIVE_SYNC_RELAY_TIMEOUT_SECS),
            Some(sync),
        )
        .await
        {
            Ok(report) => tracing::debug!(
                drive_root_events_applied = report.drive_root_events_applied,
                fips_download = report.fips_download.is_some(),
                blossom_download = report.blossom_download.is_some(),
                "synced drive roots after native device-link roster"
            ),
            Err(error) => tracing::warn!(
                error = %error,
                "syncing drive roots after native device-link roster failed"
            ),
        }
    }
    Ok(true)
}

#[cfg(not(test))]
fn handle_native_device_link_roster_ack(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
    acked_rosters: &mut BTreeSet<String>,
) -> Result<bool, String> {
    let frame: DeviceLinkRosterAckFrame = serde_json::from_slice(&message.data)
        .map_err(|error| format!("parsing device link roster ack frame: {error}"))?;
    if frame.schema != 1 {
        return Err(format!(
            "unsupported device link roster ack schema {}",
            frame.schema
        ));
    }
    let owner_hex = normalize_pubkey(&frame.owner_pubkey)?;
    let admin_device_hex = normalize_pubkey(&frame.admin_device_pubkey)?;
    let device_hex = normalize_pubkey(&frame.device_pubkey)?;
    if normalize_pubkey(&message.peer_id).ok().as_deref() != Some(device_hex.as_str()) {
        return Ok(true);
    }

    let config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    let Some(state) = config.account.as_ref() else {
        return Ok(true);
    };
    let Some(app_keys) = state.app_keys.as_ref() else {
        return Ok(true);
    };
    if !state.can_manage_devices()
        || state.owner_pubkey != owner_hex
        || state.device_pubkey != admin_device_hex
        || !app_keys.contains(&device_hex)
        || app_keys.created_at != frame.app_keys_created_at
        || app_keys.dck_generation != frame.dck_generation
    {
        return Ok(true);
    }

    acked_rosters.insert(device_link_roster_fingerprint(&device_hex, app_keys));
    Ok(true)
}

#[cfg(not(test))]
async fn send_native_device_link_roster_ack(
    sync: &iris_drive_core::FsFipsBlockSync,
    admin_device_hex: &str,
    owner_hex: &str,
    device_hex: &str,
    app_keys_event_id: &str,
    app_keys: &iris_drive_core::AppKeysSnapshot,
) -> Result<(), String> {
    let frame = DeviceLinkRosterAckFrame {
        schema: 1,
        owner_pubkey: owner_hex.to_owned(),
        admin_device_pubkey: admin_device_hex.to_owned(),
        device_pubkey: device_hex.to_owned(),
        app_keys_event_id: app_keys_event_id.to_owned(),
        app_keys_created_at: app_keys.created_at,
        dck_generation: app_keys.dck_generation,
        acknowledged_at: unix_now_seconds(),
    };
    sync.send_app_message(
        &account_npub(admin_device_hex),
        DEVICE_LINK_ROSTER_ACK_APP_TOPIC,
        serde_json::to_vec(&frame)
            .map_err(|error| format!("encoding device-link roster ack: {error}"))?,
    )
    .await
    .map_err(|error| format!("sending device-link roster ack over FIPS: {error}"))?;
    Ok(())
}

#[cfg(not(test))]
fn device_link_roster_fingerprint(
    device_pubkey: &str,
    app_keys: &iris_drive_core::AppKeysSnapshot,
) -> String {
    format!(
        "{}:{}:{}",
        device_pubkey, app_keys.created_at, app_keys.dck_generation
    )
}

#[derive(Debug, Deserialize)]
struct NativeFipsStatus {
    #[serde(default)]
    running: bool,
    #[serde(default)]
    endpoint_npub: Option<String>,
    #[serde(default)]
    updated_at: u64,
    #[serde(default)]
    connected_peers: Vec<String>,
    #[serde(default)]
    mesh_peers: Vec<String>,
    #[serde(default)]
    error: Option<String>,
}

impl NativeFipsStatus {
    fn is_fresh(&self) -> bool {
        self.running
            && self.error.as_deref().unwrap_or_default().is_empty()
            && unix_now_seconds().saturating_sub(self.updated_at) <= NATIVE_FIPS_STATUS_FRESH_SECS
    }
}

fn native_fips_status_path(config_dir: &Path) -> PathBuf {
    config_dir.join(NATIVE_FIPS_STATUS_FILE_NAME)
}

fn load_native_fips_status(config_dir: &Path) -> Option<NativeFipsStatus> {
    let data = std::fs::read(native_fips_status_path(config_dir)).ok()?;
    serde_json::from_slice(&data).ok()
}

#[cfg(not(test))]
async fn write_native_fips_status(
    config_dir: &Path,
    sync: &iris_drive_core::FsFipsBlockSync,
    error: Option<&str>,
) -> Result<(), String> {
    let value = json!({
        "running": error.is_none(),
        "updated_at": unix_now_seconds(),
        "endpoint_npub": sync.endpoint_npub(),
        "discovery_scope": sync.discovery_scope(),
        "authorized_peers": sync.authorized_peer_ids().await,
        "connected_peers": sync.connected_peer_ids().await,
        "mesh_peers": sync.mesh_peer_ids().await,
        "peer_statuses": sync.fips_peer_statuses().await,
        "error": error,
    });
    write_native_fips_status_value(config_dir, &value)
}

#[cfg(not(test))]
fn write_native_fips_error(config_dir: &Path, error: &str) {
    let value = json!({
        "running": false,
        "updated_at": unix_now_seconds(),
        "connected_peers": [],
        "mesh_peers": [],
        "peer_statuses": [],
        "error": error,
    });
    if let Err(write_error) = write_native_fips_status_value(config_dir, &value) {
        tracing::warn!(error = %write_error, "writing native FIPS error failed");
    }
}

#[cfg(not(test))]
fn write_native_fips_status_value(
    config_dir: &Path,
    value: &serde_json::Value,
) -> Result<(), String> {
    let path = native_fips_status_path(config_dir);
    let data =
        serde_json::to_vec(value).map_err(|error| format!("encoding FIPS status: {error}"))?;
    std::fs::write(&path, data).map_err(|error| format!("writing {}: {error}", path.display()))
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
            label: "Blossom remote".to_owned(),
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
    fips_status: Option<&NativeFipsStatus>,
) -> Vec<UiDevice> {
    let Some(app_keys) = state.app_keys.as_ref() else {
        return Vec::new();
    };
    let fips_is_fresh = fips_status.is_some_and(NativeFipsStatus::is_fresh);

    app_keys
        .devices
        .iter()
        .map(|device| {
            let is_current = device.pubkey == state.device_pubkey;
            let device_npub = account_npub(&device.pubkey);
            let is_online = fips_is_fresh
                && fips_status.is_some_and(|status| {
                    if is_current {
                        return status
                            .endpoint_npub
                            .as_deref()
                            .is_none_or(|endpoint| endpoint == device_npub);
                    }
                    status
                        .connected_peers
                        .iter()
                        .any(|peer| peer == &device_npub)
                        || status.mesh_peers.iter().any(|peer| peer == &device_npub)
                });
            let role = device_role_label(device.role).to_owned();
            UiDevice {
                pubkey: device_npub.clone(),
                label: device.label.clone().unwrap_or_default(),
                state: "Linked".to_owned(),
                role,
                detail: device_npub,
                is_current_device: is_current,
                is_online,
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
        || request.starts_with("iris-drive:/device-link")
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

#[cfg(not(test))]
fn device_approval_link_secret(request: &str) -> String {
    if request.starts_with("iris-drive://device-link")
        || request.starts_with("iris-drive:/device-link")
        || request.starts_with("https://drive.iris.to/device-link")
    {
        let query = request.split_once('?').map_or("", |(_, query)| query);
        return query_value(query, "secret")
            .or_else(|| query_value(query, "link_secret"))
            .unwrap_or_default()
            .trim()
            .to_owned();
    }
    String::new()
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

fn update_snapshot_link(state: &mut NativeAppState, config: &AppConfig) {
    state.ui.snapshot_link = current_primary_root_cid(config)
        .and_then(|root| drive_iris_to_nhash_url_for_root(&root))
        .unwrap_or_default();
}

fn current_primary_root_cid(config: &AppConfig) -> Option<String> {
    config
        .account
        .as_ref()
        .and_then(|account| {
            config
                .drive(iris_drive_core::PRIMARY_DRIVE_ID)
                .and_then(|drive| drive.device_roots.get(&account.device_pubkey))
                .map(|root| root.root_cid.clone())
        })
        .or_else(|| {
            config
                .drive(iris_drive_core::PRIMARY_DRIVE_ID)
                .and_then(|drive| drive.last_root_cid.clone())
        })
}

fn drive_iris_to_nhash_url_for_root(root_cid: &str) -> Option<String> {
    let cid = Cid::parse(root_cid).ok()?;
    let nhash = nhash_encode_full(&NHashData {
        hash: cid.hash,
        decrypt_key: cid.key,
    })
    .ok()?;
    Some(format!("https://drive.iris.to/#/{nhash}"))
}

#[cfg(test)]
mod tests;
