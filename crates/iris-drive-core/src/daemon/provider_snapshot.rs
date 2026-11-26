//! A local materialization can serve provider reads while its exact source
//! heads are being fetched. Keep this separate from the author-aware merge.
use std::io::{self, ErrorKind};

use hashtree_core::{Cid, HashTreeError, Store, sha256, to_hex};

use super::{Daemon, DaemonError, PRIMARY_DRIVE_ID};
use crate::indexer::{IndexError, filter_ignored_entries_from_root, read_root_meta};
use crate::merge::{MergedEntry, walk_app_key_tree};
use crate::projection::{PrimaryMergedRoot, PrimaryMergedView, ProjectionError};
use crate::root_meta::{DriveRootMeta, root_cid_identity_matches};

impl Daemon {
    /// Returns the provider view, visible root, and whether those were served
    /// from a verified local materialization while a source was unavailable.
    pub async fn provider_projection(
        &self,
    ) -> Result<(PrimaryMergedView, PrimaryMergedRoot, bool), DaemonError> {
        let view = crate::primary_merged_view(self.tree(), self.config()).await;
        let result = match view {
            Ok(view) => crate::primary_merged_root_from_view(self.tree(), self.config(), &view)
                .await
                .map(|root| (view, root)),
            Err(error) => Err(error),
        };
        match result {
            Ok((view, root)) => Ok((view, root, false)),
            Err(error) => self
                .provider_snapshot_for_error(error)
                .await
                .map(|(view, root)| (view, root, true)),
        }
    }

    /// A status/list query does not need to build a new content-addressed
    /// directory when the author-aware merge already produced its entries.
    pub async fn provider_view(&self) -> Result<(PrimaryMergedView, bool), DaemonError> {
        match crate::primary_merged_view(self.tree(), self.config()).await {
            Ok(view) => Ok((view, false)),
            Err(error) => self
                .provider_snapshot_for_error(error)
                .await
                .map(|(view, _)| (view, true)),
        }
    }

    async fn provider_snapshot_for_error(
        &self,
        error: ProjectionError,
    ) -> Result<(PrimaryMergedView, PrimaryMergedRoot), DaemonError> {
        if !matches!(
            error,
            ProjectionError::Tree(HashTreeError::MissingChunk(_))
                | ProjectionError::Index(IndexError::Tree(HashTreeError::MissingChunk(_)))
        ) {
            return Err(error.into());
        }
        let Some(cid) = self.current_provider_snapshot().await? else {
            return Err(error.into());
        };
        self.verify_provider_snapshot(&cid).await?;
        let (files, _) = walk_app_key_tree(self.tree(), &cid)
            .await
            .map_err(ProjectionError::from)?;
        let mut view = PrimaryMergedView::default();
        let account = self
            .config
            .profile
            .as_ref()
            .ok_or(ProjectionError::NoAccount)?;
        let drive = self
            .config
            .drive(PRIMARY_DRIVE_ID)
            .ok_or(DaemonError::PrimaryDriveMissing)?;
        view.authorized_app_keys = account.active_root_writer_app_key_pubkeys().len();
        view.app_key_roots_present = drive.active_app_key_root_count(Some(account));
        view.view.files = files
            .into_iter()
            .map(|file| MergedEntry {
                path: file.path,
                source_path: None,
                hash: file.hash,
                size: file.size,
                whole_file_hash: file.whole_file_hash,
                modified_at: file.modified_at,
                // Materializations retain visible entries, not author attribution.
                source_app_key_pubkey: String::new(),
                published_at: 0,
            })
            .collect();
        let root = PrimaryMergedRoot {
            top_level_entries: self
                .tree
                .list_directory_required(&cid)
                .await
                .map_err(ProjectionError::from)?
                .len(),
            root_cid: cid,
            file_count: view.file_count(),
        };
        Ok((view, root))
    }

    pub(super) async fn provider_import_base(
        &self,
        drive_id: &str,
        edited: &Cid,
        base: Option<&Cid>,
        paths: Option<&std::collections::BTreeSet<String>>,
    ) -> Result<(Option<Cid>, Option<Cid>), DaemonError> {
        let Some(base) = base.filter(|_| drive_id == PRIMARY_DRIVE_ID) else {
            return Ok((base.cloned(), None));
        };
        let (_, projection, cached) = self.provider_projection().await?;
        if !cached {
            return Ok((Some(projection.root_cid), None));
        }
        if &projection.root_cid != base
            || !crate::indexer::only_visible_additions(self.tree(), base, edited, paths).await?
        {
            return Err(DaemonError::Store("Waiting for device blocks before changing existing files or applying edits from an older view".into()));
        }
        let visible = filter_ignored_entries_from_root(self.tree(), edited).await?;
        self.verify_provider_snapshot(&visible).await?;
        Ok((Some(projection.root_cid), Some(visible)))
    }

    async fn current_provider_snapshot(&self) -> Result<Option<Cid>, DaemonError> {
        if !self.config.shared_folders.is_empty() || !self.config.share_shortcuts.is_empty() {
            return Ok(None);
        }
        // Daemon::open uses a lightweight profile without roster history.
        // Verify recovery coverage against the complete persisted authorization.
        let supplied = self
            .config
            .profile
            .as_ref()
            .ok_or(ProjectionError::NoAccount)?;
        let full =
            crate::AppConfig::load_or_default(crate::paths::config_path_in(self.config_dir()))?;
        if !full.shared_folders.is_empty() || !full.share_shortcuts.is_empty() {
            return Ok(None);
        }
        let Some(account) = full.profile.as_ref().filter(|p| {
            p.profile_id == supplied.profile_id
                && p.app_key_pubkey == supplied.app_key_pubkey
                && super::full_profile_covers_known_app_key_roots(p, &self.config)
        }) else {
            return Ok(None);
        };
        let drive = self
            .config
            .drive(PRIMARY_DRIVE_ID)
            .ok_or(DaemonError::PrimaryDriveMissing)?;
        let active = drive.active_app_key_roots(Some(account));
        let Some((_, head)) = active
            .iter()
            .find(|(key, _)| *key == &account.app_key_pubkey)
        else {
            return Ok(None);
        };
        let cid = Cid::parse(&head.root_cid).map_err(|e| DaemonError::Store(e.to_string()))?;
        let Some(meta) = read_root_meta(self.tree(), &cid).await? else {
            return Ok(None);
        };
        let peers = active
            .iter()
            .filter(|(key, _)| *key != &account.app_key_pubkey);
        let observed = meta
            .observed
            .keys()
            .filter(|key| *key != &account.app_key_pubkey);
        if meta.schema != DriveRootMeta::SCHEMA
            || meta.drive_id != drive.drive_id
            || meta.app_key_pubkey != account.app_key_pubkey
            || meta.app_key_seq != head.app_key_seq
            || meta.local_only != head.local_only
            || meta.dck_generation
                != account
                    .current_app_keys_projection()
                    .map_or(0, |s| s.dck_generation)
            || observed.count() != peers.clone().count()
            || !peers.clone().all(|(key, peer)| {
                !peer.local_only
                    && peer.app_key_seq > 0
                    && meta.observed.get(*key).is_some_and(|seen| {
                        seen.app_key_seq == peer.app_key_seq
                            && root_cid_identity_matches(&seen.root_cid, &peer.root_cid)
                    })
            })
        {
            return Ok(None);
        }
        if meta.local_only {
            return Ok(Some(
                filter_ignored_entries_from_root(self.tree(), &cid).await?,
            ));
        }
        let path = self.provider_snapshot_path(meta.app_key_seq);
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let (signed_hash, visible): (String, String) =
            serde_json::from_slice(&bytes).map_err(io::Error::other)?;
        if signed_hash != to_hex(&cid.hash) {
            return Ok(None);
        }
        Cid::parse(&visible)
            .map(Some)
            .map_err(|e| DaemonError::Store(e.to_string()))
    }

    pub(super) async fn verify_provider_snapshot(&self, cid: &Cid) -> Result<(), DaemonError> {
        let hashes = crate::block_sync::collect_live_sync_hashes_without_leaf_reads(
            self.tree(),
            cid,
            crate::block_sync::LIVE_SYNC_BLOCK_CONCURRENCY,
        )
        .await
        .map_err(ProjectionError::from)?;
        let store = self.tree.get_store();
        for hash in hashes {
            let bytes = store
                .get(&hash)
                .await
                .map_err(|e| DaemonError::Store(e.to_string()))?
                .ok_or_else(|| ProjectionError::Tree(HashTreeError::MissingChunk(to_hex(&hash))))?;
            if sha256(&bytes) != hash {
                return Err(DaemonError::Store(
                    "Cached provider block failed integrity check".into(),
                ));
            }
        }
        Ok(())
    }

    // Alternating by the monotonically increasing local sequence retains the
    // current checkpoint until publishing the next config succeeds. A crash
    // can never pair a checkpoint with a different signed root.
    fn provider_snapshot_path(&self, seq: u64) -> std::path::PathBuf {
        self.config_dir()
            .join(format!("provider-snapshot-{}.json", seq % 2))
    }

    pub(super) fn save_provider_snapshot(
        &self,
        signed: &Cid,
        seq: u64,
        visible: &Cid,
    ) -> Result<(), DaemonError> {
        let bytes = serde_json::to_vec(&(to_hex(&signed.hash), visible.to_string()))
            .map_err(io::Error::other)?;
        crate::atomic_file::atomic_write_private(&self.provider_snapshot_path(seq), &bytes)?;
        Ok(())
    }
}
