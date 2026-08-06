use crate::actions::NativeAppAction;

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) const APP_KEY_LINK_REQUEST_RETRY_SECS: u64 = 30;
#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) const APP_KEY_LINK_REQUEST_STARTUP_RETRY_MILLIS: u64 = 1_000;
#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) const APP_KEY_LINK_REQUEST_STARTUP_BURST_ATTEMPTS: u8 = 90;
#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
pub(super) const APP_KEY_LINK_ROSTER_RETRY_SECS: u64 = 2;
#[cfg(all(not(test), any(target_os = "ios", target_os = "android")))]
pub(super) const APP_KEY_LINK_RELAY_PUBLISH_TIMEOUT_SECS: u64 = 5;
#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) const APP_KEY_LINK_EXCHANGE_ACTIVE_TICK_MILLIS: u64 =
    APP_KEY_LINK_REQUEST_STARTUP_RETRY_MILLIS;
#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) const APP_KEY_LINK_EXCHANGE_IDLE_TICK_MILLIS: u64 = 15_000;

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
#[derive(Debug, Clone, Copy)]
pub(super) struct SentAppKeyLinkRequest {
    pub(super) last_sent: std::time::Instant,
    pub(super) attempts: u8,
}

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) fn app_key_link_request_send_due(
    sent: Option<SentAppKeyLinkRequest>,
    now: std::time::Instant,
) -> bool {
    let Some(sent) = sent else {
        return true;
    };
    now.duration_since(sent.last_sent) >= app_key_link_request_retry_interval(sent.attempts)
}

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) fn app_key_link_request_retry_interval(attempts: u8) -> std::time::Duration {
    if attempts < APP_KEY_LINK_REQUEST_STARTUP_BURST_ATTEMPTS {
        std::time::Duration::from_millis(APP_KEY_LINK_REQUEST_STARTUP_RETRY_MILLIS)
    } else {
        std::time::Duration::from_secs(APP_KEY_LINK_REQUEST_RETRY_SECS)
    }
}

#[cfg(any(test, all(not(test), any(target_os = "ios", target_os = "android"))))]
pub(super) fn app_key_link_exchange_tick_millis(
    state: Option<&iris_drive_core::ProfileState>,
) -> u64 {
    let approval_pending = state.is_some_and(|state| {
        state.authorization_state == iris_drive_core::AppKeyAuthorizationState::AwaitingApproval
            || !state.pending_device_approval_receipts.is_empty()
    });
    if approval_pending {
        APP_KEY_LINK_EXCHANGE_ACTIVE_TICK_MILLIS
    } else {
        APP_KEY_LINK_EXCHANGE_IDLE_TICK_MILLIS
    }
}

pub(super) fn native_action_uses_short_config_transaction(action: &NativeAppAction) -> bool {
    !matches!(
        action,
        NativeAppAction::Refresh
            | NativeAppAction::RefreshProfile
            | NativeAppAction::SetLaunchOnStartup { .. }
            | NativeAppAction::SyncBackups { .. }
            | NativeAppAction::CheckBackups { .. }
            | NativeAppAction::StartSync
            | NativeAppAction::StopSync
            | NativeAppAction::RestartSync
            | NativeAppAction::ExportShareRecipientEvidence { .. }
            | NativeAppAction::ImportFile { .. }
            | NativeAppAction::ImportContentLink { .. }
    )
}
