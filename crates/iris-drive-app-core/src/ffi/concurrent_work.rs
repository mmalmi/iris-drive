use super::{FfiApp, NativeAppRuntime, ProviderSummaryMode};
use crate::NativeAppAction;
use crate::native_provider::{
    native_sync_status_label, run_native_provider_list, run_native_sync_once,
};
use crate::state::NativeAppState;

impl FfiApp {
    /// Provider scans and network sync can wait on peers. Keep those waits out
    /// of the shared state lock used by a new screen and explicit approvals.
    pub(super) fn dispatch_concurrent_work(&self, action: NativeAppAction) -> NativeAppState {
        let work = self.with_runtime(|runtime| runtime.background_work.clone());
        let _work = work
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sync = matches!(
            action,
            NativeAppAction::StartSync | NativeAppAction::RestartSync
        );
        let prepared = self.with_runtime(|runtime| {
            runtime.state_revision = runtime.state_revision.wrapping_add(1);
            runtime.dispatch(NativeAppAction::RefreshProfile);
            if sync {
                if !runtime.persist_sync_intent(true) {
                    return None;
                }
                runtime.set_sync_running(true);
                runtime.reconcile_app_key_link_exchange(true);
            }
            Some((runtime.data_dir.clone(), runtime.state_revision))
        });
        let Some((data_dir, revision)) = prepared else {
            return self.state();
        };

        let sync_result = sync.then(|| run_native_sync_once(&data_dir));
        let provider = run_native_provider_list(&data_dir);

        self.with_runtime(|runtime| {
            // Approval, logout, pause, or a later action wins over this work.
            // Its disk changes are picked up by the next ordinary refresh.
            if runtime.state_revision != revision {
                return runtime.state();
            }
            if let Some(result) = sync_result {
                runtime.apply_sync_result(result);
            }
            runtime.reload_from_disk_preserving_error(ProviderSummaryMode::Skip);
            if let Ok(value) = provider {
                runtime.apply_provider_summary(&value);
            }
            runtime.reconcile_app_key_link_exchange(sync);
            runtime.start_browser_gateway_if_needed();
            runtime.state()
        })
    }
}

impl NativeAppRuntime {
    pub(super) fn apply_sync_result(
        &mut self,
        result: anyhow::Result<iris_drive_core::NetworkSyncReport>,
    ) {
        match result {
            Ok(report) => {
                native_sync_status_label(&report).clone_into(&mut self.state.ui.sync.status);
            }
            Err(error) => {
                "sync error".clone_into(&mut self.state.ui.sync.status);
                self.state.error = format!("syncing drive: {error:#}");
            }
        }
        self.refresh_sync_status_label();
    }
}
