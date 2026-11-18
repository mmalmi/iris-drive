use hashtree_core::Cid;

use super::{
    Daemon, DaemonError, ImportReport, PRIMARY_DRIVE_ID, current_root_observes_drive_roots,
};

impl Daemon {
    /// Materialize the complete logical primary-drive view as a publishable
    /// root for a newly approved `AppKey`.
    ///
    /// Normal background convergence deliberately records merged projections
    /// as `local_only`, because another `AppKey` should retain authorship of its
    /// own contribution. Device approval is different: the new `AppKey` cannot
    /// decrypt root envelopes published before it joined. A one-time causal
    /// snapshot authored by the approving `AppKey` gives it the complete current
    /// view immediately, while retaining observations of every source root.
    pub async fn materialize_primary_merged_root_for_device_approval(
        &mut self,
    ) -> Result<Option<ImportReport>, DaemonError> {
        self.load_full_profile_for_mutation()?;
        let Some(account) = self.config.profile.clone() else {
            return Ok(None);
        };
        let drive = self
            .config
            .drive(PRIMARY_DRIVE_ID)
            .ok_or(DaemonError::PrimaryDriveMissing)?
            .clone();
        if drive.app_key_roots.is_empty() {
            let empty_root = self
                .tree
                .put_directory(Vec::new())
                .await
                .map_err(|error| DaemonError::Store(error.to_string()))?;
            return self
                .import_visible_root_for_drive(PRIMARY_DRIVE_ID, empty_root)
                .await
                .map(Some);
        }

        let merged = crate::primary_merged_root(&self.tree, &self.config).await?;
        if let Some(current) = drive.app_key_roots.get(&account.app_key_pubkey)
            && !current.local_only
        {
            let current_cid =
                Cid::parse(&current.root_cid).map_err(|e| DaemonError::Store(e.to_string()))?;
            let current_visible =
                crate::indexer::filter_ignored_entries_from_root(&self.tree, &current_cid).await?;
            if current_visible == merged.root_cid
                && current_root_observes_drive_roots(&self.tree, &current_cid, &account, &drive)
                    .await?
            {
                return Ok(None);
            }
        }

        self.import_visible_root_for_drive(PRIMARY_DRIVE_ID, merged.root_cid)
            .await
            .map(Some)
    }
}
