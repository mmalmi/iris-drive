use std::collections::BTreeMap;
#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::Context;
#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
use std::sync::atomic::{AtomicBool, Ordering};

use hashtree_core::{Cid, NHashData, nhash_encode_full};
#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
use iris_drive_core::app_key_link_transport::{
    APP_KEY_LINK_REQUEST_APP_TOPIC, APP_KEY_LINK_ROSTER_ACK_APP_TOPIC,
    APP_KEY_LINK_ROSTER_APP_TOPIC, AppKeyLinkRequestFrame, AppKeyLinkRosterAckFrame,
    AppKeyLinkRosterFrame, app_key_link_roster_ack_frame, app_key_link_roster_ack_matches_state,
    app_key_link_roster_frame, app_key_link_roster_recipients, pending_app_key_link_request_frame,
};
use iris_drive_core::app_key_link_transport::{
    AppKeyApprovalRequest, encode_app_key_approval_request, parse_app_key_approval_request,
};
use iris_drive_core::app_key_summary::{
    AppKeyConnectionDetails, AppKeyConnectivity, app_key_roster_rows, nostr_identity_summary,
    primary_status_for_setup_state, primary_status_label, setup_label_for_setup_state,
    setup_state_flags, sync_status_label,
};
use iris_drive_core::backup_ops::{
    add_backup_target as core_add_backup_target, add_blossom_server as core_add_blossom_server,
    check_backups as core_check_backups, default_backup_check_sample_size,
    effective_backup_targets, remove_backup_target as core_remove_backup_target,
    remove_blossom_server as core_remove_blossom_server, sync_backups as core_sync_backups,
};
use iris_drive_core::backup_summary::{backup_target_summary, blossom_backup_target};
use iris_drive_core::config::{DEFAULT_BLOSSOM_SERVERS, DEFAULT_RELAYS};
#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
use iris_drive_core::daemon::EmbeddedHashtreeHost;
#[cfg(any(test, target_os = "ios", target_os = "android"))]
use iris_drive_core::fips_status::fips_error_is_present;
#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
use iris_drive_core::fips_status::online_device_ids;
use iris_drive_core::fips_status::{normalize_fips_status_value, string_vec_from_json_array};
use iris_drive_core::paths::{config_path_in, key_path_in, recovery_phrase_path_in};
use iris_drive_core::relay_config::{dedupe_relay_urls, normalize_relay_url};
use iris_drive_core::relay_status::normalized_relay_statuses_for_relays;
use iris_drive_core::{AppConfig, AppKeyAuthorizationState, BackupTarget, Drive, Profile};
#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
use iris_drive_core::{Daemon, GatewayBind, GatewayProxyServer, GatewayServer};
#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
use nostr_sdk::Event;
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::ToBech32;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::actions::NativeAppAction;
pub use crate::native_link_input::{
    classify_link_input, validate_device_approval_input, validate_device_invite_input,
    validate_link_input,
};
#[cfg(test)]
pub(crate) use crate::native_provider::run_native_sync_once_with_drive_root_events_for_test;
use crate::native_provider::{
    install_rustls_crypto_provider, native_provider_import_content_link,
    native_provider_import_shared_file, native_sync_status_label, run_native_provider_list,
    run_native_sync_once,
};
pub(crate) use crate::native_provider::{
    native_provider_compose_path_json, native_provider_delete_json,
    native_provider_import_shared_file_json, native_provider_is_child_document_json,
    native_provider_list_json, native_provider_mkdir_json, native_provider_normalize_path_json,
    native_provider_read_json, native_provider_rename_json, native_provider_resolve_path_json,
    native_provider_write_json,
};
use crate::state::{
    NativeAppState, UiAppActor, UiAppKeyLinkRequest, UiBackup, UiFipsPeerStatus, UiFipsStatus,
    UiPaths, UiPendingShareInvite, UiProfile, UiRelayStatus, UiShare, UiShareMember, UiState,
    UiSyncRoot, UiSyncStatus,
};

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoverySecretExport {
    pub can_export: bool,
    pub recovery_phrase: String,
    pub words: Vec<String>,
    pub secret_key: String,
    pub error: String,
}

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct GeneratedRecoveryKey {
    pub words: Vec<String>,
    pub recovery_pubkey: String,
    pub error: String,
}

#[derive(uniffi::Record, Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriveLinkForCid {
    pub url: String,
    pub error: String,
}

#[uniffi::export]
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn export_recovery_secret(data_dir: String) -> RecoverySecretExport {
    export_recovery_secret_value(&data_dir)
}

#[uniffi::export]
#[must_use]
pub fn generate_recovery_key() -> GeneratedRecoveryKey {
    generate_recovery_key_value()
}

#[uniffi::export]
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn recovery_pubkey_for_phrase(recovery_phrase: String) -> GeneratedRecoveryKey {
    recovery_pubkey_for_phrase_value(&recovery_phrase)
}

#[uniffi::export]
#[must_use]
#[allow(clippy::needless_pass_by_value)]
pub fn drive_link_for_cid(root_cid: String) -> DriveLinkForCid {
    drive_link_for_cid_value(&root_cid)
}

pub(crate) fn native_calendar_export_json(data_dir: &str) -> Value {
    match run_native_calendar_export(data_dir) {
        Ok(calendar) => json!({
            "calendar": calendar,
            "error": "",
        }),
        Err(error) => json!({
            "calendar": null,
            "error": format!("{error:#}"),
        }),
    }
}

#[cfg(target_os = "android")]
#[path = "ffi_android_test_support.rs"]
mod android_test_support;
#[cfg(target_os = "android")]
pub(crate) use android_test_support::native_apply_owner_snapshot_for_test_json;

const DEFAULT_ROOT_STATUS: &str = "SAF provider root";
const DAEMON_STATUS_FILE_NAME: &str = "daemon-status.json";
const DAEMON_STATUS_FRESH_SECS: u64 = 15;
const NATIVE_FIPS_STATUS_FILE_NAME: &str = "native-fips-status.json";
#[cfg(any(test, target_os = "ios", target_os = "android"))]
const NATIVE_FIPS_STATUS_FRESH_SECS: u64 = 20;
#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
const NATIVE_SYNC_RELAY_TIMEOUT_SECS: u64 = 10;
#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
const APP_KEY_LINK_REQUEST_RETRY_SECS: u64 = 30;
#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
const APP_KEY_LINK_REQUEST_STARTUP_RETRY_MILLIS: u64 = 1_000;
#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
const APP_KEY_LINK_REQUEST_STARTUP_BURST_ATTEMPTS: u8 = 3;
#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
const APP_KEY_LINK_ROSTER_RETRY_SECS: u64 = 2;
#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
const APP_KEY_LINK_EXCHANGE_TICK_MILLIS: u64 = 5_000;
#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
const NATIVE_DIRECT_ROOT_EXCHANGE_MILLIS: u64 = 10_000;
#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
const NATIVE_APP_MESSAGE_DRAIN_LIMIT: usize = 4096;

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
#[derive(Debug, Clone, Copy)]
struct SentAppKeyLinkRequest {
    last_sent: std::time::Instant,
    attempts: u8,
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeConfigFileFingerprint {
    len: u64,
    modified: Option<std::time::SystemTime>,
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
#[derive(Debug, Default)]
struct NativeAppConfigCache {
    fingerprint: Option<NativeConfigFileFingerprint>,
    config: Option<AppConfig>,
}

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NativeAppKeyLinkRelayEventApply {
    Ignored,
    Current,
    RecordedRequest,
    AppliedRoster,
}

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
fn apply_native_app_key_link_relay_event_to_config(
    config: &mut AppConfig,
    event: &Event,
) -> Result<NativeAppKeyLinkRelayEventApply, String> {
    if iris_drive_core::nostr_events::is_app_key_link_request_event_coordinate(event) {
        let outcome =
            iris_drive_core::relay_sync::apply_remote_app_key_link_request_event(config, event)
                .map_err(|error| format!("applying app-key link request relay event: {error}"))?;
        return Ok(match outcome {
            iris_drive_core::relay_sync::AppKeyLinkRequestApply::Recorded => {
                NativeAppKeyLinkRelayEventApply::RecordedRequest
            }
            iris_drive_core::relay_sync::AppKeyLinkRequestApply::Current => {
                NativeAppKeyLinkRelayEventApply::Current
            }
            iris_drive_core::relay_sync::AppKeyLinkRequestApply::NotOurProfile
            | iris_drive_core::relay_sync::AppKeyLinkRequestApply::NotAdmin
            | iris_drive_core::relay_sync::AppKeyLinkRequestApply::InvalidInvite => {
                NativeAppKeyLinkRelayEventApply::Ignored
            }
        });
    }

    if event.kind.as_u16() == iris_drive_core::KIND_NOSTR_IDENTITY_ROSTER_OP {
        let outcome =
            iris_drive_core::relay_sync::apply_remote_nostr_identity_roster_op_event(config, event)
                .map_err(|error| format!("applying NostrIdentity roster relay event: {error}"))?;
        return Ok(match outcome {
            iris_drive_core::relay_sync::NostrIdentityRosterOpApply::Applied => {
                NativeAppKeyLinkRelayEventApply::AppliedRoster
            }
            iris_drive_core::relay_sync::NostrIdentityRosterOpApply::Current => {
                NativeAppKeyLinkRelayEventApply::Current
            }
            iris_drive_core::relay_sync::NostrIdentityRosterOpApply::NotOurProfile => {
                NativeAppKeyLinkRelayEventApply::Ignored
            }
        });
    }

    Ok(NativeAppKeyLinkRelayEventApply::Ignored)
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
impl NativeAppConfigCache {
    fn load(&mut self, config_dir: &Path) -> Result<AppConfig, String> {
        let config_path = config_path_in(config_dir);
        let fingerprint = native_config_file_fingerprint(&config_path)
            .map_err(|error| format!("reading config metadata: {error}"))?;
        if self.fingerprint.as_ref() == Some(&fingerprint)
            && let Some(config) = self.config.as_ref()
        {
            return Ok(config.clone());
        }

        let config = AppConfig::load_or_default(&config_path)
            .map_err(|error| format!("loading config: {error}"))?;
        self.fingerprint = Some(fingerprint);
        self.config = Some(config.clone());
        Ok(config)
    }
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
fn native_config_file_fingerprint(path: &Path) -> std::io::Result<NativeConfigFileFingerprint> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(NativeConfigFileFingerprint {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(NativeConfigFileFingerprint {
                len: 0,
                modified: None,
            })
        }
        Err(error) => Err(error),
    }
}

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
fn app_key_link_request_send_due(
    sent: Option<SentAppKeyLinkRequest>,
    now: std::time::Instant,
) -> bool {
    let Some(sent) = sent else {
        return true;
    };
    now.duration_since(sent.last_sent) >= app_key_link_request_retry_interval(sent.attempts)
}

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
fn app_key_link_request_retry_interval(attempts: u8) -> std::time::Duration {
    if attempts < APP_KEY_LINK_REQUEST_STARTUP_BURST_ATTEMPTS {
        std::time::Duration::from_millis(APP_KEY_LINK_REQUEST_STARTUP_RETRY_MILLIS)
    } else {
        std::time::Duration::from_secs(APP_KEY_LINK_REQUEST_RETRY_SECS)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeConfigFileFingerprint {
    len: u64,
    modified: Option<std::time::SystemTime>,
}

#[derive(Clone)]
struct RuntimeConfigCacheEntry {
    fingerprint: RuntimeConfigFileFingerprint,
    config: AppConfig,
}

static NATIVE_RUNTIME_CONFIG_CACHE: LazyLock<Mutex<BTreeMap<PathBuf, RuntimeConfigCacheEntry>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

pub(crate) fn load_native_runtime_config_cached(config_path: &Path) -> Result<AppConfig, String> {
    let fingerprint = runtime_config_file_fingerprint(config_path)
        .map_err(|error| format!("reading config metadata: {error}"))?;
    if let Ok(cache) = NATIVE_RUNTIME_CONFIG_CACHE.lock()
        && let Some(entry) = cache.get(config_path)
        && entry.fingerprint == fingerprint
    {
        return Ok(entry.config.clone());
    }

    let config = AppConfig::load_or_default(config_path)
        .map_err(|error| format!("loading config: {error}"))?;
    if let Ok(mut cache) = NATIVE_RUNTIME_CONFIG_CACHE.lock() {
        cache.insert(
            config_path.to_path_buf(),
            RuntimeConfigCacheEntry {
                fingerprint,
                config: config.clone(),
            },
        );
    }
    Ok(config)
}

fn runtime_config_file_fingerprint(path: &Path) -> std::io::Result<RuntimeConfigFileFingerprint> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(RuntimeConfigFileFingerprint {
            len: metadata.len(),
            modified: metadata.modified().ok(),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(RuntimeConfigFileFingerprint {
                len: 0,
                modified: None,
            })
        }
        Err(error) => Err(error),
    }
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
    #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
    app_key_link_exchange_running: Arc<AtomicBool>,
    #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
    app_key_link_exchange_stop: Arc<AtomicBool>,
    #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
    browser_gateway_running: Arc<AtomicBool>,
    #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
    browser_gateway_stop: Arc<AtomicBool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProviderSummaryMode {
    Skip,
    Refresh,
}

fn ui_sync_status(running: bool, status: &str) -> UiSyncStatus {
    UiSyncStatus {
        running,
        status: status.to_owned(),
        status_label: sync_status_label(status),
    }
}

fn ready_ui_sync_status() -> UiSyncStatus {
    ui_sync_status(false, "ready")
}

impl NativeAppRuntime {
    fn new(data_dir: String, app_version: String) -> Self {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut state = NativeAppState::default();
        state.ui.paths = paths_for(&data_dir);
        state.ui.sync = ready_ui_sync_status();

        let mut runtime = Self {
            state,
            data_dir,
            app_version,
            #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
            app_key_link_exchange_running: Arc::new(AtomicBool::new(false)),
            #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
            app_key_link_exchange_stop: Arc::new(AtomicBool::new(false)),
            #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
            browser_gateway_running: Arc::new(AtomicBool::new(false)),
            #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
            browser_gateway_stop: Arc::new(AtomicBool::new(false)),
        };
        runtime.reset_native_browser_gateway_status_for_new_process();
        runtime.reload_from_disk(ProviderSummaryMode::Skip);
        runtime.start_app_key_link_exchange_if_needed();
        runtime.start_browser_gateway_if_needed();
        runtime
    }

    fn state(&mut self) -> NativeAppState {
        let _ = (&self.data_dir, &self.app_version);
        self.state.clone()
    }

    #[allow(clippy::too_many_lines)]
    fn dispatch(&mut self, action: NativeAppAction) {
        self.state.error.clear();
        let provider_summary = match action {
            NativeAppAction::RefreshProfile => ProviderSummaryMode::Skip,
            _ => ProviderSummaryMode::Refresh,
        };
        match action {
            NativeAppAction::Refresh | NativeAppAction::RefreshProfile => {}
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
            NativeAppAction::AddRecoveryDevice { recovery_pubkey } => {
                self.add_recovery_device(&recovery_pubkey);
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
            NativeAppAction::SetLaunchOnStartup { enabled } => {
                self.set_launch_on_startup(enabled);
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
            share_action @ (NativeAppAction::CreateShare { .. }
            | NativeAppAction::DeleteShare { .. }
            | NativeAppAction::InviteShareMember { .. }
            | NativeAppAction::InviteShareMemberFromEvidence { .. }
            | NativeAppAction::RecordPendingShareInvite { .. }
            | NativeAppAction::AcceptShareInvite { .. }
            | NativeAppAction::RevokeShareMember { .. }
            | NativeAppAction::SetShareMemberRole { .. }
            | NativeAppAction::AddShareShortcut { .. }
            | NativeAppAction::RepairShareWraps { .. }) => self.dispatch_share_action(share_action),
            NativeAppAction::ExportShareRecipientEvidence { display_name } => {
                self.export_share_recipient_evidence(&display_name);
            }
            NativeAppAction::ImportFile {
                display_name,
                source_path,
            } => {
                self.import_file(&display_name, &source_path);
            }
            NativeAppAction::ImportContentLink { link } => {
                self.import_content_link(&link);
            }
        }
        self.reload_from_disk_preserving_error(provider_summary);
        self.start_app_key_link_exchange_if_needed();
        self.start_browser_gateway_if_needed();
    }

    fn dispatch_share_action(&mut self, action: NativeAppAction) {
        match self.try_dispatch_share_action(action) {
            Ok(result) => {
                if let Some(invite) = result.last_share_invite {
                    self.state.ui.last_share_invite = invite;
                }
            }
            Err(error) => {
                self.state.error = format!("running share action: {error:#}");
            }
        }
    }

    fn export_share_recipient_evidence(&mut self, display_name: &str) {
        self.state.ui.last_share_recipient_evidence.clear();
        match self.try_export_share_recipient_evidence(display_name) {
            Ok(evidence_json) => {
                self.state.ui.last_share_recipient_evidence = evidence_json;
            }
            Err(error) => {
                self.state.error = format!("exporting share recipient evidence: {error:#}");
            }
        }
    }

    fn try_export_share_recipient_evidence(&self, display_name: &str) -> anyhow::Result<String> {
        let config_dir = Path::new(&self.data_dir);
        let config = AppConfig::load_or_default(config_path_in(config_dir))?;
        let state = config
            .profile
            .context("profile is required before exporting recipient evidence")?;
        let profile = Profile::load(state, config_dir)?;
        let evidence = iris_drive_core::share_recipient_profile_evidence_for_app_key(
            profile.state.profile_id,
            &profile.state.profile_roster_ops,
            profile.app_key.keys(),
            optional_trimmed(display_name),
            share_now_seconds(),
        )
        .context("exporting share recipient evidence")?;
        serde_json::to_string(&evidence).context("encoding recipient evidence")
    }

    fn try_dispatch_share_action(
        &self,
        action: NativeAppAction,
    ) -> anyhow::Result<iris_drive_core::ShareActionResult> {
        let action = match action {
            NativeAppAction::CreateShare {
                source_path,
                display_name,
            } => iris_drive_core::ShareAction::CreateShare {
                source_path,
                display_name: optional_trimmed(&display_name),
            },
            NativeAppAction::DeleteShare { share_id } => {
                iris_drive_core::ShareAction::DeleteShare {
                    share_id: share_id.parse()?,
                }
            }
            NativeAppAction::InviteShareMember {
                share_id,
                profile_id,
                app_key,
                role,
                representative_npub_hint,
                display_name,
                label,
            } => iris_drive_core::ShareAction::InviteShareMember {
                share_id: share_id.parse()?,
                profile_id: profile_id.parse()?,
                app_key,
                role: parse_share_role(&role)?,
                representative_npub_hint: optional_trimmed(&representative_npub_hint),
                display_name: optional_trimmed(&display_name),
                label: optional_trimmed(&label),
            },
            NativeAppAction::InviteShareMemberFromEvidence {
                share_id,
                evidence_json,
                role,
                display_name,
            } => iris_drive_core::ShareAction::InviteShareMemberFromEvidence {
                share_id: share_id.parse()?,
                evidence_json,
                role: parse_share_role(&role)?,
                display_name: optional_trimmed(&display_name),
            },
            NativeAppAction::RecordPendingShareInvite {
                share_id,
                representative_npub_hint,
                role,
                display_name,
            } => iris_drive_core::ShareAction::RecordPendingShareInvite {
                share_id: share_id.parse()?,
                representative_npub_hint,
                role: parse_share_role(&role)?,
                display_name: optional_trimmed(&display_name),
            },
            NativeAppAction::AcceptShareInvite { invite } => {
                iris_drive_core::ShareAction::AcceptShareInvite { invite }
            }
            NativeAppAction::RevokeShareMember {
                share_id,
                profile_id,
                reason,
            } => iris_drive_core::ShareAction::RevokeShareMember {
                share_id: share_id.parse()?,
                profile_id: profile_id.parse()?,
                reason: optional_trimmed(&reason),
            },
            NativeAppAction::SetShareMemberRole {
                share_id,
                profile_id,
                role,
            } => iris_drive_core::ShareAction::SetShareMemberRole {
                share_id: share_id.parse()?,
                profile_id: profile_id.parse()?,
                role: parse_share_role(&role)?,
            },
            NativeAppAction::AddShareShortcut {
                share_id,
                path,
                parent,
                target_path,
            } => iris_drive_core::ShareAction::AddShareShortcut {
                share_id: share_id.parse()?,
                path: optional_trimmed(&path),
                parent: optional_trimmed(&parent),
                target_path: optional_trimmed(&target_path),
            },
            NativeAppAction::RepairShareWraps { share_id } => {
                iris_drive_core::ShareAction::RepairShareWraps {
                    share_id: share_id.parse()?,
                }
            }
            _ => unreachable!("non-share action dispatched to share action handler"),
        };
        iris_drive_core::dispatch_share_action(
            Path::new(&self.data_dir),
            action,
            share_now_seconds(),
        )
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
            self.state.error = format!("recovering device key: {error}");
            return;
        }
        config.profile = Some(account.state);
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
        } else {
            self.set_sync_running(true);
        }
    }

    fn add_recovery_device(&mut self, recovery_pubkey: &str) {
        let recovery_pubkey = match normalize_pubkey(recovery_pubkey) {
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
            "profile admin is required to add recovery keys".clone_into(&mut self.state.error);
            return;
        };
        if !state.can_admin_profile() {
            "profile admin is required to add recovery keys".clone_into(&mut self.state.error);
            return;
        }
        let mut account = match Profile::load(state, Path::new(&self.data_dir)) {
            Ok(account) => account,
            Err(error) => {
                self.state.error = format!("loading profile: {error}");
                return;
            }
        };
        if let Err(error) = account.add_recovery_pubkey(&recovery_pubkey) {
            self.state.error = format!("adding recovery key: {error}");
            return;
        }
        config.profile = Some(account.state);
        if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
            self.state.error = format!("saving config: {error}");
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
                self.state.error = format!("linking device key: {error}");
                return;
            }
        };
        if target.invite_pubkey.trim().is_empty() {
            "device invite is missing invite pubkey".clone_into(&mut self.state.error);
            return;
        }
        let requested_at = unix_now_seconds();
        let request_url = match encode_app_key_approval_request(
            account.app_key.keys(),
            account.state.profile_id,
            Some(&target.admin_app_key_hex),
            account.state.app_key_label.as_deref(),
            requested_at,
        ) {
            Ok(url) => url,
            Err(error) => {
                self.state.error = format!("building app-key link request: {error}");
                return;
            }
        };
        if let Err(error) = account.state.queue_outbound_app_key_link_request(
            target.admin_app_key_hex,
            &target.invite_pubkey,
            requested_at,
        ) {
            self.state.error = format!("queueing app-key link request: {error}");
            return;
        }
        if let Some(pending) = account.state.outbound_app_key_link_request.as_mut() {
            pending.request_url = request_url;
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
                self.stop_app_key_link_exchange();
                self.stop_browser_gateway();
                self.state.ui.roots.clear();
                self.state.ui.app_actors.clear();
                self.set_sync_ready();
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
            "profile admin is required to approve devices".clone_into(&mut self.state.error);
            return;
        };
        if !state.can_admin_profile() {
            "profile admin is required to approve devices".clone_into(&mut self.state.error);
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
            self.state.error = format!("approving device: {error}");
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
            "profile admin is required to reject devices".clone_into(&mut self.state.error);
            return;
        };
        if !state.can_admin_profile() {
            "profile admin is required to reject devices".clone_into(&mut self.state.error);
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
            self.state.error = format!("rejecting device: {error}");
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
            "profile admin is required to remove devices".clone_into(&mut self.state.error);
            return;
        };
        if state.app_key_pubkey == app_key_pubkey {
            "cannot remove this device from itself".clone_into(&mut self.state.error);
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
            self.state.error = format!("removing device: {error}");
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

    fn set_launch_on_startup(&mut self, enabled: bool) {
        let mut config = match self.load_config() {
            Ok(config) => config,
            Err(error) => {
                self.state.error = error;
                return;
            }
        };
        config.launch_on_startup = enabled;
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
        load_native_runtime_config_cached(&config_path_in(Path::new(&self.data_dir)))
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
    fn start_app_key_link_exchange_if_needed(&mut self) {
        #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
        {
            let Ok(config) = self.load_config() else {
                return;
            };
            if config.profile.is_none() {
                return;
            }
            if self
                .app_key_link_exchange_running
                .swap(true, Ordering::AcqRel)
            {
                return;
            }

            self.app_key_link_exchange_stop
                .store(false, Ordering::Release);
            let data_dir = self.data_dir.clone();
            let running = self.app_key_link_exchange_running.clone();
            let stop = self.app_key_link_exchange_stop.clone();
            std::thread::spawn(move || {
                if let Err(error) = run_app_key_link_exchange(&data_dir, stop) {
                    tracing::warn!(error = %error, "native app-key-link FIPS exchange stopped");
                }
                running.store(false, Ordering::Release);
            });
        }
    }

    #[allow(clippy::unused_self)]
    fn stop_app_key_link_exchange(&mut self) {
        #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
        {
            self.app_key_link_exchange_stop
                .store(true, Ordering::Release);
        }
    }

    #[allow(clippy::unused_self)]
    fn start_browser_gateway_if_needed(&mut self) {
        #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
        {
            let Ok(config) = self.load_config() else {
                write_native_browser_gateway_status(
                    Path::new(&self.data_dir),
                    json!({"running": false, "state": "error", "error": "loading config failed"}),
                );
                return;
            };
            let Some(mut profile) = config.profile.clone() else {
                write_native_browser_gateway_status(
                    Path::new(&self.data_dir),
                    json!({"running": false, "state": "not_configured"}),
                );
                return;
            };
            profile.recompute_authorization();
            match profile.authorization_state {
                AppKeyAuthorizationState::Authorized => {}
                AppKeyAuthorizationState::AwaitingApproval => {
                    write_native_browser_gateway_status(
                        Path::new(&self.data_dir),
                        json!({"running": false, "state": "awaiting_approval"}),
                    );
                    return;
                }
                AppKeyAuthorizationState::Revoked => {
                    write_native_browser_gateway_status(
                        Path::new(&self.data_dir),
                        json!({"running": false, "state": "revoked"}),
                    );
                    return;
                }
            }
            if !config.local_nhash_resolver_enabled {
                write_native_browser_gateway_status(
                    Path::new(&self.data_dir),
                    json!({"running": false, "state": "disabled_by_settings"}),
                );
                return;
            }
            if self.browser_gateway_running.swap(true, Ordering::AcqRel) {
                return;
            }

            write_native_browser_gateway_status(
                Path::new(&self.data_dir),
                json!({"running": false, "state": "starting"}),
            );
            self.browser_gateway_stop.store(false, Ordering::Release);
            let data_dir = self.data_dir.clone();
            let running = self.browser_gateway_running.clone();
            let stop = self.browser_gateway_stop.clone();
            std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    run_native_browser_gateway(&data_dir, stop)
                }));
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(error)) => {
                        write_native_browser_gateway_status(
                            Path::new(&data_dir),
                            json!({"running": false, "state": "error", "error": error}),
                        );
                        tracing::warn!(error = %error, "native browser gateway stopped");
                    }
                    Err(payload) => {
                        let panic = if let Some(value) = payload.downcast_ref::<&str>() {
                            (*value).to_owned()
                        } else if let Some(value) = payload.downcast_ref::<String>() {
                            value.clone()
                        } else {
                            "non-string panic payload".to_owned()
                        };
                        write_native_browser_gateway_status(
                            Path::new(&data_dir),
                            json!({"running": false, "state": "panic", "error": panic}),
                        );
                        tracing::warn!(panic = %panic, "native browser gateway panicked");
                    }
                }
                running.store(false, Ordering::Release);
            });
        }
    }

    #[allow(clippy::unused_self)]
    fn reset_native_browser_gateway_status_for_new_process(&mut self) {
        #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
        write_native_browser_gateway_status(
            Path::new(&self.data_dir),
            json!({"running": false, "state": "new_process"}),
        );
    }

    #[allow(clippy::unused_self)]
    fn stop_browser_gateway(&mut self) {
        #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
        {
            self.browser_gateway_stop.store(true, Ordering::Release);
            write_native_browser_gateway_status(
                Path::new(&self.data_dir),
                json!({"running": false, "state": "stopping"}),
            );
        }
    }

    fn reload_from_disk_preserving_error(&mut self, provider_summary: ProviderSummaryMode) {
        let error = self.state.error.clone();
        let last_share_invite = self.state.ui.last_share_invite.clone();
        let last_share_recipient_evidence = self.state.ui.last_share_recipient_evidence.clone();
        self.reload_from_disk(provider_summary);
        self.state.error = error;
        self.state.ui.last_share_invite = last_share_invite;
        self.state.ui.last_share_recipient_evidence = last_share_recipient_evidence;
    }

    #[allow(clippy::too_many_lines)]
    fn reload_from_disk(&mut self, provider_summary: ProviderSummaryMode) {
        let paths = paths_for(&self.data_dir);
        let sync = self.state.ui.sync.clone();
        let previous_roots = self.state.ui.roots.clone();
        self.reset_ui_for_reload(paths, sync);

        let Ok(mut config) = self.load_config() else {
            self.set_sync_running(false);
            self.refresh_ui_summary(None);
            return;
        };
        match iris_drive_core::repair_missing_share_shortcuts(&mut config) {
            Ok(true) => {
                if let Err(error) = config.save(config_path_in(Path::new(&self.data_dir))) {
                    self.state.error = format!("saving repaired share shortcuts: {error}");
                }
            }
            Ok(false) => {}
            Err(error) => {
                self.state.error = format!("repairing share shortcuts: {error:#}");
            }
        }
        if let Err(error) =
            ensure_cached_app_key_link_request_url(&mut config, Path::new(&self.data_dir))
        {
            self.state.error = format!("saving app-key link request: {error}");
        }
        self.state.ui.local_nhash_resolver_enabled = config.local_nhash_resolver_enabled;
        self.state.ui.launch_on_startup = config.launch_on_startup;
        self.state.ui.sites_portal_url.clear();
        self.state.ui.caldav_url.clear();
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
            self.set_sync_ready();
            self.refresh_ui_summary(None);
            return;
        };
        let account = raw_account.clone();
        if account.authorization_state == AppKeyAuthorizationState::Revoked {
            self.logout_current_revoked_device(provider_summary);
            return;
        }
        let gateway_port = if config.local_nhash_resolver_enabled && account.is_authorized() {
            native_browser_gateway_port_for_state(Path::new(&self.data_dir))
        } else {
            None
        };
        self.state.ui.sites_portal_url = gateway_port
            .map(iris_drive_core::gateway::local_portal_url)
            .unwrap_or_default();
        self.state.ui.caldav_url = gateway_port
            .map(|port| {
                iris_drive_core::gateway::local_caldav_url_for_identity(
                    port,
                    &pubkey_npub(&account.app_key_pubkey),
                )
            })
            .unwrap_or_default();
        let profile = nostr_identity_summary(&account);
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
            app_key_link_request: app_key_link_request_url(&account, Path::new(&self.data_dir)),
            app_key_link_invite: app_key_link_invite_url(&account),
            inbound_app_key_link_requests: inbound_app_key_link_requests(&account),
        });
        self.state.ui.shares = ui_shares_for_config(&config, &account.app_key_pubkey);
        let ui_fips_status = ui_fips_status_for_config_dir(Path::new(&self.data_dir));
        self.state.ui.app_actors = app_actors_from_account(&account, &ui_fips_status);
        update_snapshot_link(&mut self.state, &config);
        if provider_summary == ProviderSummaryMode::Refresh {
            self.refresh_provider_summary();
        }
        self.refresh_ui_summary(Some(ui_fips_status));
    }

    fn logout_current_revoked_device(&mut self, provider_summary: ProviderSummaryMode) {
        self.logout();
        if self.state.error.is_empty() {
            self.reload_from_disk(provider_summary);
        }
    }

    fn reset_ui_for_reload(&mut self, paths: UiPaths, sync: UiSyncStatus) {
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
            local_nhash_resolver_enabled: true,
            launch_on_startup: true,
            sites_portal_url: String::new(),
            caldav_url: String::new(),
            ..UiState::default()
        };
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
        self.state.ui.online_app_key_count = self
            .state
            .ui
            .app_actors
            .iter()
            .filter(|app_actor| app_actor.actor_kind == "device" && app_actor.is_online)
            .count() as u64;
        self.state.ui.authorized_app_key_count = self
            .state
            .ui
            .app_actors
            .iter()
            .filter(|app_actor| app_actor.actor_kind == "device")
            .count() as u64;
        self.state.ui.fips = fips_status.unwrap_or_else(paused_ui_fips_status);
    }

    fn set_sync_running(&mut self, running: bool) {
        self.set_sync_status(running, if running { "running" } else { "paused" });
        if running {
            self.start_browser_gateway_if_needed();
        }
    }

    fn set_sync_ready(&mut self) {
        self.state.ui.sync = ready_ui_sync_status();
    }

    fn set_sync_status(&mut self, running: bool, status: &str) {
        self.state.ui.sync = ui_sync_status(running, status);
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

    fn import_content_link(&mut self, link: &str) {
        if !self.initialized() {
            "profile is required before importing files".clone_into(&mut self.state.error);
            return;
        }
        if link.trim().is_empty() {
            "content link is required".clone_into(&mut self.state.error);
            return;
        }
        if let Err(error) = native_provider_import_content_link(&self.data_dir, link) {
            self.state.error = format!("importing content link: {error:#}");
        }
    }
}

#[cfg(not(test))]
impl Drop for NativeAppRuntime {
    fn drop(&mut self) {
        #[cfg(any(target_os = "ios", target_os = "android"))]
        self.app_key_link_exchange_stop
            .store(true, Ordering::Release);
        #[cfg(any(target_os = "ios", target_os = "android"))]
        self.browser_gateway_stop.store(true, Ordering::Release);
    }
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
fn run_app_key_link_exchange(data_dir: &str, stop: Arc<AtomicBool>) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| format!("building app-key-link exchange runtime: {error}"))?;
    let result = runtime.block_on(run_app_key_link_exchange_async(data_dir, stop));
    if let Err(error) = &result {
        write_native_fips_error(Path::new(data_dir), error);
    }
    result
}

#[allow(clippy::unnecessary_wraps)]
fn native_browser_gateway_port_for_state(config_dir: &Path) -> Option<u16> {
    #[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
    {
        native_browser_gateway_status_port(config_dir)
    }
    #[cfg(not(all(not(test), any(target_os = "ios", target_os = "android"))))]
    {
        let _ = config_dir;
        Some(iris_drive_core::gateway::DEFAULT_GATEWAY_PORT)
    }
}

fn run_native_calendar_export(
    data_dir: &str,
) -> anyhow::Result<iris_drive_core::calendar::CalendarData> {
    let config_dir = Path::new(data_dir);
    let config = load_native_runtime_config_cached(&config_path_in(config_dir))
        .map_err(anyhow::Error::msg)?;
    let owner_npub = config.profile.as_ref().map_or_else(
        || "iris-android".to_owned(),
        |profile| pubkey_npub(&profile.app_key_pubkey),
    );
    let daemon = iris_drive_core::Daemon::open_with_config(config_dir, config)
        .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
    block_on_backup_operation(async {
        iris_drive_core::calendar::load_calendar_data(daemon.tree(), daemon.config(), &owner_npub)
            .await
            .context("loading Iris calendar")
    })
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
fn run_native_browser_gateway(data_dir: &str, stop: Arc<AtomicBool>) -> Result<(), String> {
    let data_dir = PathBuf::from(data_dir);
    write_native_browser_gateway_status(
        &data_dir,
        json!({"running": false, "state": "loading_config"}),
    );
    let config = AppConfig::load_or_default(config_path_in(&data_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    let Some(mut profile) = config.profile.clone() else {
        write_native_browser_gateway_status(
            &data_dir,
            json!({"running": false, "state": "disabled"}),
        );
        return Ok(());
    };
    profile.recompute_authorization();
    if !profile.is_authorized() || !config.local_nhash_resolver_enabled {
        write_native_browser_gateway_status(
            &data_dir,
            json!({"running": false, "state": "disabled"}),
        );
        return Ok(());
    }
    write_native_browser_gateway_status(
        &data_dir,
        json!({"running": false, "state": "starting_embedded_hashtree"}),
    );
    let embedded_hashtree = EmbeddedHashtreeHost::start(&data_dir, &config)
        .map_err(|error| format!("starting embedded hashtree: {error:#}"))?;
    let hashtree_base_url = embedded_hashtree.status().base_url.clone();
    write_native_browser_gateway_status(
        &data_dir,
        json!({
            "running": false,
            "state": "starting_gateway",
            "hashtree_base_url": hashtree_base_url,
        }),
    );
    let daemon =
        Daemon::open(&data_dir).map_err(|error| format!("opening daemon for gateway: {error}"))?;
    let tree = daemon.tree_handle();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .map_err(|error| format!("building browser gateway runtime: {error}"))?;
    let gateway_data_dir = data_dir.clone();
    let gateway_hashtree_base_url = hashtree_base_url.clone();
    let result = runtime.block_on(async move {
        let gateway = GatewayServer::bind_with_tree_and_htree_daemon(
            &gateway_data_dir,
            tree,
            gateway_hashtree_base_url.clone(),
            GatewayBind::loopback_v4(0),
        )
        .await
        .map_err(|error| format!("starting browser gateway: {error}"))?;
        let proxy = GatewayProxyServer::bind_for_gateway(gateway.local_addr())
            .await
            .map_err(|error| format!("starting browser gateway proxy: {error}"))?;
        let gateway_port = gateway.local_addr().port();
        let proxy_port = proxy.local_addr().port();
        write_native_browser_gateway_status(
            &gateway_data_dir,
            json!({
                "running": true,
                "state": "running",
                "bind": gateway.local_addr().to_string(),
                "hashtree_base_url": gateway_hashtree_base_url,
                "portal_url": iris_drive_core::gateway::local_portal_url(gateway_port),
                "caldav_url": native_caldav_url_for_config_dir(&gateway_data_dir, gateway_port),
                "proxy_bind": proxy.local_addr().to_string(),
                "proxy_port": proxy_port,
                "proxy_url": format!("http://127.0.0.1:{proxy_port}/"),
            }),
        );

        while !stop.load(Ordering::Acquire) {
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
        proxy
            .shutdown()
            .await
            .map_err(|error| format!("stopping browser gateway proxy: {error}"))?;
        gateway
            .shutdown()
            .await
            .map_err(|error| format!("stopping browser gateway: {error}"))?;
        write_native_browser_gateway_status(
            &gateway_data_dir,
            json!({"running": false, "state": "stopped"}),
        );
        Ok(())
    });
    drop(runtime);
    drop(embedded_hashtree);
    result
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
fn native_caldav_url_for_config_dir(config_dir: &Path, port: u16) -> String {
    AppConfig::load_or_default_cached_profile(config_path_in(config_dir))
        .ok()
        .and_then(|config| {
            config
                .profile
                .map(|profile| pubkey_npub(&profile.app_key_pubkey))
        })
        .map_or_else(
            || iris_drive_core::gateway::local_caldav_url(port),
            |identity| iris_drive_core::gateway::local_caldav_url_for_identity(port, &identity),
        )
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
fn native_browser_gateway_status_port(config_dir: &Path) -> Option<u16> {
    let value: Value = serde_json::from_slice(
        &std::fs::read(native_browser_gateway_status_path(config_dir)).ok()?,
    )
    .ok()?;
    native_browser_gateway_status_value_port(&value)
}

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
fn native_browser_gateway_status_value_port(value: &Value) -> Option<u16> {
    if !value.get("running")?.as_bool()? {
        return None;
    }
    value
        .get("bind")?
        .as_str()?
        .rsplit_once(':')?
        .1
        .parse()
        .ok()
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
fn native_browser_gateway_status_path(config_dir: &Path) -> PathBuf {
    config_dir.join("native-browser-gateway-status.json")
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
fn write_native_browser_gateway_status(config_dir: &Path, value: Value) {
    let path = native_browser_gateway_status_path(config_dir);
    let _ = std::fs::create_dir_all(config_dir);
    if let Err(error) = std::fs::write(
        &path,
        serde_json::to_vec_pretty(&value).unwrap_or_else(|_| b"{}".to_vec()),
    ) {
        tracing::warn!(
            error = %error,
            path = %path.display(),
            "writing native browser gateway status failed"
        );
    }
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
#[allow(clippy::too_many_lines)]
async fn run_app_key_link_exchange_async(
    data_dir: &str,
    stop: Arc<AtomicBool>,
) -> Result<(), String> {
    let config_dir = Path::new(data_dir);
    let startup_config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    let Some(account_state) = startup_config.profile.as_ref() else {
        return Ok(());
    };

    let device = iris_drive_core::AppKey::load(key_path_in(config_dir))
        .map_err(|error| format!("loading app key: {error}"))?;
    let relays = if startup_config.relays.is_empty() {
        default_relays()
    } else {
        normalized_config_relays(&startup_config.relays)
            .map_err(|error| format!("normalizing relays: {error}"))?
    };
    let root_scope_id = account_state.root_scope_id();
    let relay_filters = iris_drive_core::relay_sync::subscription_filters(
        &account_state.app_key_pubkey,
        &root_scope_id,
        iris_drive_core::PRIMARY_DRIVE_ID,
    );
    let relay_client = iris_drive_core::relay_sync::connect(&relays)
        .await
        .map_err(|error| format!("connecting app-key-link relays: {error}"))?;
    for relay_filter in relay_filters {
        relay_client
            .subscribe(relay_filter, None)
            .await
            .map_err(|error| format!("subscribing app-key-link relays: {error}"))?;
    }
    let mut relay_notifications = relay_client.notifications();
    let daemon = iris_drive_core::Daemon::open(config_dir)
        .map_err(|error| format!("opening block store: {error}"))?;
    let local = daemon.tree().get_store().clone();
    let sync = iris_drive_core::FipsBlockSync::start(&device, local, &startup_config)
        .await
        .map_err(|error| format!("starting FIPS app-key-link exchange: {error}"))?;
    if let Err(error) = write_native_fips_status(config_dir, &sync, None).await {
        tracing::warn!(error = %error, "writing native FIPS status failed");
    }
    let mut app_messages = sync.subscribe_app_messages();
    let mut sent_requests = BTreeMap::new();
    let mut sent_rosters = BTreeMap::new();
    let mut acked_rosters = BTreeSet::new();
    let mut app_key_link_config_cache = NativeAppConfigCache::default();
    let mut direct_roots = iris_drive_core::DirectRootExchange::default();
    let mut app_key_link_tick = tokio::time::interval(std::time::Duration::from_millis(
        APP_KEY_LINK_EXCHANGE_TICK_MILLIS,
    ));
    app_key_link_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let direct_root_period = std::time::Duration::from_millis(NATIVE_DIRECT_ROOT_EXCHANGE_MILLIS);
    let mut direct_root_tick = tokio::time::interval_at(
        tokio::time::Instant::now() + direct_root_period,
        direct_root_period,
    );
    direct_root_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let _ = drive_app_key_link_exchange_tick(
        config_dir,
        &relay_client,
        device.keys(),
        &sync,
        &mut sent_requests,
        &mut sent_rosters,
        &acked_rosters,
        &mut app_key_link_config_cache,
    )
    .await?;
    if let Err(error) = direct_roots.announce_current_state(config_dir, &sync).await {
        tracing::warn!(error = %error, "native direct-root FIPS exchange failed");
    }
    if let Err(error) = direct_roots.drain_mesh_events(config_dir, &sync).await {
        tracing::warn!(error = %error, "native direct-root FIPS mesh drain failed");
    }

    while !stop.load(Ordering::Acquire) {
        tokio::select! {
            _ = app_key_link_tick.tick() => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let _ = drive_app_key_link_exchange_tick(
                    config_dir,
                    &relay_client,
                    device.keys(),
                    &sync,
                    &mut sent_requests,
                    &mut sent_rosters,
                    &acked_rosters,
                    &mut app_key_link_config_cache,
                ).await?;
            }
            _ = direct_root_tick.tick() => {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                if let Err(error) = direct_roots.announce_current_state(config_dir, &sync).await {
                    tracing::warn!(error = %error, "native direct-root FIPS exchange failed");
                }
            }
            message = sync.recv_mesh_pubsub_event() => {
                let mut messages = vec![message];
                messages.extend(sync.drain_mesh_pubsub_events().await);
                let received_messages = messages.len();
                let (messages, skipped_roots) =
                    iris_drive_core::coalesce_direct_root_mesh_events(messages);
                if skipped_roots > 0 {
                    tracing::debug!(
                        received_messages,
                        applied_messages = messages.len(),
                        skipped_roots,
                        "coalesced native direct-root FIPS mesh events"
                    );
                }
                for message in messages {
                    if let Err(error) = direct_roots.handle_mesh_event(config_dir, &sync, message).await {
                        tracing::warn!(error = %error, "native direct-root FIPS mesh event failed");
                        continue;
                    }
                }
            }
            message = app_messages.recv() => {
                match message {
                    Ok(message) => {
                        let mut messages = vec![message];
                        while messages.len() < NATIVE_APP_MESSAGE_DRAIN_LIMIT {
                            match app_messages.try_recv() {
                                Ok(message) => messages.push(message),
                                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
                                Err(tokio::sync::broadcast::error::TryRecvError::Lagged(skipped)) => {
                                    tracing::warn!(skipped, "native app-key-link FIPS receiver lagged");
                                }
                                Err(tokio::sync::broadcast::error::TryRecvError::Closed) => break,
                            }
                        }
                        let received_messages = messages.len();
                        let (messages, skipped_roots) =
                            iris_drive_core::coalesce_direct_root_app_messages(messages);
                        if skipped_roots > 0 {
                            tracing::debug!(
                                received_messages,
                                applied_messages = messages.len(),
                                skipped_roots,
                                "coalesced native direct-root FIPS messages"
                            );
                        }
                        for message in messages {
                            if let Err(error) = handle_native_app_key_link_app_message(
                                config_dir,
                                &sync,
                                &message,
                                &mut acked_rosters,
                            ).await {
                                tracing::warn!(error = %error, topic = message.topic, "handling native app-key-link FIPS message failed");
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
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "native app-key-link FIPS receiver lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
            notification = relay_notifications.recv() => {
                match notification {
                    Ok(nostr_sdk::RelayPoolNotification::Event { event, .. }) => {
                        if let Err(error) = handle_native_app_key_link_relay_event(config_dir, &event) {
                            tracing::warn!(error = %error, event_id = %event.id.to_hex(), "handling native app-key-link relay event failed");
                        }
                    }
                    Ok(nostr_sdk::RelayPoolNotification::Shutdown) => {
                        tracing::warn!("native app-key-link relay notifications shut down");
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "native app-key-link relay receiver lagged");
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
    iris_drive_core::relay_sync::shutdown_client(&relay_client).await;
    Ok(())
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
async fn drive_app_key_link_exchange_tick(
    config_dir: &Path,
    relay_client: &nostr_sdk::Client,
    device_keys: &nostr_sdk::Keys,
    sync: &iris_drive_core::FsFipsBlockSync,
    sent_requests: &mut BTreeMap<String, SentAppKeyLinkRequest>,
    sent_rosters: &mut BTreeMap<String, std::time::Instant>,
    acked_rosters: &BTreeSet<String>,
    config_cache: &mut NativeAppConfigCache,
) -> Result<bool, String> {
    let config = config_cache.load(config_dir)?;
    let Some(state) = config.profile.as_ref() else {
        return Ok(false);
    };

    sync.refresh_authorized_peers(&config).await;
    send_native_pending_app_key_link_request(relay_client, device_keys, sync, state, sent_requests)
        .await?;
    send_native_authorized_app_key_link_rosters(
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

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
async fn send_native_pending_app_key_link_request(
    relay_client: &nostr_sdk::Client,
    device_keys: &nostr_sdk::Keys,
    sync: &iris_drive_core::FsFipsBlockSync,
    state: &iris_drive_core::ProfileState,
    sent_requests: &mut BTreeMap<String, SentAppKeyLinkRequest>,
) -> Result<(), String> {
    let Some(frame) = pending_app_key_link_request_frame(state, device_keys)
        .map_err(|error| format!("building app-key link request frame: {error}"))?
    else {
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
        .map_err(|error| format!("encoding app-key link request: {error}"))?;
    let relay_event_id = iris_drive_core::relay_sync::publish_app_key_link_request(
        relay_client,
        device_keys,
        &frame,
    )
    .await
    .map_err(|error| format!("publishing app-key link request relay event: {error}"))?;
    let attempts = sent_requests
        .get(&fingerprint)
        .map_or(1, |sent| sent.attempts.saturating_add(1));
    sent_requests.insert(
        fingerprint,
        SentAppKeyLinkRequest {
            last_sent: now,
            attempts,
        },
    );
    match sync
        .send_app_message(&admin_npub, APP_KEY_LINK_REQUEST_APP_TOPIC, bytes)
        .await
    {
        Ok(()) => {
            tracing::debug!(
                admin_npub,
                relay_event_id = %relay_event_id.to_hex(),
                requested_at = frame.requested_at,
                "sent native app-key-link request over relay and FIPS"
            );
        }
        Err(error) => tracing::warn!(
            admin_npub,
            relay_event_id = %relay_event_id.to_hex(),
            error = %error,
            "sent native app-key-link request over relay, FIPS send failed"
        ),
    }
    Ok(())
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
async fn send_native_authorized_app_key_link_rosters(
    _config_dir: &Path,
    sync: &iris_drive_core::FsFipsBlockSync,
    state: &iris_drive_core::ProfileState,
    sent_rosters: &mut BTreeMap<String, std::time::Instant>,
    acked_rosters: &BTreeSet<String>,
) -> Result<(), String> {
    if !state.can_admin_profile() {
        return Ok(());
    }
    let Some(app_keys) = state.current_app_keys_projection() else {
        return Ok(());
    };
    if !app_keys.contains(&state.app_key_pubkey) {
        return Ok(());
    }

    let now = std::time::Instant::now();
    let due_devices = app_key_link_roster_recipients(state)
        .into_iter()
        .filter(|recipient| {
            if acked_rosters.contains(&recipient.roster_fingerprint) {
                return false;
            }
            !sent_rosters
                .get(&recipient.roster_fingerprint)
                .is_some_and(|last_sent| {
                    now.duration_since(*last_sent)
                        < std::time::Duration::from_secs(APP_KEY_LINK_ROSTER_RETRY_SECS)
                })
        })
        .collect::<Vec<_>>();
    if due_devices.is_empty() {
        return Ok(());
    }

    let Some(frame) = app_key_link_roster_frame(state, unix_now_seconds()) else {
        return Ok(());
    };
    let bytes = serde_json::to_vec(&frame)
        .map_err(|error| format!("encoding app-key link roster: {error}"))?;
    for recipient in due_devices {
        let recipient_npub = pubkey_npub(&recipient.app_key_pubkey);
        match sync
            .send_app_message(
                &recipient_npub,
                APP_KEY_LINK_ROSTER_APP_TOPIC,
                bytes.clone(),
            )
            .await
        {
            Ok(()) => {
                sent_rosters.insert(recipient.roster_fingerprint, now);
                tracing::debug!(
                    recipient_npub,
                    dck_generation = app_keys.dck_generation,
                    "sent native app-key-link roster over FIPS"
                );
            }
            Err(error) => tracing::warn!(
                recipient_npub,
                error = %error,
                "sending native app-key-link roster over FIPS failed"
            ),
        }
    }
    Ok(())
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
async fn handle_native_app_key_link_app_message(
    config_dir: &Path,
    sync: &iris_drive_core::FsFipsBlockSync,
    message: &iris_drive_core::FipsAppMessage,
    acked_rosters: &mut BTreeSet<String>,
) -> Result<bool, String> {
    match message.topic.as_str() {
        APP_KEY_LINK_REQUEST_APP_TOPIC => handle_native_app_key_link_request(config_dir, message),
        APP_KEY_LINK_ROSTER_APP_TOPIC => {
            handle_native_app_key_link_roster(config_dir, sync, message).await
        }
        APP_KEY_LINK_ROSTER_ACK_APP_TOPIC => {
            handle_native_app_key_link_roster_ack(config_dir, message, acked_rosters)
        }
        _ => Ok(false),
    }
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
fn handle_native_app_key_link_request(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
) -> Result<bool, String> {
    let frame: AppKeyLinkRequestFrame = serde_json::from_slice(&message.data)
        .map_err(|error| format!("parsing app-key link request frame: {error}"))?;
    if frame.schema != 1 {
        return Err(format!(
            "unsupported app-key link request schema {}",
            frame.schema
        ));
    }
    let app_key_hex = normalize_pubkey(&frame.app_key_pubkey)?;
    let invite_pubkey = if frame.invite_pubkey.trim().is_empty() {
        app_key_approval_invite_pubkey(&frame.url)
    } else {
        frame.invite_pubkey
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
            &invite_pubkey,
            Some(frame.url),
            frame.requested_at,
        )
        .map_err(|error| format!("recording inbound app-key link request: {error}"))?;
    if changed {
        config
            .save(config_path_in(config_dir))
            .map_err(|error| format!("saving config: {error}"))?;
        tracing::debug!(
            peer = message.peer_id,
            app_key_npub = pubkey_npub(&app_key_hex),
            requested_at = frame.requested_at,
            "received native app-key-link request over FIPS"
        );
    }
    Ok(true)
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
fn handle_native_app_key_link_relay_event(
    config_dir: &Path,
    event: &nostr_sdk::Event,
) -> Result<bool, String> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))
        .map_err(|error| format!("loading config: {error}"))?;
    let outcome = apply_native_app_key_link_relay_event_to_config(&mut config, event)?;
    match outcome {
        NativeAppKeyLinkRelayEventApply::RecordedRequest => {
            config
                .save(config_path_in(config_dir))
                .map_err(|error| format!("saving config: {error}"))?;
            tracing::debug!(
                event_id = %event.id.to_hex(),
                app_key_npub = pubkey_npub(&event.pubkey.to_hex()),
                "received native app-key-link request over relay"
            );
            Ok(true)
        }
        NativeAppKeyLinkRelayEventApply::AppliedRoster => {
            config
                .save(config_path_in(config_dir))
                .map_err(|error| format!("saving config: {error}"))?;
            tracing::debug!(
                event_id = %event.id.to_hex(),
                app_key_npub = pubkey_npub(&event.pubkey.to_hex()),
                "applied native app-key-link roster op over relay"
            );
            Ok(true)
        }
        NativeAppKeyLinkRelayEventApply::Current => Ok(true),
        NativeAppKeyLinkRelayEventApply::Ignored => Ok(false),
    }
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
#[allow(clippy::too_many_lines)]
async fn handle_native_app_key_link_roster(
    config_dir: &Path,
    sync: &iris_drive_core::FsFipsBlockSync,
    message: &iris_drive_core::FipsAppMessage,
) -> Result<bool, String> {
    let frame: AppKeyLinkRosterFrame = serde_json::from_slice(&message.data)
        .map_err(|error| format!("parsing app-key link roster frame: {error}"))?;
    if frame.schema != 1 {
        return Err(format!(
            "unsupported app-key link roster schema {}",
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

    let outcome = iris_drive_core::relay_sync::apply_app_key_link_roster_frame(
        &mut config,
        &frame,
        &admin_app_key_hex,
    )
    .map_err(|error| format!("applying signed app-key-link profile roster ops: {error}"))?;
    let accepted = match outcome {
        iris_drive_core::relay_sync::AppKeyLinkRosterApply::Current => true,
        iris_drive_core::relay_sync::AppKeyLinkRosterApply::Applied(decision) => {
            decision != iris_drive_core::ApplyDecision::Rejected
        }
        iris_drive_core::relay_sync::AppKeyLinkRosterApply::Ignored => false,
    };
    let changed = matches!(
        outcome,
        iris_drive_core::relay_sync::AppKeyLinkRosterApply::Applied(decision)
            if decision != iris_drive_core::ApplyDecision::Rejected
    );
    let state = config
        .profile
        .as_ref()
        .ok_or_else(|| "profile disappeared while applying app-key-link roster".to_string())?;
    let ack_frame = if accepted {
        app_key_link_roster_ack_frame(state, &admin_app_key_hex, unix_now_seconds())
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
            "accepted native app-key-link roster over FIPS"
        );
    }
    if let Some(frame) = ack_frame {
        send_native_app_key_link_roster_ack(sync, &frame).await?;
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
                "synced drive roots after native app-key-link roster"
            ),
            Err(error) => tracing::warn!(
                error = %error,
                "syncing drive roots after native app-key-link roster failed"
            ),
        }
    }
    Ok(true)
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
fn handle_native_app_key_link_roster_ack(
    config_dir: &Path,
    message: &iris_drive_core::FipsAppMessage,
    acked_rosters: &mut BTreeSet<String>,
) -> Result<bool, String> {
    let frame: AppKeyLinkRosterAckFrame = serde_json::from_slice(&message.data)
        .map_err(|error| format!("parsing app-key link roster ack frame: {error}"))?;
    if frame.schema != 1 {
        return Err(format!(
            "unsupported app-key link roster ack schema {}",
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
        || !app_key_link_roster_ack_matches_state(state, &frame)
    {
        return Ok(true);
    }

    acked_rosters.insert(frame.roster_fingerprint);
    Ok(true)
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
async fn send_native_app_key_link_roster_ack(
    sync: &iris_drive_core::FsFipsBlockSync,
    frame: &AppKeyLinkRosterAckFrame,
) -> Result<(), String> {
    sync.send_app_message(
        &pubkey_npub(&frame.admin_app_key_pubkey),
        APP_KEY_LINK_ROSTER_ACK_APP_TOPIC,
        serde_json::to_vec(frame)
            .map_err(|error| format!("encoding app-key-link roster ack: {error}"))?,
    )
    .await
    .map_err(|error| format!("sending app-key-link roster ack over FIPS: {error}"))?;
    Ok(())
}

fn ui_fips_status_for_config_dir(config_dir: &Path) -> UiFipsStatus {
    #[cfg(not(any(target_os = "ios", target_os = "android")))]
    remove_legacy_native_fips_status(config_dir);
    if let Some(status) = load_daemon_ui_fips_status(config_dir) {
        return status;
    }
    ui_native_fips_status_for_current_target(config_dir)
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn remove_legacy_native_fips_status(config_dir: &Path) {
    let path = config_dir.join(NATIVE_FIPS_STATUS_FILE_NAME);
    match std::fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            tracing::warn!(error = %error, path = %path.display(), "removing legacy native FIPS status failed");
        }
    }
}

#[cfg(any(target_os = "ios", target_os = "android"))]
fn ui_native_fips_status_for_current_target(config_dir: &Path) -> UiFipsStatus {
    ui_fips_status_for_native_config_dir(config_dir)
}

#[cfg(not(any(target_os = "ios", target_os = "android")))]
fn ui_native_fips_status_for_current_target(_config_dir: &Path) -> UiFipsStatus {
    paused_ui_fips_status()
}

#[cfg(any(test, target_os = "ios", target_os = "android"))]
fn ui_fips_status_for_native_config_dir(config_dir: &Path) -> UiFipsStatus {
    let native_status = load_native_fips_status(config_dir);
    ui_fips_status(native_status.as_ref())
}

#[cfg(any(test, target_os = "ios", target_os = "android"))]
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
    ui_fips_status_from_normalized(&normalized)
}

fn ui_fips_status_from_normalized(normalized: &Value) -> UiFipsStatus {
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
        roster_peer_count: normalized_u64(normalized, "roster_peer_count"),
        roster_online_device_count: normalized_u64(normalized, "roster_online_device_count"),
        roster_direct_device_count: normalized_u64(normalized, "roster_direct_device_count"),
        online_device_count: normalized_u64(normalized, "online_device_count"),
        direct_device_count: normalized_u64(normalized, "direct_device_count"),
        mesh_device_count: normalized_u64(normalized, "mesh_device_count"),
        other_peer_count: normalized_u64(normalized, "other_peer_count"),
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

fn daemon_status_path(config_dir: &Path) -> PathBuf {
    config_dir.join(DAEMON_STATUS_FILE_NAME)
}

fn load_daemon_ui_fips_status(config_dir: &Path) -> Option<UiFipsStatus> {
    let data = std::fs::read(daemon_status_path(config_dir)).ok()?;
    let status: Value = serde_json::from_slice(&data).ok()?;
    let fips_status = status
        .get("fips_block_sync")
        .filter(|value| value.is_object())
        .or_else(|| status.get("fips").filter(|value| value.is_object()))?;
    let running = status
        .get("running")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fresh_flag = status.get("fresh").and_then(Value::as_bool).unwrap_or(true);
    let updated_at = status
        .get("updated_at")
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let fresh = running
        && fresh_flag
        && unix_now_seconds().saturating_sub(updated_at) <= DAEMON_STATUS_FRESH_SECS;
    let error = status
        .get("fips_block_sync_error")
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| {
            status
                .get("fips")
                .and_then(|fips| fips.get("error"))
                .cloned()
        })
        .unwrap_or(Value::Null);
    let normalized = normalize_fips_status_value(Some(fips_status), running, fresh, error, &[]);
    Some(ui_fips_status_from_normalized(&normalized))
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

#[cfg(any(test, target_os = "ios", target_os = "android"))]
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

#[cfg(any(test, target_os = "ios", target_os = "android"))]
fn native_fips_status_path(config_dir: &Path) -> PathBuf {
    config_dir.join(NATIVE_FIPS_STATUS_FILE_NAME)
}

#[cfg(any(test, target_os = "ios", target_os = "android"))]
fn load_native_fips_status(config_dir: &Path) -> Option<Value> {
    let data = std::fs::read(native_fips_status_path(config_dir)).ok()?;
    serde_json::from_slice(&data).ok()
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
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

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
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

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
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
                facet.has_purpose(iris_drive_core::NostrIdentityKeyPurpose::RecoveryPhrase)
            });
    if !phrase_matches_profile {
        return RecoverySecretExport {
            error: "recovery phrase does not match NostrIdentity".to_owned(),
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

fn generate_recovery_key_value() -> GeneratedRecoveryKey {
    let phrase = match iris_drive_core::recovery_phrase::generate_recovery_phrase() {
        Ok(phrase) => phrase,
        Err(error) => {
            return GeneratedRecoveryKey {
                error: format!("generating recovery phrase: {error}"),
                ..GeneratedRecoveryKey::default()
            };
        }
    };
    let keys = match iris_drive_core::recovery_phrase::recovery_phrase_to_keys(&phrase) {
        Ok(keys) => keys,
        Err(error) => {
            return GeneratedRecoveryKey {
                error: format!("deriving recovery key: {error}"),
                ..GeneratedRecoveryKey::default()
            };
        }
    };
    let recovery_pubkey = match keys.public_key().to_bech32() {
        Ok(npub) => npub,
        Err(error) => {
            return GeneratedRecoveryKey {
                error: format!("encoding recovery key: {error}"),
                ..GeneratedRecoveryKey::default()
            };
        }
    };
    GeneratedRecoveryKey {
        words: phrase.split_whitespace().map(ToOwned::to_owned).collect(),
        recovery_pubkey,
        error: String::new(),
    }
}

fn recovery_pubkey_for_phrase_value(recovery_phrase: &str) -> GeneratedRecoveryKey {
    let keys = match iris_drive_core::recovery_phrase::recovery_phrase_to_keys(recovery_phrase) {
        Ok(keys) => keys,
        Err(error) => {
            return GeneratedRecoveryKey {
                error: error.to_string(),
                ..GeneratedRecoveryKey::default()
            };
        }
    };
    let recovery_pubkey = match keys.public_key().to_bech32() {
        Ok(npub) => npub,
        Err(error) => {
            return GeneratedRecoveryKey {
                error: format!("encoding recovery key: {error}"),
                ..GeneratedRecoveryKey::default()
            };
        }
    };
    GeneratedRecoveryKey {
        recovery_pubkey,
        error: String::new(),
        ..GeneratedRecoveryKey::default()
    }
}

fn drive_link_for_cid_value(root_cid: &str) -> DriveLinkForCid {
    match drive_iris_to_nhash_url_for_root(root_cid) {
        Some(url) => DriveLinkForCid {
            url,
            error: String::new(),
        },
        None => DriveLinkForCid {
            error: "invalid content id".to_owned(),
            ..DriveLinkForCid::default()
        },
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

fn app_actors_from_account(
    state: &iris_drive_core::ProfileState,
    fips_status: &UiFipsStatus,
) -> Vec<UiAppActor> {
    let mut rows = Vec::new();
    if let Some(app_keys) = state.current_app_keys_projection() {
        let current_app_key_npub = pubkey_npub(&state.app_key_pubkey);
        let current_app_key_online = fips_status.fresh
            && (fips_status.endpoint_npub.is_empty()
                || fips_status.endpoint_npub == current_app_key_npub);
        let connectivity = app_key_connectivity_from_fips_status(fips_status);

        rows.extend(
            app_key_roster_rows(
                &app_keys.app_actors,
                &state.app_key_pubkey,
                state.can_admin_profile(),
                current_app_key_online,
                &connectivity,
            )
            .iter()
            .map(|app_key| UiAppActor {
                actor_kind: "device".to_owned(),
                pubkey: app_key.npub.clone(),
                label: local_app_key_row_label(state, app_key),
                display_label: local_app_key_row_display_label(state, app_key),
                state: app_key.state.clone(),
                state_label: app_key.state_label.clone(),
                connection_label: app_key.connection_label.clone(),
                connection_state: app_key.connection_state.clone(),
                role: app_key.role.clone(),
                role_label: app_key.role_label.clone(),
                detail: app_key.npub.clone(),
                is_current_app_key: app_key.is_current_app_key,
                is_online: app_key.is_online,
                can_revoke: app_key.can_revoke,
                can_appoint_admin: app_key.can_appoint_admin,
                can_demote_admin: app_key.can_demote_admin,
            }),
        );
    }
    rows.extend(recovery_devices_from_account(state));
    rows
}

fn local_app_key_row_label(
    state: &iris_drive_core::ProfileState,
    app_key: &iris_drive_core::app_key_summary::AppKeyRosterRow,
) -> String {
    if app_key.is_current_app_key {
        state.app_key_label.clone().unwrap_or_default()
    } else {
        app_key.label.clone().unwrap_or_default()
    }
}

fn local_app_key_row_display_label(
    state: &iris_drive_core::ProfileState,
    app_key: &iris_drive_core::app_key_summary::AppKeyRosterRow,
) -> String {
    let label = local_app_key_row_label(state, app_key);
    if label.trim().is_empty() {
        app_key.display_label.clone()
    } else {
        label
    }
}

fn recovery_devices_from_account(state: &iris_drive_core::ProfileState) -> Vec<UiAppActor> {
    let projection = state.profile_projection();
    let current_epoch = projection.secret_epochs.keys().next_back().copied();
    projection
        .active_facets
        .values()
        .filter(|facet| facet.has_purpose(iris_drive_core::NostrIdentityKeyPurpose::RecoveryPhrase))
        .map(|facet| {
            let npub = pubkey_npub(&facet.pubkey);
            let has_current_wrap = current_epoch.is_some_and(|epoch| {
                projection.secret_wrap_status(&facet.pubkey, epoch)
                    == iris_drive_core::SecretWrapStatus::Available
            });
            let label = facet
                .label
                .as_deref()
                .map(str::trim)
                .filter(|label| !label.is_empty())
                .unwrap_or("Recovery key")
                .to_owned();
            UiAppActor {
                actor_kind: "recovery_key".to_owned(),
                pubkey: npub.clone(),
                label: label.clone(),
                display_label: label,
                state: if has_current_wrap {
                    "linked"
                } else {
                    "repair_needed"
                }
                .to_owned(),
                state_label: if has_current_wrap {
                    "Linked"
                } else {
                    "Needs key wrap"
                }
                .to_owned(),
                connection_label: "Recovery key".to_owned(),
                connection_state: "recovery".to_owned(),
                role: "recovery".to_owned(),
                role_label: "Recovery".to_owned(),
                detail: npub,
                is_current_app_key: false,
                is_online: false,
                can_revoke: false,
                can_appoint_admin: false,
                can_demote_admin: false,
            }
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
        source_path: share.source_path,
        shared_with_me_path: share.shared_with_me_path,
        role: share.local_role.as_str().to_owned(),
        role_label: share.local_role.label().to_owned(),
        key_status: share.key_status.as_str().to_owned(),
        key_status_label: share.key_status.label().to_owned(),
        write_authorization: share.write_authorization.as_str().to_owned(),
        write_authorization_label: share.write_authorization.label().to_owned(),
        can_write: share.can_write,
        can_admin: share.can_admin,
        current_key_epoch: share.current_key_epoch,
        has_current_key_wrap: share.has_current_key_wrap,
        key_unavailable: share.key_unavailable,
        repair_needed: share.repair_needed,
        missing_key_wrap_count: share.missing_key_wrap_count as u64,
        missing_key_wraps: share
            .missing_key_wrap_pubkeys
            .iter()
            .map(|pubkey| pubkey_npub(pubkey))
            .collect(),
        participant_count: share.participant_count as u64,
        app_key_count: share.app_key_count as u64,
        members: share
            .members
            .into_iter()
            .map(|member| UiShareMember {
                profile_id: member.profile_id.to_string(),
                display_name: member.display_name,
                representative_npub_hint: member.representative_npub_hint.unwrap_or_default(),
                role: member.role.as_str().to_owned(),
                role_label: member.role.label().to_owned(),
                status: member.status.as_str().to_owned(),
                status_label: member.status.label().to_owned(),
                app_key_count: member.app_key_count as u64,
                can_revoke: member.can_revoke,
                can_change_role: member.can_change_role,
            })
            .collect(),
        pending_invites: share
            .pending_invites
            .into_iter()
            .map(|invite| UiPendingShareInvite {
                representative_npub_hint: invite.representative_npub_hint,
                display_name: invite.display_name,
                role: invite.role.as_str().to_owned(),
                role_label: invite.role.label().to_owned(),
                status: invite.status.as_str().to_owned(),
                status_label: invite.status.label().to_owned(),
                created_at: invite.created_at,
            })
            .collect(),
        shortcut_paths: share.shortcut_paths,
    })
    .collect()
}

fn app_key_connectivity_from_fips_status(fips_status: &UiFipsStatus) -> AppKeyConnectivity {
    if !fips_status.fresh {
        return AppKeyConnectivity::default();
    }
    AppKeyConnectivity {
        online_app_keys: fips_status.online_devices.iter().cloned().collect(),
        direct_app_keys: fips_status.direct_devices.iter().cloned().collect(),
        mesh_app_keys: fips_status.mesh_devices.iter().cloned().collect(),
        peer_statuses: fips_status
            .peer_statuses
            .iter()
            .map(|peer| {
                (
                    peer.npub.clone(),
                    AppKeyConnectionDetails {
                        transport_type: label_option(&peer.transport_type),
                        srtt_ms: peer.srtt_ms,
                    },
                )
            })
            .collect(),
    }
}

fn app_key_link_request_url(state: &iris_drive_core::ProfileState, config_dir: &Path) -> String {
    if state.can_admin_profile()
        || state.authorization_state != AppKeyAuthorizationState::AwaitingApproval
    {
        return String::new();
    }
    let Some(pending) = state.outbound_app_key_link_request.as_ref() else {
        return String::new();
    };
    if !pending.request_url.trim().is_empty() {
        return pending.request_url.clone();
    }
    let Ok(app_key) = iris_drive_core::AppKey::load(key_path_in(config_dir)) else {
        return String::new();
    };
    encode_app_key_approval_request(
        app_key.keys(),
        state.profile_id,
        Some(&pending.admin_app_key_pubkey),
        state.app_key_label.as_deref(),
        pending.requested_at,
    )
    .unwrap_or_default()
}

fn ensure_cached_app_key_link_request_url(
    config: &mut AppConfig,
    config_dir: &Path,
) -> Result<(), String> {
    let Some(state) = config.profile.as_ref() else {
        return Ok(());
    };
    if state.can_admin_profile()
        || state.authorization_state != AppKeyAuthorizationState::AwaitingApproval
    {
        return Ok(());
    }
    let Some(pending) = state.outbound_app_key_link_request.as_ref() else {
        return Ok(());
    };
    if !pending.request_url.trim().is_empty() {
        return Ok(());
    }
    let profile_id = state.profile_id;
    let admin_app_key_pubkey = pending.admin_app_key_pubkey.clone();
    let app_key_label = state.app_key_label.clone();
    let requested_at = pending.requested_at;
    let app_key = iris_drive_core::AppKey::load(key_path_in(config_dir))
        .map_err(|error| error.to_string())?;
    let request_url = encode_app_key_approval_request(
        app_key.keys(),
        profile_id,
        Some(&admin_app_key_pubkey),
        app_key_label.as_deref(),
        requested_at,
    )
    .map_err(|error| error.to_string())?;
    if let Some(pending) = config
        .profile
        .as_mut()
        .and_then(|state| state.outbound_app_key_link_request.as_mut())
    {
        pending.request_url = request_url;
    }
    config
        .save(config_path_in(config_dir))
        .map_err(|error| error.to_string())
}

fn app_key_link_invite_url(state: &iris_drive_core::ProfileState) -> String {
    if !state.can_admin_profile() {
        return String::new();
    }
    let Ok(invite_pubkey) = iris_drive_core::app_key_link_invite_pubkey(&state.app_key_link_secret)
    else {
        return String::new();
    };
    iris_drive_core::app_key_link_invite::encode_app_key_link_invite(
        state.profile_id,
        &state.app_key_pubkey,
        &invite_pubkey,
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
        .map(|request| {
            let request_link = request.request_url.trim();
            UiAppKeyLinkRequest {
                app_key_pubkey: pubkey_npub(&request.app_key_pubkey),
                label: request.label.clone().unwrap_or_default(),
                requested_at: request.requested_at,
                request_link: if request_link.is_empty() {
                    pubkey_npub(&request.app_key_pubkey)
                } else {
                    request.request_url.clone()
                },
            }
        })
        .collect()
}

fn resolve_app_key_link_target(input: &str) -> Result<iris_drive_core::AppKeyLinkTarget, String> {
    iris_drive_core::resolve_app_key_link_target(input, None).map_err(|error| {
        if error.to_string().contains("NostrIdentity UUID") {
            "paste an NostrIdentity invite URL to link this device".to_owned()
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
        invite_pubkey: String::new(),
        label: None,
    })
}

#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
fn app_key_approval_invite_pubkey(request: &str) -> String {
    parse_app_key_approval_request(request)
        .ok()
        .flatten()
        .map(|request| request.invite_pubkey)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn optional_trimmed(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn parse_share_role(value: &str) -> anyhow::Result<iris_drive_core::ShareRole> {
    iris_drive_core::ShareRole::parse_user_input(value).ok_or_else(|| {
        anyhow::anyhow!(
            "invalid share role {}; expected reader, editor, or admin",
            value.trim()
        )
    })
}

fn share_now_seconds() -> i64 {
    i64::try_from(unix_now_seconds()).unwrap_or(i64::MAX)
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
                .and_then(|drive| drive.app_key_roots.get(&account.app_key_pubkey))
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
mod app_key_link_flow_tests;
#[cfg(test)]
mod backup_tests;
#[cfg(test)]
mod browser_gateway_tests;
#[cfg(test)]
mod idle_tests;
#[cfg(test)]
mod provider_tests;
#[cfg(test)]
mod tests;
