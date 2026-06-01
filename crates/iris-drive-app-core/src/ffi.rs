#[cfg(not(test))]
use std::collections::BTreeMap;
#[cfg(not(test))]
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

#[cfg(not(test))]
use std::sync::atomic::{AtomicBool, Ordering};

use hashtree_core::{Cid, NHashData, nhash_encode_full};
use iris_drive_core::config::{DEFAULT_BLOSSOM_SERVERS, DEFAULT_RELAYS};
#[cfg(not(test))]
use iris_drive_core::device_link_transport::{
    DEVICE_LINK_REQUEST_APP_TOPIC, DEVICE_LINK_ROSTER_ACK_APP_TOPIC, DEVICE_LINK_ROSTER_APP_TOPIC,
    DeviceLinkRequestFrame, DeviceLinkRosterAckFrame, DeviceLinkRosterFrame,
    pending_device_link_request_frame,
};
use iris_drive_core::device_summary::{
    authorization_state_key, device_connection_label, device_connection_state,
    device_display_label, device_management_actions, device_role_key, device_role_label,
    primary_status_for_setup_state, primary_status_label, setup_label_for_setup_state,
    sync_status_label,
};
#[cfg(not(test))]
use iris_drive_core::fips_status::online_device_ids;
use iris_drive_core::fips_status::{
    fips_error_is_present, normalize_fips_status_value, string_vec_from_json_array,
};
use iris_drive_core::paths::{config_path_in, key_path_in};
use iris_drive_core::relay_config::{dedupe_relay_urls, normalize_relay_url};
use iris_drive_core::{Account, AppConfig, DeviceAuthorizationState, Drive};
#[cfg(not(test))]
use nostr_sdk::JsonUtil;
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::{FromBech32, ToBech32};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(not(test))]
use serde_json::json;

use crate::actions::NativeAppAction;
#[cfg(test)]
pub(crate) use crate::native_provider::run_native_sync_once_with_drive_root_events_for_test;
use crate::native_provider::{
    install_rustls_crypto_provider, native_provider_import_shared_file, native_sync_status_label,
    run_native_provider_list, run_native_sync_once,
};
pub(crate) use crate::native_provider::{
    native_provider_delete_json, native_provider_import_shared_file_json,
    native_provider_list_json, native_provider_mkdir_json, native_provider_normalize_path_json,
    native_provider_read_json, native_provider_rename_json, native_provider_resolve_path_json,
    native_provider_write_json,
};
use crate::state::{
    NativeAppState, UiAccount, UiBackup, UiDevice, UiDeviceLinkRequest, UiFipsStatus, UiPaths,
    UiRelayStatus, UiState, UiSyncRoot, UiSyncStatus,
};
use iris_drive_core::relay_status::{relay_status_health, relay_status_label};

#[cfg(target_os = "android")]
#[path = "ffi_android_test_support.rs"]
mod android_test_support;
#[cfg(target_os = "android")]
pub(crate) use android_test_support::native_apply_owner_snapshot_for_test_json;

const DEFAULT_ROOT_STATUS: &str = "SAF provider root";
const NATIVE_FIPS_STATUS_FILE_NAME: &str = "native-fips-status.json";
const NATIVE_FIPS_STATUS_FRESH_SECS: u64 = 20;
#[cfg(not(test))]
const NATIVE_SYNC_RELAY_TIMEOUT_SECS: u64 = 10;
#[cfg(not(test))]
const DEVICE_LINK_REQUEST_RETRY_SECS: u64 = 10;
#[cfg(not(test))]
const DEVICE_LINK_ROSTER_RETRY_SECS: u64 = 2;
#[cfg(not(test))]
const DEVICE_LINK_EXCHANGE_TICK_SECS: u64 = 1;

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinkInputClassification {
    pub kind: String,
    pub is_complete: bool,
    pub is_valid: bool,
    pub normalized_input: String,
    pub owner_pubkey: String,
    pub device_pubkey: String,
    pub admin_device_pubkey: String,
    pub has_link_secret: bool,
    pub error: String,
}

#[uniffi::export]
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn classify_link_input(input: String) -> LinkInputClassification {
    classify_link_input_value(&input)
}

#[uniffi::export]
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn validate_link_input(input: String) -> LinkInputClassification {
    classify_link_input_value(&input)
}

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
    #[cfg(not(test))]
    device_link_exchange_stop: Arc<AtomicBool>,
}

impl NativeAppRuntime {
    fn new(data_dir: String, app_version: String) -> Self {
        let mut state = NativeAppState::default();
        state.ui.paths = paths_for(&data_dir);
        state.ui.sync = UiSyncStatus {
            running: true,
            status: "running".to_owned(),
            status_label: sync_status_label("running"),
        };

        let mut runtime = Self {
            state,
            data_dir,
            app_version,
            #[cfg(not(test))]
            device_link_exchange_running: Arc::new(AtomicBool::new(false)),
            #[cfg(not(test))]
            device_link_exchange_stop: Arc::new(AtomicBool::new(false)),
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
                self.stop_device_link_exchange();
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
        let url = match normalize_relay_url(url) {
            Ok(url) => url,
            Err(error) => {
                self.state.error = error.to_string();
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
        let mut relays = match normalized_config_relays(&config.relays) {
            Ok(relays) => relays,
            Err(error) => {
                self.state.error = format!("normalizing relays: {error}");
                return;
            }
        };
        if !relays.iter().any(|existing| existing == &url) {
            relays.push(url);
        }
        config.relays = relays;
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
        }
    }

    fn remove_relay(&mut self, url: &str) {
        let url = match normalize_relay_url(url) {
            Ok(url) => url,
            Err(error) => {
                self.state.error = error.to_string();
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
        let mut relays = match normalized_config_relays(&config.relays) {
            Ok(relays) => relays,
            Err(error) => {
                self.state.error = format!("normalizing relays: {error}");
                return;
            }
        };
        let before = relays.len();
        relays.retain(|relay| relay != &url);
        if before == relays.len() {
            self.state.error = format!("relay not found: {url}");
            return;
        }
        config.relays = relays;
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

    #[allow(clippy::unused_self)]
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

            self.device_link_exchange_stop
                .store(false, Ordering::Release);
            let data_dir = self.data_dir.clone();
            let running = self.device_link_exchange_running.clone();
            let stop = self.device_link_exchange_stop.clone();
            std::thread::spawn(move || {
                if let Err(error) = run_device_link_exchange(&data_dir, stop) {
                    tracing::warn!(error = %error, "native device-link FIPS exchange stopped");
                }
                running.store(false, Ordering::Release);
            });
        }
    }

    #[allow(clippy::unused_self)]
    fn stop_device_link_exchange(&mut self) {
        #[cfg(not(test))]
        {
            self.device_link_exchange_stop
                .store(true, Ordering::Release);
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
            relay_statuses: default_relay_statuses(&default_relays()),
            backups: default_backups(),
            paths,
            sync,
            setup_state: "not_configured".to_owned(),
            setup_label: setup_label_for_setup_state("not_configured").to_owned(),
            primary_status: "not_setup".to_owned(),
            primary_status_label: primary_status_label("not_setup").to_owned(),
            snapshot_link: String::new(),
            ..UiState::default()
        };

        let Ok(config) = self.load_config() else {
            self.refresh_ui_summary(None);
            return;
        };
        self.state.ui.relays = if config.relays.is_empty() {
            default_relays()
        } else {
            config.relays.clone()
        };
        self.state.ui.relay_statuses = default_relay_statuses(&self.state.ui.relays);
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
            self.refresh_ui_summary(None);
            return;
        };
        let mut account = raw_account.clone();
        account.recompute_authorization();
        self.state.ui.account = Some(UiAccount {
            owner_pubkey: account_npub(&account.owner_pubkey),
            device_pubkey: account_npub(&account.device_pubkey),
            device_label: account.device_label.clone().unwrap_or_default(),
            authorization_state: authorization_state_key(account.authorization_state).to_owned(),
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
            self.refresh_ui_summary(None);
            return;
        }
        let fips_status = load_native_fips_status(Path::new(&self.data_dir));
        let ui_fips_status = ui_fips_status(fips_status.as_ref());
        self.state.ui.devices = devices_from_account(&account, &ui_fips_status);
        self.refresh_device_actions();
        update_snapshot_link(&mut self.state, &config);
        self.refresh_provider_summary();
        self.refresh_ui_summary(Some(ui_fips_status));
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
            let actions = device_management_actions(can_manage, is_current, is_admin, admin_count);
            device.can_revoke = actions.can_revoke;
            device.can_appoint_admin = actions.can_appoint_admin;
            device.can_demote_admin = actions.can_demote_admin;
            device.is_current_device = is_current;
        }
    }

    fn refresh_provider_summary(&mut self) {
        let Ok(value) = run_native_provider_list(&self.data_dir) else {
            return;
        };
        self.state.ui.file_count = value
            .get("file_count")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        self.state.ui.visible_file_bytes = value
            .get("visible_file_bytes")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
    }

    fn refresh_ui_summary(&mut self, fips_status: Option<UiFipsStatus>) {
        let setup_state = self.state.ui.account.as_ref().map_or_else(
            || "not_configured".to_owned(),
            |account| account.authorization_state.clone(),
        );
        primary_status_for_setup_state(&setup_state).clone_into(&mut self.state.ui.primary_status);
        self.state.ui.setup_state = setup_state;
        setup_label_for_setup_state(&self.state.ui.setup_state)
            .clone_into(&mut self.state.ui.setup_label);
        primary_status_label(&self.state.ui.primary_status)
            .clone_into(&mut self.state.ui.primary_status_label);
        self.state.ui.authorized_device_count = self.state.ui.devices.len() as u64;
        self.state.ui.online_device_count = self
            .state
            .ui
            .devices
            .iter()
            .filter(|device| device.is_online)
            .count() as u64;
        self.state.ui.fips = fips_status.unwrap_or_else(paused_ui_fips_status);
    }

    fn set_sync_running(&mut self, running: bool) {
        self.set_sync_status(running, if running { "running" } else { "paused" });
    }

    fn set_sync_status(&mut self, running: bool, status: &str) {
        self.state.ui.sync = UiSyncStatus {
            running,
            status: status.to_owned(),
            status_label: sync_status_label(status),
        };
    }

    fn refresh_sync_status_label(&mut self) {
        self.state.ui.sync.status_label = sync_status_label(&self.state.ui.sync.status);
    }

    fn start_sync(&mut self) {
        self.set_sync_running(true);
        match run_native_sync_once(&self.data_dir) {
            Ok(report) => {
                native_sync_status_label(&report).clone_into(&mut self.state.ui.sync.status);
                self.refresh_sync_status_label();
            }
            Err(error) => {
                "sync error".clone_into(&mut self.state.ui.sync.status);
                self.refresh_sync_status_label();
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

#[cfg(not(test))]
impl Drop for NativeAppRuntime {
    fn drop(&mut self) {
        self.device_link_exchange_stop
            .store(true, Ordering::Release);
    }
}

#[cfg(not(test))]
fn run_device_link_exchange(data_dir: &str, stop: Arc<AtomicBool>) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| format!("building device-link exchange runtime: {error}"))?;
    let result = runtime.block_on(run_device_link_exchange_async(data_dir, stop));
    if let Err(error) = &result {
        write_native_fips_error(Path::new(data_dir), error);
    }
    result
}

#[cfg(not(test))]
async fn run_device_link_exchange_async(
    data_dir: &str,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
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

    while !stop.load(Ordering::Acquire) {
        tokio::select! {
            _ = tick.tick() => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
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
#[allow(clippy::too_many_lines)]
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

fn ui_fips_status(status: Option<&Value>) -> UiFipsStatus {
    let Some(status) = status else {
        return paused_ui_fips_status();
    };
    let running = status
        .get("running")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fresh = native_fips_status_is_fresh(status);
    let error = status.get("error").cloned().unwrap_or(Value::Null);
    let normalized = normalize_fips_status_value(Some(status), running, fresh, error, &[]);
    let online_devices = string_vec_from_json_array(normalized.get("online_devices"));
    let direct_devices = string_vec_from_json_array(normalized.get("direct_devices"));
    let mesh_devices = string_vec_from_json_array(normalized.get("mesh_devices"));
    UiFipsStatus {
        enabled: normalized
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        running: normalized
            .get("running")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        fresh: normalized
            .get("fresh")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        state: normalized
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("paused")
            .to_owned(),
        state_label: normalized
            .get("state_label")
            .and_then(Value::as_str)
            .unwrap_or("Paused")
            .to_owned(),
        endpoint_npub: normalized
            .get("endpoint_npub")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        online_device_count: online_devices.len() as u64,
        direct_device_count: direct_devices.len() as u64,
        mesh_device_count: mesh_devices.len() as u64,
        online_devices,
        direct_devices,
        mesh_devices,
        error: normalized
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
    }
}

fn paused_ui_fips_status() -> UiFipsStatus {
    UiFipsStatus {
        state: "paused".to_owned(),
        state_label: "Paused".to_owned(),
        ..UiFipsStatus::default()
    }
}

fn native_fips_status_is_fresh(status: &Value) -> bool {
    let running = status
        .get("running")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let error = status.get("error").unwrap_or(&Value::Null);
    let updated_at = status
        .get("updated_at")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    running
        && !fips_error_is_present(error)
        && unix_now_seconds().saturating_sub(updated_at) <= NATIVE_FIPS_STATUS_FRESH_SECS
}

fn native_fips_status_path(config_dir: &Path) -> PathBuf {
    config_dir.join(NATIVE_FIPS_STATUS_FILE_NAME)
}

fn load_native_fips_status(config_dir: &Path) -> Option<Value> {
    let data = std::fs::read(native_fips_status_path(config_dir)).ok()?;
    serde_json::from_slice(&data).ok()
}

#[cfg(not(test))]
async fn write_native_fips_status(
    config_dir: &Path,
    sync: &iris_drive_core::FsFipsBlockSync,
    error: Option<&str>,
) -> Result<(), String> {
    let direct_devices = sync.connected_peer_ids().await;
    let mesh_devices = sync.mesh_peer_ids().await;
    let online_devices = online_device_ids(&direct_devices, &mesh_devices);
    let updated_at = unix_now_seconds();
    let error_value = error.map_or(Value::Null, |error| Value::String(error.to_owned()));
    let raw = json!({
        "running": error.is_none(),
        "updated_at": updated_at,
        "endpoint_npub": sync.endpoint_npub(),
        "discovery_scope": sync.discovery_scope(),
        "authorized_peers": sync.authorized_peer_ids().await,
        "online_devices": online_devices.clone(),
        "online_peers": online_devices,
        "direct_devices": direct_devices.clone(),
        "direct_peers": direct_devices.clone(),
        "connected_peers": direct_devices,
        "mesh_devices": mesh_devices.clone(),
        "mesh_peers": mesh_devices,
        "peer_statuses": sync.fips_peer_statuses().await,
        "error": error,
    });
    let value = normalize_fips_status_value(
        Some(&raw),
        error.is_none(),
        error.is_none(),
        error_value,
        &[],
    );
    write_native_fips_status_value(config_dir, &value)
}

#[cfg(not(test))]
fn write_native_fips_error(config_dir: &Path, error: &str) {
    let raw = json!({
        "running": false,
        "updated_at": unix_now_seconds(),
        "online_devices": [],
        "online_peers": [],
        "direct_devices": [],
        "direct_peers": [],
        "connected_peers": [],
        "mesh_devices": [],
        "mesh_peers": [],
        "peer_statuses": [],
        "error": error,
    });
    let value = normalize_fips_status_value(
        Some(&raw),
        false,
        false,
        Value::String(error.to_owned()),
        &[],
    );
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

fn normalized_config_relays(
    relays: &[String],
) -> Result<Vec<String>, iris_drive_core::relay_config::RelayConfigError> {
    let mut relays = relays.to_vec();
    dedupe_relay_urls(&mut relays)?;
    Ok(relays)
}

fn default_relay_statuses(relays: &[String]) -> Vec<UiRelayStatus> {
    relays
        .iter()
        .map(|relay| UiRelayStatus {
            url: relay.clone(),
            status: "configured".to_owned(),
            status_label: relay_status_label("configured"),
            health: relay_status_health("configured").to_owned(),
        })
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

fn devices_from_account(
    state: &iris_drive_core::AccountState,
    fips_status: &UiFipsStatus,
) -> Vec<UiDevice> {
    let Some(app_keys) = state.app_keys.as_ref() else {
        return Vec::new();
    };
    let fips_is_fresh = fips_status.fresh;

    app_keys
        .devices
        .iter()
        .map(|device| {
            let is_current = device.pubkey == state.device_pubkey;
            let device_npub = account_npub(&device.pubkey);
            let is_online = fips_is_fresh
                && if is_current {
                    fips_status.endpoint_npub.is_empty() || fips_status.endpoint_npub == device_npub
                } else {
                    fips_status
                        .online_devices
                        .iter()
                        .any(|peer| peer == &device_npub)
                };
            let is_direct = fips_is_fresh
                && !is_current
                && fips_status
                    .direct_devices
                    .iter()
                    .any(|peer| peer == &device_npub);
            let is_mesh = fips_is_fresh
                && !is_current
                && fips_status
                    .mesh_devices
                    .iter()
                    .any(|peer| peer == &device_npub);
            let connection_state =
                device_connection_state(is_current, is_online, is_direct, is_mesh).to_owned();
            let role = device_role_key(device.role).to_owned();
            let display_label = device_display_label(is_current, device.label.as_deref(), "Device");
            UiDevice {
                pubkey: device_npub.clone(),
                label: device.label.clone().unwrap_or_default(),
                display_label,
                state: "Linked".to_owned(),
                state_label: "Linked".to_owned(),
                connection_label: device_connection_label(&connection_state, None, None),
                connection_state,
                role,
                role_label: device_role_label(device.role).to_owned(),
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

fn classify_link_input_value(input: &str) -> LinkInputClassification {
    let trimmed = input.trim();
    let mut classification = LinkInputClassification {
        kind: "empty".to_owned(),
        normalized_input: trimmed.to_owned(),
        ..LinkInputClassification::default()
    };
    if trimmed.is_empty() {
        return classification;
    }
    if trimmed.contains(char::is_whitespace) {
        "unknown".clone_into(&mut classification.kind);
        "link input must not contain whitespace".clone_into(&mut classification.error);
        return classification;
    }

    if let Some(result) = classify_device_approval_link_input(trimmed) {
        return result;
    }
    if let Some(result) = classify_invite_link_input(trimmed) {
        return result;
    }
    if looks_like_owner_pubkey_input(trimmed) {
        "owner_pubkey".clone_into(&mut classification.kind);
        classification.is_complete = owner_pubkey_input_is_complete(trimmed);
        if classification.is_complete {
            match normalize_pubkey(trimmed) {
                Ok(owner_hex) => {
                    classification.is_valid = true;
                    classification.owner_pubkey = account_npub(&owner_hex);
                    classification
                        .normalized_input
                        .clone_from(&classification.owner_pubkey);
                }
                Err(error) => {
                    classification.error = error;
                }
            }
        }
        return classification;
    }

    "unknown".clone_into(&mut classification.kind);
    "expected owner public key or device invite link".clone_into(&mut classification.error);
    classification
}

fn classify_device_approval_link_input(input: &str) -> Option<LinkInputClassification> {
    let lower = input.to_ascii_lowercase();
    let is_device_approval = link_route_matches(&lower, "iris-drive://device-link", false)
        || link_route_matches(&lower, "iris-drive:/device-link", false)
        || link_route_matches(&lower, "https://drive.iris.to/device-link", false);
    if !is_device_approval {
        return None;
    }

    let query = input.split_once('?').map_or("", |(_, query)| query);
    let owner = query_value(query, "owner");
    let device = query_value(query, "device");
    let is_complete = owner.as_deref().is_some_and(owner_pubkey_input_is_complete)
        && device
            .as_deref()
            .is_some_and(owner_pubkey_input_is_complete);
    let mut classification = LinkInputClassification {
        kind: "device_approval".to_owned(),
        normalized_input: input.to_owned(),
        is_complete,
        ..LinkInputClassification::default()
    };
    if is_complete {
        match decode_device_approval_request(input) {
            Ok((owner_hex, device_hex, _label)) => {
                classification.is_valid = true;
                classification.owner_pubkey = account_npub(&owner_hex);
                classification.device_pubkey = account_npub(&device_hex);
            }
            Err(error) => classification.error = error,
        }
    }
    classification.has_link_secret =
        device_approval_link_secret_value(input).is_some_and(|secret| !secret.trim().is_empty());
    Some(classification)
}

fn classify_invite_link_input(input: &str) -> Option<LinkInputClassification> {
    let lower = input.to_ascii_lowercase();
    let is_canonical = [
        iris_drive_core::device_link_invite::DEVICE_LINK_INVITE_PREFIX,
        "iris-drive:/invite/",
        iris_drive_core::device_link_invite::DEVICE_LINK_INVITE_WEB_PREFIX,
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        || link_route_matches(&lower, "iris-drive://invite", true)
        || link_route_matches(&lower, "iris-drive:/invite", true)
        || link_route_matches(&lower, "https://drive.iris.to/invite", true);
    let is_legacy = link_route_matches(&lower, "iris-drive://link-device", false)
        || link_route_matches(&lower, "iris-drive:/link-device", false)
        || link_route_matches(&lower, "https://drive.iris.to/link-device", false);
    let is_json = input.starts_with('{');
    if !(is_canonical || is_legacy || is_json) {
        return None;
    }

    let mut classification = LinkInputClassification {
        kind: "invite".to_owned(),
        normalized_input: input.to_owned(),
        is_complete: invite_link_input_is_complete(input),
        ..LinkInputClassification::default()
    };
    match iris_drive_core::device_link_invite::parse_device_link_invite(input) {
        Ok(Some(invite)) => {
            classification.is_complete = true;
            classification.is_valid = true;
            classification.owner_pubkey = account_npub(&invite.owner_hex);
            classification.admin_device_pubkey = account_npub(&invite.admin_device_hex);
            classification.has_link_secret = !invite.link_secret.trim().is_empty();
        }
        Ok(None) => {
            "device invite was not recognized".clone_into(&mut classification.error);
        }
        Err(error) if classification.is_complete => {
            classification.error = error.to_string();
        }
        Err(_) => {}
    }
    Some(classification)
}

fn link_route_matches(input: &str, route: &str, allow_path_suffix: bool) -> bool {
    let Some(rest) = input.strip_prefix(route) else {
        return false;
    };
    rest.is_empty() || rest.starts_with('?') || (allow_path_suffix && rest.starts_with('/'))
}

fn invite_link_input_is_complete(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    for prefix in [
        iris_drive_core::device_link_invite::DEVICE_LINK_INVITE_PREFIX,
        "iris-drive:/invite/",
        iris_drive_core::device_link_invite::DEVICE_LINK_INVITE_WEB_PREFIX,
    ] {
        if lower.starts_with(prefix) {
            return input[prefix.len()..].len() >= 32;
        }
    }
    if lower.starts_with("iris-drive://link-device?")
        || lower.starts_with("iris-drive:/link-device?")
        || lower.starts_with("https://drive.iris.to/link-device?")
    {
        return lower.contains("owner=")
            && (lower.contains("admin=") || lower.contains("admin_device="))
            && (lower.contains("secret=") || lower.contains("link_secret="));
    }
    input.starts_with('{') && input.ends_with('}')
}

fn looks_like_owner_pubkey_input(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    lower.starts_with("npub1")
        || (input.len() <= 64 && input.chars().all(|ch| ch.is_ascii_hexdigit()))
}

fn owner_pubkey_input_is_complete(input: &str) -> bool {
    let lower = input.to_ascii_lowercase();
    if lower.starts_with("npub1") {
        return input.len() >= 63;
    }
    input.len() == 64 && input.chars().all(|ch| ch.is_ascii_hexdigit())
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
    let lower = request.to_ascii_lowercase();
    if link_route_matches(&lower, "iris-drive://device-link", false)
        || link_route_matches(&lower, "iris-drive:/device-link", false)
        || link_route_matches(&lower, "https://drive.iris.to/device-link", false)
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
    device_approval_link_secret_value(request)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn device_approval_link_secret_value(request: &str) -> Option<String> {
    let lower = request.to_ascii_lowercase();
    if link_route_matches(&lower, "iris-drive://device-link", false)
        || link_route_matches(&lower, "iris-drive:/device-link", false)
        || link_route_matches(&lower, "https://drive.iris.to/device-link", false)
    {
        let query = request.split_once('?').map_or("", |(_, query)| query);
        return query_value(query, "secret").or_else(|| query_value(query, "link_secret"));
    }
    None
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
mod provider_tests;
#[cfg(test)]
mod tests;
