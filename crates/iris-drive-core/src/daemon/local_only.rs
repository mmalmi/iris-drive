use std::collections::BTreeSet;

use super::{Cid, Daemon, DaemonError};
use crate::indexer::remove_visible_path_if_present;

impl Daemon {
    pub(super) async fn local_only_tombstone_mask(
        &self,
        previous_root: Option<&Cid>,
    ) -> Result<Option<BTreeSet<String>>, DaemonError> {
        let Some(previous_root) = previous_root else {
            return Ok(None);
        };
        let (_, tombstones) = crate::merge::walk_app_key_tree(&self.tree, previous_root)
            .await
            .map_err(|e| DaemonError::Store(e.to_string()))?;
        let paths = tombstones
            .into_iter()
            .map(|tombstone| tombstone.path)
            .collect::<BTreeSet<_>>();
        Ok((!paths.is_empty()).then_some(paths))
    }

    pub(super) async fn remove_legacy_local_only_tombstoned_paths(
        &self,
        mut import_root: Cid,
        edited_root: &Cid,
        paths: &BTreeSet<String>,
    ) -> Result<Cid, DaemonError> {
        for path in paths {
            if self
                .tree
                .resolve(edited_root, path)
                .await
                .map_err(|e| DaemonError::Store(e.to_string()))?
                .is_none()
            {
                import_root =
                    remove_visible_path_if_present(&self.tree, &import_root, path).await?;
            }
        }
        Ok(import_root)
    }
}
