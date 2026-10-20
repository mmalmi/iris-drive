//! Existing backup target mutations shared by native shells.

use std::path::Path;

use anyhow::Context;
use iris_drive_core::backup_ops::{
    add_backup_target as core_add_backup_target, check_backups as core_check_backups,
    default_backup_check_sample_size, remove_backup_target as core_remove_backup_target,
    sync_backups as core_sync_backups,
};

use super::{NativeAppRuntime, label_option};

impl NativeAppRuntime {
    pub(super) fn add_backup_target(&mut self, target: &str, label: &str) {
        if let Err(error) =
            core_add_backup_target(Path::new(&self.data_dir), target, label_option(label))
        {
            self.state.error = format!("adding backup target: {error:#}");
        }
    }

    pub(super) fn remove_backup_target(&mut self, target: &str) {
        if let Err(error) = core_remove_backup_target(Path::new(&self.data_dir), target) {
            self.state.error = format!("removing backup target: {error:#}");
        }
    }

    pub(super) fn sync_backups(&mut self, target: &str) {
        let data_dir = self.data_dir.clone();
        let target = label_option(target);
        match block_on_backup_operation(async move {
            core_sync_backups(Path::new(&data_dir), target.as_deref()).await
        }) {
            Ok(_) => {}
            Err(error) => self.state.error = format!("syncing backups: {error:#}"),
        }
    }

    pub(super) fn check_backups(&mut self, target: &str) {
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
}

pub(super) fn block_on_backup_operation<T>(
    future: impl std::future::Future<Output = anyhow::Result<T>>,
) -> anyhow::Result<T> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building backup runtime")?;
    runtime.block_on(future)
}
