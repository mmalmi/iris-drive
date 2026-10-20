use super::NativeAppRuntime;
use crate::native_provider::run_native_sync_pending_device_approval_acks;

impl NativeAppRuntime {
    pub(super) fn sync_approval_acks(&mut self) {
        match run_native_sync_pending_device_approval_acks(&self.data_dir) {
            Ok(report) => {
                if report.device_approval_applied_acks_applied > 0 {
                    self.set_sync_status(true, "approval acknowledged");
                }
            }
            Err(error) => {
                self.state.error = format!("syncing device approval ACK: {error:#}");
            }
        }
    }
}
