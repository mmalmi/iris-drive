#[cfg(not(test))]
use std::collections::BTreeMap;
#[cfg(not(test))]
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context;
#[cfg(not(test))]
use std::sync::atomic::{AtomicBool, Ordering};

use hashtree_core::{Cid, NHashData, nhash_encode_full};
use iris_drive_core::backup_ops::{
    add_backup_target as core_add_backup_target, add_blossom_server as core_add_blossom_server,
    check_backups as core_check_backups, default_backup_check_sample_size,
    effective_backup_targets, remove_backup_target as core_remove_backup_target,
    remove_blossom_server as core_remove_blossom_server, sync_backups as core_sync_backups,
};
use iris_drive_core::backup_summary::{backup_target_summary, blossom_backup_target};
use iris_drive_core::config::{DEFAULT_BLOSSOM_SERVERS, DEFAULT_RELAYS};
use iris_drive_core::device_link_transport::{
    AppKeyApprovalRequest, encode_app_key_approval_request, parse_app_key_approval_request,
};
#[cfg(not(test))]
use iris_drive_core::device_link_transport::{
    DEVICE_LINK_REQUEST_APP_TOPIC, DEVICE_LINK_ROSTER_ACK_APP_TOPIC, DEVICE_LINK_ROSTER_APP_TOPIC,
    DeviceLinkRequestFrame, DeviceLinkRosterAckFrame, DeviceLinkRosterFrame,
    device_link_roster_ack_frame, device_link_roster_ack_matches_state, device_link_roster_frame,
    device_link_roster_recipients, pending_app_key_link_request_frame,
};
use iris_drive_core::device_summary::{
    DeviceConnectionDetails, DeviceConnectivity, device_roster_rows, iris_profile_summary,
    primary_status_for_setup_state, primary_status_label, setup_label_for_setup_state,
    setup_state_flags, sync_status_label,
};
#[cfg(not(test))]
use iris_drive_core::fips_status::online_device_ids;
use iris_drive_core::fips_status::{
    fips_error_is_present, normalize_fips_status_value, string_vec_from_json_array,
};
use iris_drive_core::paths::{config_path_in, key_path_in, recovery_phrase_path_in};
use iris_drive_core::relay_config::{dedupe_relay_urls, normalize_relay_url};
use iris_drive_core::relay_status::normalized_relay_statuses_for_relays;
use iris_drive_core::{AppConfig, AppKeyAuthorizationState, BackupTarget, Drive, Profile};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::ToBech32;
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
    native_provider_is_child_document_json, native_provider_list_json, native_provider_mkdir_json,
    native_provider_normalize_path_json, native_provider_read_json, native_provider_rename_json,
    native_provider_resolve_path_json, native_provider_write_json,
};
use crate::state::{
    NativeAppState, UiAppKeyLinkRequest, UiBackup, UiDevice, UiFipsPeerStatus, UiFipsStatus,
    UiPaths, UiProfile, UiRelayStatus, UiShare, UiState, UiSyncRoot, UiSyncStatus,
};

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoverySecretExport {
    pub can_export: bool,
    pub recovery_phrase: String,
    pub words: Vec<String>,
    pub secret_key: String,
    pub error: String,
}

#[uniffi::export]
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn export_recovery_secret(data_dir: String) -> RecoverySecretExport {
    export_recovery_secret_value(&data_dir)
}

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
const DEVICE_LINK_REQUEST_RETRY_SECS: u64 = 10;
const DEVICE_LINK_REQUEST_STARTUP_RETRY_MILLIS: u64 = 250;
const DEVICE_LINK_REQUEST_STARTUP_BURST_ATTEMPTS: u8 = 40;
#[cfg(not(test))]
const DEVICE_LINK_ROSTER_RETRY_SECS: u64 = 2;
#[cfg(not(test))]
const DEVICE_LINK_EXCHANGE_TICK_MILLIS: u64 = 250;

#[derive(Debug, Clone, Copy)]
struct SentDeviceLinkRequest {
    last_sent: std::time::Instant,
    attempts: u8,
}

fn app_key_link_request_send_due(
    sent: Option<SentDeviceLinkRequest>,
    now: std::time::Instant,
) -> bool {
    let Some(sent) = sent else {
        return true;
    };
    now.duration_since(sent.last_sent) >= app_key_link_request_retry_interval(sent.attempts)
}

fn app_key_link_request_retry_interval(attempts: u8) -> std::time::Duration {
    if attempts < DEVICE_LINK_REQUEST_STARTUP_BURST_ATTEMPTS {
        std::time::Duration::from_millis(DEVICE_LINK_REQUEST_STARTUP_RETRY_MILLIS)
    } else {
        std::time::Duration::from_secs(DEVICE_LINK_REQUEST_RETRY_SECS)
    }
}

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LinkInputClassification {
    pub kind: String,
    pub is_complete: bool,
    pub is_valid: bool,
    pub normalized_input: String,
    pub app_key_pubkey: String,
    pub admin_app_key_pubkey: String,
    pub has_link_secret: bool,
    pub error: String,
}

impl From<iris_drive_core::LinkInputClassification> for LinkInputClassification {
    fn from(value: iris_drive_core::LinkInputClassification) -> Self {
        Self {
            kind: value.kind,
            is_complete: value.is_complete,
            is_valid: value.is_valid,
            normalized_input: value.normalized_input,
            app_key_pubkey: value.app_key_pubkey,
            admin_app_key_pubkey: value.admin_app_key_pubkey,
            has_link_secret: value.has_link_secret,
            error: value.error,
        }
    }
}

#[uniffi::export]
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn classify_link_input(input: String) -> LinkInputClassification {
    iris_drive_core::classify_link_input(&input).into()
}

#[uniffi::export]
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn validate_link_input(input: String) -> LinkInputClassification {
    iris_drive_core::classify_link_input(&input).into()
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
            NativeAppAction::CreateProfile { app_key_label } => {
                self.create_profile(&app_key_label);
            }
            NativeAppAction::RestoreProfile {
                recovery_secret,
                app_key_label,
            } => {
                self.restore_profile(&recovery_secret, &app_key_label);
            }
            NativeAppAction::AdmitAppKeyWithRecoveryPhrase {
                recovery_phrase,
                label,
            } => {
                self.admit_app_key_with_recovery_phrase(&recovery_phrase, &label);
            }
            NativeAppAction::LinkDevice {
                link_target,
                app_key_label,
            } => {
                self.link_device(&link_target, &app_key_label);
            }
            NativeAppAction::Logout => self.logout(),
            NativeAppAction::ApproveDevice { request, label } => {
                self.approve_app_key(&request, &label);
            }
            NativeAppAction::RejectDevice { request } => {
                self.reject_device(&request);
            }
            NativeAppAction::ResetInvite => self.reset_invite(),
            NativeAppAction::RevokeDevice { app_key_pubkey } => {
                self.revoke_app_key(&app_key_pubkey);
            }
            NativeAppAction::AppointAdmin { app_key_pubkey } => {
                self.set_device_admin_role(&app_key_pubkey, true);
            }
            NativeAppAction::DemoteAdmin { app_key_pubkey } => {
                self.set_device_admin_role(&app_key_pubkey, false);
            }
            NativeAppAction::AddRelay { url } => self.add_relay(&url),
            NativeAppAction::RemoveRelay { url } => self.remove_relay(&url),
            NativeAppAction::ResetRelays => self.reset_relays(),
            NativeAppAction::AddBackupTarget { target, label } => {
                self.add_backup_target(&target, &label);
            }
            NativeAppAction::RemoveBackupTarget { target } => {
                self.remove_backup_target(&target);
            }
            NativeAppAction::AddBlossomServer { url } => {
                self.add_blossom_server(&url);
            }
            NativeAppAction::RemoveBlossomServer { url } => {
                self.remove_blossom_server(&url);
            }
            NativeAppAction::SyncBackups { target } => {
                self.sync_backups(&target);
            }
            NativeAppAction::CheckBackups { target } => {
                self.check_backups(&target);
            }
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

    fn create_profile(&mut self, app_key_label: &str) {
        if self.initialized() {
            "already initialized".clone_into(&mut self.state.error);
            return;
        }
        let account = match Profile::create(Path::new(&self.data_dir), label_option(app_key_label))
        {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("creating profile: {error}");
                return;
            }
        };
        if let Err(error) = self.finish_profile_init(&account) {
            self.state.error = error;
        } else {
            self.set_sync_running(true);
        }
    }

    fn restore_profile(&mut self, recovery_secret: &str, app_key_label: &str) {
        if recovery_secret.trim().is_empty() {
            "recovery phrase or secret key is required".clone_into(&mut self.state.error);
            return;
        }
        if self.initialized() {
            "already initialized".clone_into(&mut self.state.error);
            return;
        }
        let account = match Profile::restore(
            Path::new(&self.data_dir),
            recovery_secret.trim(),
            label_option(app_key_label),
        ) {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("restoring profile: {error}");
                return;
            }
        };
        if let Err(error) = self.finish_profile_init(&account) {
            self.state.error = error;
        } else {
            self.set_sync_running(true);
        }
    }

    fn admit_app_key_with_recovery_phrase(&mut self, recovery_phrase: &str, label: &str) {
        let mut config = match self.load_config() {
            Ok(config) => config,
            Err(error) => {
                self.state.error = error;
                return;
            }
        };
        let Some(state) = config.profile.clone() else {
            "profile is required to recover this app key".clone_into(&mut self.state.error);
            return;
        };
        let phrase = if recovery_phrase.trim().is_empty() {
            match iris_drive_core::recovery_phrase::load_recovery_phrase(recovery_phrase_path_in(
                Path::new(&self.data_dir),
            )) {
                Ok(phrase) => phrase,
                Err(error) => {
                    self.state.error = format!("loading recovery phrase: {error}");
                    return;
                }
            }
        } else {
            recovery_phrase.trim().to_string()
        };
        let mut account = match Profile::load(state, Path::new(&self.data_dir)) {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("loading profile: {error}");
                return;
            }
        };
        if let Err(error) =
            account.admit_current_app_key_with_recovery_phrase(&phrase, label_option(label))
        {
            self.state.error = format!("recovering app key: {error}");
            return;
        }
        config.profile = Some(account.state);
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
        } else {
            self.set_sync_running(true);
        }
    }

    fn link_device(&mut self, link_target: &str, app_key_label: &str) {
        let target = match resolve_app_key_link_target(link_target) {
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
        let link_result = Profile::link_to_profile(
            Path::new(&self.data_dir),
            target.profile_id,
            target.admin_app_key_hex.clone(),
            label_option(app_key_label),
        );
        let mut account = match link_result {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("linking AppKey: {error}");
                return;
            }
        };
        let link_secret = if target.link_secret.trim().is_empty() {
            account.state.app_key_link_secret.clone()
        } else {
            target.link_secret
        };
        if let Err(error) = account.state.queue_outbound_app_key_link_request(
            target.admin_app_key_hex,
            &link_secret,
            unix_now_seconds(),
        ) {
            self.state.error = format!("queueing device link request: {error}");
            return;
        }
        if let Err(error) = self.finish_profile_init(&account) {
            self.state.error = error;
        } else {
            self.set_sync_running(true);
        }
    }

    fn logout(&mut self) {
        match iris_drive_core::logout_local_profile(Path::new(&self.data_dir)) {
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

    fn approve_app_key(&mut self, request: &str, label: &str) {
        let request = request.trim();
        if request.is_empty() {
            "device request is required".clone_into(&mut self.state.error);
            return;
        }
        let request = match decode_app_key_approval_request(request) {
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
        let Some(state) = config.profile.clone() else {
            "profile admin is required to approve AppKeys".clone_into(&mut self.state.error);
            return;
        };
        if !state.can_admin_profile() {
            "profile admin is required to approve AppKeys".clone_into(&mut self.state.error);
            return;
        }
        if request
            .profile_id
            .is_some_and(|profile_id| profile_id != state.profile_id)
        {
            "device request is for a different profile".clone_into(&mut self.state.error);
            return;
        }
        let label = label_option(label).or(request.label);
        let mut account = match Profile::load(state, Path::new(&self.data_dir)) {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("loading profile: {error}");
                return;
            }
        };
        if let Err(error) = account.approve_app_key(&request.app_key_hex, label) {
            self.state.error = format!("approving AppKey: {error}");
            return;
        }
        config.profile = Some(account.state);
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
        let Some(state) = config.profile.as_mut() else {
            "profile admin is required to reset invites".clone_into(&mut self.state.error);
            return;
        };
        if !state.can_admin_profile() {
            "profile admin is required to reset invites".clone_into(&mut self.state.error);
            return;
        }
        state.reset_app_key_link_secret();
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
        }
    }

    fn reject_device(&mut self, request: &str) {
        let request = request.trim();
        if request.is_empty() {
            "device request is required".clone_into(&mut self.state.error);
            return;
        }
        let request = match decode_app_key_approval_request(request) {
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
        let Some(state) = config.profile.as_mut() else {
            "profile admin is required to reject AppKeys".clone_into(&mut self.state.error);
            return;
        };
        if !state.can_admin_profile() {
            "profile admin is required to reject AppKeys".clone_into(&mut self.state.error);
            return;
        }
        if request
            .profile_id
            .is_some_and(|profile_id| profile_id != state.profile_id)
        {
            "device request is for a different profile".clone_into(&mut self.state.error);
            return;
        }
        if let Err(error) = state.reject_inbound_app_key_link_request(&request.app_key_hex) {
            self.state.error = format!("rejecting AppKey: {error}");
            return;
        }
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
        }
    }

    fn revoke_app_key(&mut self, app_key_pubkey: &str) {
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
            "profile admin is required to revoke AppKeys".clone_into(&mut self.state.error);
            return;
        };
        if state.app_key_pubkey == app_key_pubkey {
            "cannot revoke this AppKey from itself".clone_into(&mut self.state.error);
            return;
        }
        let mut account = match Profile::load(state, Path::new(&self.data_dir)) {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("loading profile: {error}");
                return;
            }
        };
        if let Err(error) = account.revoke_app_key(&app_key_pubkey) {
            self.state.error = format!("revoking AppKey: {error}");
            return;
        }
        config.profile = Some(account.state);
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
        }
    }

    fn set_device_admin_role(&mut self, app_key_pubkey: &str, make_admin: bool) {
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
            "admin profile is required to manage device admins".clone_into(&mut self.state.error);
            return;
        };
        let mut account = match Profile::load(state, Path::new(&self.data_dir)) {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("loading profile: {error}");
                return;
            }
        };
        let result = if make_admin {
            account.appoint_admin(&app_key_pubkey)
        } else {
            account.demote_admin(&app_key_pubkey)
        };
        if let Err(error) = result {
            self.state.error = format!("updating device role: {error}");
            return;
        }
        config.profile = Some(account.state);
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

    fn add_backup_target(&mut self, target: &str, label: &str) {
        if let Err(error) =
            core_add_backup_target(Path::new(&self.data_dir), target, label_option(label))
        {
            self.state.error = format!("adding backup target: {error:#}");
        }
    }

    fn remove_backup_target(&mut self, target: &str) {
        if let Err(error) = core_remove_backup_target(Path::new(&self.data_dir), target) {
            self.state.error = format!("removing backup target: {error:#}");
        }
    }

    fn add_blossom_server(&mut self, url: &str) {
        if let Err(error) = core_add_blossom_server(Path::new(&self.data_dir), url) {
            self.state.error = format!("adding Blossom endpoint: {error:#}");
        }
    }

    fn remove_blossom_server(&mut self, url: &str) {
        if let Err(error) = core_remove_blossom_server(Path::new(&self.data_dir), url) {
            self.state.error = format!("removing Blossom endpoint: {error:#}");
        }
    }

    fn sync_backups(&mut self, target: &str) {
        let data_dir = self.data_dir.clone();
        let target = label_option(target);
        match block_on_backup_operation(async move {
            core_sync_backups(Path::new(&data_dir), target.as_deref()).await
        }) {
            Ok(_) => {}
            Err(error) => self.state.error = format!("syncing backups: {error:#}"),
        }
    }

    fn check_backups(&mut self, target: &str) {
        let data_dir = self.data_dir.clone();
        let target = label_option(target);
        match block_on_backup_operation(async move {
            core_check_backups(
                Path::new(&data_dir),
                target.as_deref(),
                default_backup_check_sample_size(),
            )
            .await
        }) {
            Ok(_) => {}
            Err(error) => self.state.error = format!("checking backups: {error:#}"),
        }
    }

    fn initialized(&self) -> bool {
        key_path_in(Path::new(&self.data_dir)).exists()
            && self
                .load_config()
                .ok()
                .and_then(|config| config.profile)
                .is_some()
    }

    fn current_authorization_state(&self) -> Option<AppKeyAuthorizationState> {
        let mut account = self.load_config().ok()?.profile?;
        account.recompute_authorization();
        Some(account.authorization_state)
    }

    fn current_device_is_revoked(&self) -> bool {
        self.state
            .ui
            .profile
            .as_ref()
            .is_some_and(|account| account.authorization_state == "revoked")
            || self.current_authorization_state() == Some(AppKeyAuthorizationState::Revoked)
    }

    fn load_config(&self) -> Result<AppConfig, String> {
        AppConfig::load_or_default(config_path_in(Path::new(&self.data_dir)))
            .map_err(|error| format!("loading config: {error}"))
    }

    fn finish_profile_init(&self, account: &Profile) -> Result<(), String> {
        let mut config = self.load_config()?;
        config.profile = Some(account.state.clone());
        if config.drive(iris_drive_core::PRIMARY_DRIVE_ID).is_none() {
            config.upsert_drive(Drive::primary(account.state.root_scope_id()));
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
            if config.profile.is_none() {
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
        self.state.ui.backups = backup_ui_rows_for_config(&config);
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

        let Some(raw_account) = config.profile.as_ref() else {
            self.refresh_ui_summary(None);
            return;
        };
        let mut account = raw_account.clone();
        account.recompute_authorization();
        let profile = iris_profile_summary(&account);
        self.state.ui.profile = Some(UiProfile {
            profile_id: profile.profile_id,
            current_app_key_pubkey: profile.current_app_key_pubkey_hex,
            current_app_key_npub: profile.current_app_key_npub,
            current_app_key_label: profile.current_app_key_label.unwrap_or_default(),
            app_key_label: account.app_key_label.clone().unwrap_or_default(),
            authorization_state: profile.authorization_state,
            can_admin_profile: profile.can_admin_profile,
            can_write_roots: profile.can_write_roots,
            active_app_key_count: profile.active_app_key_count as u64,
            profile_roster_op_count: profile.profile_roster_op_count as u64,
            current_key_epoch: profile.current_key_epoch,
            recovery_phrase_facet_count: profile.recovery_phrase_facet_count as u64,
            nip46_facet_count: profile.nip46_facet_count as u64,
            social_profile_facet_count: profile.social_profile_facet_count as u64,
            missing_key_wraps: profile.missing_key_wrap_npubs,
            can_export_recovery_phrase: recovery_phrase_path_in(Path::new(&self.data_dir)).exists(),
            app_key_link_request: app_key_link_request_url(&account),
            app_key_link_invite: app_key_link_invite_url(&account),
            inbound_app_key_link_requests: inbound_app_key_link_requests(&account),
        });
        self.state.ui.shares = ui_shares_for_config(&config, &account.app_key_pubkey);
        if account.authorization_state == AppKeyAuthorizationState::Revoked {
            self.set_sync_running(false);
            self.state.ui.roots.clear();
            self.state.ui.shares.clear();
            self.state.ui.devices.clear();
            self.state.ui.snapshot_link.clear();
            self.refresh_ui_summary(None);
            return;
        }
        let fips_status = load_native_fips_status(Path::new(&self.data_dir));
        let ui_fips_status = ui_fips_status(fips_status.as_ref());
        self.state.ui.devices = devices_from_account(&account, &ui_fips_status);
        update_snapshot_link(&mut self.state, &config);
        self.refresh_provider_summary();
        self.refresh_ui_summary(Some(ui_fips_status));
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
        value
            .get("change_key")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .clone_into(&mut self.state.ui.provider_change_key);
        self.state.ui.provider_directory_paths = value
            .get("directory_paths")
            .and_then(serde_json::Value::as_array)
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default();
    }

    fn refresh_ui_summary(&mut self, fips_status: Option<UiFipsStatus>) {
        let setup_state = self.state.ui.profile.as_ref().map_or_else(
            || "not_configured".to_owned(),
            |account| account.authorization_state.clone(),
        );
        primary_status_for_setup_state(&setup_state).clone_into(&mut self.state.ui.primary_status);
        self.state.ui.setup_state = setup_state;
        let setup_flags = setup_state_flags(&self.state.ui.setup_state);
        self.state.ui.setup_complete = setup_flags.setup_complete;
        self.state.ui.awaiting_approval = setup_flags.awaiting_approval;
        self.state.ui.revoked = setup_flags.revoked;
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
#[allow(clippy::too_many_lines)]
async fn run_device_link_exchange_async(
    data_dir: &str,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let config_dir = Path::new(data_dir);
    let startup_config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    if startup_config.profile.is_none() {
        return Ok(());
    }

    let device = iris_drive_core::AppKey::load(key_path_in(config_dir))
        .map_err(|error| format!("loading app key: {error}"))?;
    let relays = if startup_config.relays.is_empty() {
        default_relays()
    } else {
        normalized_config_relays(&startup_config.relays)
            .map_err(|error| format!("normalizing relays: {error}"))?
    };
    let account_state = startup_config
        .profile
        .as_ref()
        .expect("account checked above");
    let root_scope_id = account_state.root_scope_id();
    let relay_filters = iris_drive_core::relay_sync::subscription_filters(
        &account_state.app_key_pubkey,
        &root_scope_id,
        iris_drive_core::PRIMARY_DRIVE_ID,
    );
    let relay_client = iris_drive_core::relay_sync::connect(&relays)
        .await
        .map_err(|error| format!("connecting device-link relays: {error}"))?;
    relay_client
        .subscribe(relay_filters, None)
        .await
        .map_err(|error| format!("subscribing device-link relays: {error}"))?;
    let mut relay_notifications = relay_client.notifications();
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
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(
        DEVICE_LINK_EXCHANGE_TICK_MILLIS,
    ));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let _ = drive_device_link_exchange_tick(
        config_dir,
        &relay_client,
        device.keys(),
        &sync,
        &mut sent_requests,
        &mut sent_rosters,
        &acked_rosters,
    )
    .await?;

    while !stop.load(Ordering::Acquire) {
        tokio::select! {
            _ = tick.tick() => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let _ = drive_device_link_exchange_tick(
                    config_dir,
                    &relay_client,
                    device.keys(),
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
            notification = relay_notifications.recv() => {
                match notification {
                    Ok(nostr_sdk::RelayPoolNotification::Event { event, .. }) => {
                        if let Err(error) = handle_native_app_key_link_request_event(config_dir, &event) {
                            tracing::warn!(error = %error, event_id = %event.id.to_hex(), "handling native device-link relay request failed");
                        }
                    }
                    Ok(nostr_sdk::RelayPoolNotification::Shutdown) => {
                        tracing::warn!("native device-link relay notifications shut down");
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "native device-link relay receiver lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    let _ = relay_client.disconnect().await;
    Ok(())
}

#[cfg(not(test))]
async fn drive_device_link_exchange_tick(
    config_dir: &Path,
    relay_client: &nostr_sdk::Client,
    device_keys: &nostr_sdk::Keys,
    sync: &iris_drive_core::FsFipsBlockSync,
    sent_requests: &mut BTreeMap<String, SentDeviceLinkRequest>,
    sent_rosters: &mut BTreeMap<String, std::time::Instant>,
    acked_rosters: &BTreeSet<String>,
) -> Result<bool, String> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    let Some(state) = config.profile.as_ref() else {
        return Ok(false);
    };

    sync.refresh_authorized_peers(&config).await;
    send_native_pending_app_key_link_request(relay_client, device_keys, sync, state, sent_requests)
        .await?;
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
async fn send_native_pending_app_key_link_request(
    relay_client: &nostr_sdk::Client,
    device_keys: &nostr_sdk::Keys,
    sync: &iris_drive_core::FsFipsBlockSync,
    state: &iris_drive_core::ProfileState,
    sent_requests: &mut BTreeMap<String, SentDeviceLinkRequest>,
) -> Result<(), String> {
    let Some(frame) = pending_app_key_link_request_frame(state) else {
        return Ok(());
    };
    let Some(pending) = state.outbound_app_key_link_request.as_ref() else {
        return Ok(());
    };
    let fingerprint = format!(
        "{}:{}:{}",
        pending.admin_app_key_pubkey, state.app_key_pubkey, pending.requested_at
    );
    let now = std::time::Instant::now();
    if !app_key_link_request_send_due(sent_requests.get(&fingerprint).copied(), now) {
        return Ok(());
    }
    let admin_npub = pubkey_npub(&pending.admin_app_key_pubkey);
    let bytes = serde_json::to_vec(&frame)
        .map_err(|error| format!("encoding device link request: {error}"))?;
    let relay_event_id = iris_drive_core::relay_sync::publish_app_key_link_request(
        relay_client,
        device_keys,
        &frame,
    )
    .await
    .map_err(|error| format!("publishing device link request relay event: {error}"))?;
    let attempts = sent_requests
        .get(&fingerprint)
        .map_or(1, |sent| sent.attempts.saturating_add(1));
    sent_requests.insert(
        fingerprint,
        SentDeviceLinkRequest {
            last_sent: now,
            attempts,
        },
    );
    match sync
        .send_app_message(&admin_npub, DEVICE_LINK_REQUEST_APP_TOPIC, bytes)
        .await
    {
        Ok(()) => {
            tracing::debug!(
                admin_npub,
                relay_event_id = %relay_event_id.to_hex(),
                requested_at = frame.requested_at,
                "sent native device-link request over relay and FIPS"
            );
        }
        Err(error) => tracing::warn!(
            admin_npub,
            relay_event_id = %relay_event_id.to_hex(),
            error = %error,
            "sent native device-link request over relay, FIPS send failed"
        ),
    }
    Ok(())
}

#[cfg(not(test))]
async fn send_native_authorized_device_link_rosters(
    _config_dir: &Path,
    sync: &iris_drive_core::FsFipsBlockSync,
    state: &iris_drive_core::ProfileState,
    sent_rosters: &mut BTreeMap<String, std::time::Instant>,
    acked_rosters: &BTreeSet<String>,
) -> Result<(), String> {
    if !state.can_admin_profile() {
        return Ok(());
    }
    let Some(app_keys) = state.app_keys.as_ref() else {
        return Ok(());
    };
    if !app_keys.contains(&state.app_key_pubkey) {
        return Ok(());
    }

    let now = std::time::Instant::now();
    let due_devices = device_link_roster_recipients(state)
        .into_iter()
        .filter(|recipient| {
            if acked_rosters.contains(&recipient.roster_fingerprint) {
                return false;
            }
            !sent_rosters
                .get(&recipient.roster_fingerprint)
                .is_some_and(|last_sent| {
                    now.duration_since(*last_sent)
                        < std::time::Duration::from_secs(DEVICE_LINK_ROSTER_RETRY_SECS)
                })
        })
        .collect::<Vec<_>>();
    if due_devices.is_empty() {
        return Ok(());
    }

    let Some(frame) = device_link_roster_frame(state, unix_now_seconds()) else {
        return Ok(());
    };
    let bytes = serde_json::to_vec(&frame)
        .map_err(|error| format!("encoding device link roster: {error}"))?;
    for recipient in due_devices {
        let recipient_npub = pubkey_npub(&recipient.app_key_pubkey);
        match sync
            .send_app_message(&recipient_npub, DEVICE_LINK_ROSTER_APP_TOPIC, bytes.clone())
            .await
        {
            Ok(()) => {
                sent_rosters.insert(recipient.roster_fingerprint, now);
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
async fn handle_native_device_link_app_message(
    config_dir: &Path,
    sync: &iris_drive_core::FsFipsBlockSync,
    message: &iris_drive_core::FipsAppMessage,
    acked_rosters: &mut BTreeSet<String>,
) -> Result<bool, String> {
    match message.topic.as_str() {
        DEVICE_LINK_REQUEST_APP_TOPIC => handle_native_app_key_link_request(config_dir, message),
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
fn handle_native_app_key_link_request(
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
    let app_key_hex = normalize_pubkey(&frame.app_key_pubkey)?;
    let link_secret = if frame.link_secret.trim().is_empty() {
        app_key_approval_link_secret(&frame.url)
    } else {
        frame.link_secret
    };

    let mut config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    let Some(state) = config.profile.as_mut() else {
        return Ok(true);
    };
    let changed = state
        .record_inbound_app_key_link_request(
            frame.profile_id,
            &app_key_hex,
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
            device_npub = pubkey_npub(&app_key_hex),
            requested_at = frame.requested_at,
            "received native device-link request over FIPS"
        );
    }
    Ok(true)
}

#[cfg(not(test))]
fn handle_native_app_key_link_request_event(
    config_dir: &Path,
    event: &nostr_sdk::Event,
) -> Result<bool, String> {
    if !iris_drive_core::nostr_events::is_app_key_link_request_event_coordinate(event) {
        return Ok(false);
    }
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    let outcome =
        iris_drive_core::relay_sync::apply_remote_app_key_link_request_event(&mut config, event)
            .map_err(|error| format!("applying device link request relay event: {error}"))?;
    if matches!(
        outcome,
        iris_drive_core::relay_sync::DeviceLinkRequestApply::Recorded
    ) {
        config
            .save(config_path_in(config_dir))
            .map_err(|error| format!("saving config: {error}"))?;
        tracing::debug!(
            event_id = %event.id.to_hex(),
            device_npub = pubkey_npub(&event.pubkey.to_hex()),
            "received native device-link request over relay"
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
    let admin_app_key_hex = normalize_pubkey(&frame.admin_app_key_pubkey)?;
    let sender_hex = normalize_pubkey(&message.peer_id).ok();

    let mut config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    let Some(state) = config.profile.as_ref() else {
        return Ok(true);
    };
    if state.can_admin_profile() {
        return Ok(true);
    }
    if sender_hex.as_deref() != Some(admin_app_key_hex.as_str()) {
        return Ok(true);
    }

    let outcome = iris_drive_core::relay_sync::apply_device_link_roster_frame(
        &mut config,
        &frame,
        &admin_app_key_hex,
    )
    .map_err(|error| format!("applying signed device-link profile roster ops: {error}"))?;
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
    let state = config.profile.as_ref().expect("account still present");
    let ack_frame = if accepted {
        device_link_roster_ack_frame(state, &admin_app_key_hex, unix_now_seconds())
    } else {
        None
    };
    if changed {
        config
            .save(config_path_in(config_dir))
            .map_err(|error| format!("saving config: {error}"))?;
        tracing::debug!(
            peer = message.peer_id,
            admin_app_key_npub = pubkey_npub(&admin_app_key_hex),
            apply_outcome = ?outcome,
            "accepted native device-link roster over FIPS"
        );
    }
    if let Some(frame) = ack_frame {
        send_native_device_link_roster_ack(sync, &frame).await?;
    }
    let should_sync_roots = changed
        && config
            .profile
            .as_ref()
            .is_some_and(iris_drive_core::ProfileState::is_authorized);
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
    let admin_app_key_hex = normalize_pubkey(&frame.admin_app_key_pubkey)?;
    let app_key_hex = normalize_pubkey(&frame.app_key_pubkey)?;
    if normalize_pubkey(&message.peer_id).ok().as_deref() != Some(app_key_hex.as_str()) {
        return Ok(true);
    }

    let config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    let Some(state) = config.profile.as_ref() else {
        return Ok(true);
    };
    if admin_app_key_hex != frame.admin_app_key_pubkey
        || app_key_hex != frame.app_key_pubkey
        || !device_link_roster_ack_matches_state(state, &frame)
    {
        return Ok(true);
    }

    acked_rosters.insert(frame.roster_fingerprint);
    Ok(true)
}

#[cfg(not(test))]
async fn send_native_device_link_roster_ack(
    sync: &iris_drive_core::FsFipsBlockSync,
    frame: &DeviceLinkRosterAckFrame,
) -> Result<(), String> {
    sync.send_app_message(
        &pubkey_npub(&frame.admin_app_key_pubkey),
        DEVICE_LINK_ROSTER_ACK_APP_TOPIC,
        serde_json::to_vec(frame)
            .map_err(|error| format!("encoding device-link roster ack: {error}"))?,
    )
    .await
    .map_err(|error| format!("sending device-link roster ack over FIPS: {error}"))?;
    Ok(())
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
        discovery_scope: normalized
            .get("discovery_scope")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        roster_label: normalized
            .get("roster_label")
            .and_then(Value::as_str)
            .unwrap_or("0/0 online")
            .to_owned(),
        roster_peer_count: normalized_u64(&normalized, "roster_peer_count"),
        roster_online_device_count: normalized_u64(&normalized, "roster_online_device_count"),
        roster_direct_device_count: normalized_u64(&normalized, "roster_direct_device_count"),
        online_device_count: normalized_u64(&normalized, "online_device_count"),
        direct_device_count: normalized_u64(&normalized, "direct_device_count"),
        mesh_device_count: normalized_u64(&normalized, "mesh_device_count"),
        other_peer_count: normalized_u64(&normalized, "other_peer_count"),
        online_devices,
        direct_devices,
        mesh_devices,
        peer_statuses: ui_fips_peer_statuses(normalized.get("peer_statuses")),
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
        roster_label: "0/0 online".to_owned(),
        ..UiFipsStatus::default()
    }
}

fn normalized_u64(status: &Value, key: &str) -> u64 {
    status.get(key).and_then(Value::as_u64).unwrap_or_default()
}

fn ui_fips_peer_statuses(value: Option<&Value>) -> Vec<UiFipsPeerStatus> {
    value
        .and_then(Value::as_array)
        .map(|statuses| {
            statuses
                .iter()
                .filter_map(|status| {
                    Some(UiFipsPeerStatus {
                        npub: status.get("npub")?.as_str()?.to_owned(),
                        transport_type: status
                            .get("transport_type")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        srtt_ms: status.get("srtt_ms").and_then(Value::as_u64),
                        connection_label: status
                            .get("connection_label")
                            .and_then(Value::as_str)
                            .unwrap_or("Online")
                            .to_owned(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
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

fn export_recovery_secret_value(data_dir: &str) -> RecoverySecretExport {
    let config_dir = Path::new(data_dir);
    let config = match AppConfig::load_or_default(config_path_in(config_dir)) {
        Ok(config) => config,
        Err(error) => {
            return RecoverySecretExport {
                error: format!("loading config: {error}"),
                ..RecoverySecretExport::default()
            };
        }
    };
    let Some(account) = config.profile else {
        return RecoverySecretExport {
            error: "profile is required".to_owned(),
            ..RecoverySecretExport::default()
        };
    };
    let phrase_path = recovery_phrase_path_in(config_dir);
    let phrase = match iris_drive_core::recovery_phrase::load_recovery_phrase(&phrase_path) {
        Ok(phrase) => phrase,
        Err(error) => {
            return RecoverySecretExport {
                error: format!("loading recovery phrase: {error}"),
                ..RecoverySecretExport::default()
            };
        }
    };
    let recovery_key = match iris_drive_core::identity::RecoveryKey::from_recovery_phrase(
        &phrase,
        config_dir.join("recovery-export-check"),
    ) {
        Ok(key) => key,
        Err(error) => {
            return RecoverySecretExport {
                error: format!("validating recovery phrase key: {error}"),
                ..RecoverySecretExport::default()
            };
        }
    };
    let projection = account.profile_projection();
    let recovery_pubkey = recovery_key.pubkey_hex();
    let phrase_matches_profile =
        projection
            .active_facets
            .get(&recovery_pubkey)
            .is_some_and(|facet| {
                facet.has_purpose(iris_drive_core::IrisProfileKeyPurpose::RecoveryPhrase)
            });
    if !phrase_matches_profile {
        return RecoverySecretExport {
            error: "recovery phrase does not match IrisProfile".to_owned(),
            ..RecoverySecretExport::default()
        };
    }
    let phrase_secret = match iris_drive_core::recovery_phrase::recovery_phrase_to_nsec(&phrase) {
        Ok(secret) => secret,
        Err(error) => {
            return RecoverySecretExport {
                error: format!("validating recovery phrase: {error}"),
                ..RecoverySecretExport::default()
            };
        }
    };
    RecoverySecretExport {
        can_export: true,
        words: phrase.split_whitespace().map(ToOwned::to_owned).collect(),
        recovery_phrase: phrase,
        secret_key: phrase_secret,
        error: String::new(),
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
    normalized_relay_statuses_for_relays(relays, None)
        .into_iter()
        .map(|relay| UiRelayStatus {
            url: relay.url,
            status: relay.status,
            status_label: relay.status_label,
            health: relay.health,
        })
        .collect()
}

fn block_on_backup_operation<T>(
    future: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building backup runtime")?;
    runtime.block_on(future)
}

fn backup_ui_rows_for_config(config: &AppConfig) -> Vec<UiBackup> {
    let targets = effective_backup_targets(config);

    if targets.is_empty() {
        return default_backups();
    }

    targets.iter().map(ui_backup_from_target).collect()
}

fn ui_backup_from_target(target: &BackupTarget) -> UiBackup {
    let summary = backup_target_summary(target);
    UiBackup {
        id: summary.id,
        kind: summary.kind,
        target: summary.target,
        label: summary.title,
        configured_label: summary.label.unwrap_or_default(),
        state: summary.state,
        detail: summary.detail,
        enabled: summary.enabled,
    }
}

fn default_backups() -> Vec<UiBackup> {
    DEFAULT_BLOSSOM_SERVERS
        .iter()
        .filter_map(|server| blossom_backup_target(server))
        .map(|target| ui_backup_from_target(&target))
        .collect()
}

fn label_option(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn normalize_pubkey(input: &str) -> Result<String, String> {
    iris_drive_core::normalize_app_key_pubkey(input).map_err(|error| error.to_string())
}

fn pubkey_npub(hex: &str) -> String {
    PublicKey::from_hex(hex)
        .ok()
        .and_then(|pubkey| pubkey.to_bech32().ok())
        .unwrap_or_else(|| hex.to_owned())
}

fn devices_from_account(
    state: &iris_drive_core::ProfileState,
    fips_status: &UiFipsStatus,
) -> Vec<UiDevice> {
    let Some(app_keys) = state.app_keys.as_ref() else {
        return Vec::new();
    };
    let current_device_npub = pubkey_npub(&state.app_key_pubkey);
    let current_device_online = fips_status.fresh
        && (fips_status.endpoint_npub.is_empty()
            || fips_status.endpoint_npub == current_device_npub);
    let connectivity = device_connectivity_from_fips_status(fips_status);

    device_roster_rows(
        &app_keys.app_actors,
        &state.app_key_pubkey,
        state.can_admin_profile(),
        current_device_online,
        &connectivity,
    )
    .iter()
    .map(|device| UiDevice {
        pubkey: device.npub.clone(),
        label: device.label.clone().unwrap_or_default(),
        display_label: device.display_label.clone(),
        state: device.state.clone(),
        state_label: device.state_label.clone(),
        connection_label: device.connection_label.clone(),
        connection_state: device.connection_state.clone(),
        role: device.role.clone(),
        role_label: device.role_label.clone(),
        detail: device.npub.clone(),
        is_current_device: device.is_current_device,
        is_online: device.is_online,
        can_revoke: device.can_revoke,
        can_appoint_admin: device.can_appoint_admin,
        can_demote_admin: device.can_demote_admin,
    })
    .collect()
}

fn ui_shares_for_config(config: &AppConfig, current_app_pubkey: &str) -> Vec<UiShare> {
    iris_drive_core::shared_folder_views(
        &config.shared_folders,
        &config.share_shortcuts,
        current_app_pubkey,
    )
    .into_iter()
    .map(|share| UiShare {
        share_id: share.share_id.to_string(),
        display_name: share.display_name,
        shared_with_me_path: share.shared_with_me_path,
        role: share_role_key(share.local_role).to_owned(),
        role_label: share_role_label(share.local_role).to_owned(),
        key_status: share.key_status.as_str().to_owned(),
        key_status_label: share.key_status.label().to_owned(),
        can_write: share.can_write,
        can_admin: share.can_admin,
        current_key_epoch: share.current_key_epoch,
        has_current_key_wrap: share.has_current_key_wrap,
        key_unavailable: share.key_unavailable,
        repair_needed: share.repair_needed,
        missing_key_wraps: share
            .missing_key_wrap_pubkeys
            .into_iter()
            .map(|pubkey| iris_drive_core::device_summary::pubkey_npub(&pubkey))
            .collect(),
        participant_count: share.participant_count as u64,
        shortcut_paths: share.shortcut_paths,
    })
    .collect()
}

fn share_role_key(role: iris_drive_core::ShareRole) -> &'static str {
    match role {
        iris_drive_core::ShareRole::Admin => "admin",
        iris_drive_core::ShareRole::Editor => "editor",
        iris_drive_core::ShareRole::Reader => "reader",
    }
}

fn share_role_label(role: iris_drive_core::ShareRole) -> &'static str {
    match role {
        iris_drive_core::ShareRole::Admin => "Admin",
        iris_drive_core::ShareRole::Editor => "Editor",
        iris_drive_core::ShareRole::Reader => "Reader",
    }
}

fn device_connectivity_from_fips_status(fips_status: &UiFipsStatus) -> DeviceConnectivity {
    if !fips_status.fresh {
        return DeviceConnectivity::default();
    }
    DeviceConnectivity {
        online_devices: fips_status.online_devices.iter().cloned().collect(),
        direct_devices: fips_status.direct_devices.iter().cloned().collect(),
        mesh_devices: fips_status.mesh_devices.iter().cloned().collect(),
        peer_statuses: fips_status
            .peer_statuses
            .iter()
            .map(|peer| {
                (
                    peer.npub.clone(),
                    DeviceConnectionDetails {
                        transport_type: label_option(&peer.transport_type),
                        srtt_ms: peer.srtt_ms,
                    },
                )
            })
            .collect(),
    }
}

fn app_key_link_request_url(state: &iris_drive_core::ProfileState) -> String {
    if state.can_admin_profile()
        || state.authorization_state != AppKeyAuthorizationState::AwaitingApproval
    {
        return String::new();
    }
    encode_app_key_approval_request(
        state.profile_id,
        &state.app_key_pubkey,
        state
            .outbound_app_key_link_request
            .as_ref()
            .and_then(|request| {
                (!request.link_secret.trim().is_empty()).then_some(request.link_secret.as_str())
            })
            .unwrap_or(state.app_key_link_secret.as_str()),
        state.app_key_label.as_deref(),
    )
}

fn app_key_link_invite_url(state: &iris_drive_core::ProfileState) -> String {
    if !state.can_admin_profile() {
        return String::new();
    }
    iris_drive_core::app_key_link_invite::encode_app_key_link_invite(
        state.profile_id,
        &state.app_key_pubkey,
        &state.app_key_link_secret,
    )
    .unwrap_or_default()
}

fn inbound_app_key_link_requests(
    state: &iris_drive_core::ProfileState,
) -> Vec<UiAppKeyLinkRequest> {
    if !state.can_admin_profile() {
        return Vec::new();
    }
    state
        .inbound_app_key_link_requests
        .iter()
        .map(|request| UiAppKeyLinkRequest {
            app_key_pubkey: pubkey_npub(&request.app_key_pubkey),
            label: request.label.clone().unwrap_or_default(),
            requested_at: request.requested_at,
            request_link: encode_app_key_approval_request(
                state.profile_id,
                &request.app_key_pubkey,
                &request.link_secret,
                request.label.as_deref(),
            ),
        })
        .collect()
}

fn resolve_app_key_link_target(input: &str) -> Result<iris_drive_core::AppKeyLinkTarget, String> {
    iris_drive_core::resolve_app_key_link_target(input, None).map_err(|error| {
        if error.to_string().contains("IrisProfile UUID") {
            "paste an IrisProfile invite URL to link this AppKey".to_owned()
        } else {
            error.to_string()
        }
    })
}

fn decode_app_key_approval_request(request: &str) -> Result<AppKeyApprovalRequest, String> {
    if let Some(request) =
        parse_app_key_approval_request(request).map_err(|error| error.to_string())?
    {
        return Ok(request);
    }
    let device = normalize_pubkey(request)?;
    Ok(AppKeyApprovalRequest {
        profile_id: None,
        app_key_hex: device,
        link_secret: String::new(),
        label: None,
    })
}

#[cfg(not(test))]
fn app_key_approval_link_secret(request: &str) -> String {
    parse_app_key_approval_request(request)
        .ok()
        .flatten()
        .map(|request| request.link_secret)
        .unwrap_or_default()
        .trim()
        .to_owned()
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
        .profile
        .as_ref()
        .and_then(|account| {
            config
                .drive(iris_drive_core::PRIMARY_DRIVE_ID)
                .and_then(|drive| drive.device_roots.get(&account.app_key_pubkey))
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
mod backup_tests;
#[cfg(test)]
mod provider_tests;
#[cfg(test)]
mod tests;
