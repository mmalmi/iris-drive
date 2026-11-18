//! Restore an existing Drive from its recovery phrase.
//!
//! The phrase finds the account on the configured relays and admits this
//! install. Each device's latest root is opened through the wrap it made for
//! the phrase, so the files come back even when no other device is left. The
//! phrase key lives only in memory during the restore; it is never saved.
//! When the relays know no account for the phrase, the restore makes a new
//! account with the phrase, as before.

use super::{FfiApp, ProviderSummaryMode, label_option};
use crate::{NativeAppAction, NativeAppState};
use anyhow::{Context, Result, ensure};
use iris_drive_core::recovery_restore::{
    RecoveryPhraseRestore, publish_recovered_drive_root, restore_from_recovery_phrase,
};
use iris_drive_core::{
    AppConfig, config_lock::ConfigMutationLock, paths::config_path_in, paths::key_path_in,
    recovery_phrase::validate_recovery_phrase, relay_sync,
};
use std::path::Path;
use std::time::Duration;

const ACCOUNT_LOOKUP_TIMEOUT: Duration = Duration::from_secs(8);
const RESTORE_PUBLISH_TIMEOUT: Duration = Duration::from_secs(30);

impl FfiApp {
    pub(super) fn dispatch_recovery_phrase_restore(
        &self,
        action: &NativeAppAction,
    ) -> Option<NativeAppState> {
        let NativeAppAction::RestoreProfile {
            recovery_secret,
            app_key_label,
        } = action
        else {
            return None;
        };
        let phrase = validate_recovery_phrase(recovery_secret).ok()?;
        Some(self.restore_with_recovery_phrase(&phrase, app_key_label))
    }

    fn restore_with_recovery_phrase(&self, phrase: &str, label: &str) -> NativeAppState {
        let dir = self.with_runtime(|r| r.data_dir.clone());
        let result = restore_existing_profile(Path::new(&dir), phrase, label);
        match result {
            Ok(Some(root_cids)) => {
                let state = self.with_runtime(|r| {
                    r.state_revision = r.state_revision.wrapping_add(1);
                    r.state.error.clear();
                    r.set_sync_running(true);
                    r.reload_from_disk(ProviderSummaryMode::Skip);
                    r.reconcile_app_key_link_exchange(true);
                    r.start_browser_gateway_if_needed();
                    r.state()
                });
                self.bring_back_restored_files(dir, root_cids);
                state
            }
            // No account uses this phrase: make a new one with it, as before.
            Ok(None) => self.with_runtime(|r| {
                r.state_revision = r.state_revision.wrapping_add(1);
                r.dispatch(NativeAppAction::RestoreProfile {
                    recovery_secret: phrase.to_owned(),
                    app_key_label: label.to_owned(),
                });
                r.state()
            }),
            Err(error) => self.with_runtime(|r| {
                r.state_revision = r.state_revision.wrapping_add(1);
                r.state.error = format!("restoring from recovery phrase: {error:#}");
                r.state()
            }),
        }
    }

    /// Download the restored roots and publish this device's own root in the
    /// background, so the setup screen does not wait for the whole Drive.
    fn bring_back_restored_files(&self, dir: String, root_cids: Vec<String>) {
        let runtime = self.runtime.clone();
        std::thread::spawn(move || {
            let dir = Path::new(&dir);
            let result = (|| -> Result<()> {
                let relays = AppConfig::load_or_default(config_path_in(dir))?.relays;
                let tokio = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()?;
                tokio.block_on(async {
                    let client = relay_sync::connect(&relays).await?;
                    let result = publish_recovered_drive_root(
                        dir,
                        &client,
                        &root_cids,
                        RESTORE_PUBLISH_TIMEOUT,
                    )
                    .await;
                    relay_sync::shutdown_client(&client).await;
                    result.map(|_| ())
                })
            })();
            let mut runtime = runtime
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            runtime.state_revision = runtime.state_revision.wrapping_add(1);
            match result {
                Ok(()) => runtime.reload_from_disk(ProviderSummaryMode::Skip),
                Err(error) => {
                    tracing::warn!(error = %format!("{error:#}"), "restoring Drive files failed");
                    runtime.state.error = format!("bringing back your files: {error:#}");
                }
            }
        });
    }
}

/// Join the account the phrase belongs to and save it. Returns the roots to
/// download, or `None` when the relays know no account for the phrase.
fn restore_existing_profile(dir: &Path, phrase: &str, label: &str) -> Result<Option<Vec<String>>> {
    let config = AppConfig::load_or_default(config_path_in(dir))?;
    ensure!(
        config
            .profile
            .as_ref()
            .is_none_or(|p| p.profile_roster_ops.is_empty() && !p.is_authorized()),
        "already initialized"
    );
    // Unit tests make throwaway profiles with the production relays. Keep
    // them off the network; tests that restore configure a loopback relay.
    #[cfg(test)]
    if !config
        .relays
        .iter()
        .all(|relay| relay.starts_with("ws://127.0.0.1:") || relay.starts_with("ws://[::1]:"))
    {
        return Ok(None);
    }
    let tokio = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let restored = tokio.block_on(async {
        let client = relay_sync::connect(&config.relays).await?;
        let result = restore_from_recovery_phrase(
            dir,
            &client,
            phrase,
            label_option(label),
            ACCOUNT_LOOKUP_TIMEOUT,
        )
        .await;
        relay_sync::shutdown_client(&client).await;
        result
    })?;
    let Some(restored) = restored else {
        return Ok(None);
    };
    let root_cids = restored.drive_roots.root_cids_to_download.clone();
    persist_restore(dir, &config, restored)?;
    Ok(Some(root_cids))
}

fn persist_restore(dir: &Path, config: &AppConfig, restored: RecoveryPhraseRestore) -> Result<()> {
    let _guard = ConfigMutationLock::acquire_blocking(dir)?;
    let mut current = AppConfig::load_or_default(config_path_in(dir))?;
    ensure!(
        current
            .profile
            .as_ref()
            .map(|p| (p.profile_id, &p.app_key_pubkey))
            == config
                .profile
                .as_ref()
                .map(|p| (p.profile_id, &p.app_key_pubkey)),
        "Drive setup changed during restore"
    );
    let app_key = &restored.profile.app_key;
    if key_path_in(dir).exists() {
        ensure!(
            iris_drive_core::AppKey::load(key_path_in(dir))?.pubkey_hex() == app_key.pubkey_hex(),
            "The device key changed during restore"
        );
    } else {
        app_key.save().context("saving this device's key")?;
    }
    // A pending, unbound join has a placeholder drive. Replace it.
    current.upsert_drive(restored.drive);
    current.remove_drive(iris_drive_core::private_trash::DRIVE_ID);
    if let Some(trash) = restored.trash {
        current.upsert_drive(trash);
    }
    current.profile = Some(restored.profile.state);
    current.save(config_path_in(dir))?;
    Ok(())
}
