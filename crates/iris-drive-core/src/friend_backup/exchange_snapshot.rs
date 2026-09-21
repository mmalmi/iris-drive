//! Select and cache the complete visible Drive snapshot without publishing a root.

use anyhow::Result;
use hashtree_core::{sha256, to_hex};
use nostr_sdk::nips::nip44;

use super::{Exchange, ExportedBackup};
use crate::friend_backup::storage::prepare_backup;

impl Exchange {
    pub(super) async fn refresh_snapshot(&mut self) -> Result<()> {
        let daemon = crate::Daemon::open(&self.config_dir)?;
        let config = daemon.config();
        // Profile serialization omits its derived authorization roster, so include
        // the active writers explicitly. Unrelated relay/status changes do not
        // require walking and materializing the entire visible Drive again.
        let writers = config.profile.as_ref().map(|profile| {
            (
                profile.has_profile_roster_evidence(),
                profile.active_root_writer_app_key_pubkeys(),
            )
        });
        let source_fingerprint = sha256(&serde_json::to_vec(&(
            &config.drives,
            &config.shared_folders,
            &config.share_shortcuts,
            writers,
        ))?);
        if self
            .prepared
            .as_ref()
            .is_some_and(|existing| existing.source_fingerprint == source_fingerprint)
        {
            return Ok(());
        }
        let root = crate::primary_merged_root(daemon.tree(), config)
            .await?
            .root_cid;
        let root_string = root.to_string();
        if let Some(existing) = self.prepared.as_mut()
            && existing.root == root_string
        {
            existing.source_fingerprint = source_fingerprint;
            return Ok(());
        }
        let capsule = nip44::encrypt(
            self.keys.secret_key(),
            &self.keys.public_key(),
            &root_string,
            nip44::Version::V2,
        )?;
        let export_dir = self.config_dir.join("friend-backup-exports");
        std::fs::create_dir_all(&export_dir)?;
        let directory = tempfile::tempdir_in(export_dir)?;
        let backup = prepare_backup(daemon.tree(), &root, capsule, directory.path()).await?;
        // Only cache a fully prepared snapshot. Missing blocks and other transient
        // failures must retry even when no config field has changed.
        self.prepared = Some(ExportedBackup {
            root: root_string,
            source_fingerprint,
            backup,
            _directory: directory,
        });
        self.last_offer = None;
        let root_hash = to_hex(&root.hash);
        for status in self.status.values_mut() {
            if status.root_hash.as_deref() != Some(root_hash.as_str()) {
                status.last_checked_at = None;
                status.last_synced_at = None;
                status.last_audit_attempt_at = None;
                status.root_hash = None;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "exchange_snapshot_tests.rs"]
mod tests;
