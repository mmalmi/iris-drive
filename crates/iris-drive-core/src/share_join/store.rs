//! Join links this device answers, and links it asked to join, kept in a
//! private file beside the config. Links carry secret keys, so they never go
//! into `config.toml` or onto relays.

use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use super::ShareJoinLink;
use crate::NostrIdentityId;
use crate::sharing::{ShareRole, SharedFolder};

pub const SHARE_JOINS_FILE: &str = "share-joins.json";

/// A link this device opened and asked to join with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingShareJoin {
    pub link: ShareJoinLink,
    pub requested_at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShareJoinStore {
    /// Links made on this device; it lets in whoever asks with one.
    #[serde(default)]
    pub links: Vec<ShareJoinLink>,
    /// Folders this device asked to join and is waiting to be let into.
    #[serde(default)]
    pub pending: Vec<PendingShareJoin>,
}

impl ShareJoinStore {
    pub fn load(config_dir: &Path) -> Result<Self> {
        let path = config_dir.join(SHARE_JOINS_FILE);
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("reading {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub fn save(&self, config_dir: &Path) -> Result<()> {
        let path = config_dir.join(SHARE_JOINS_FILE);
        let bytes = serde_json::to_vec_pretty(self)?;
        crate::atomic_file::atomic_write_private(&path, &bytes)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// The link for `role`, made on first use and the same one every time after.
    pub fn link_for(
        &mut self,
        folder: &SharedFolder,
        admin: &str,
        role: ShareRole,
    ) -> Result<(ShareJoinLink, bool)> {
        if let Some(link) = self.links.iter().find(|link| {
            link.share_id == folder.share_id && link.role == role && link.admin == admin
        }) {
            return Ok((link.clone(), false));
        }
        let link = ShareJoinLink::new(folder, admin, role)?;
        self.links.push(link.clone());
        Ok((link, true))
    }

    /// Stop links from adding anyone else. People who joined keep their access.
    pub fn disable_links(&mut self, share_id: NostrIdentityId) -> bool {
        let before = self.links.len();
        self.links.retain(|link| link.share_id != share_id);
        before != self.links.len()
    }

    #[must_use]
    pub fn links_for(&self, share_id: NostrIdentityId) -> Vec<&ShareJoinLink> {
        self.links
            .iter()
            .filter(|link| link.share_id == share_id)
            .collect()
    }

    #[must_use]
    pub fn link_signed_by(&self, pubkey: &str) -> Option<&ShareJoinLink> {
        self.links
            .iter()
            .find(|link| link.pubkey().is_ok_and(|key| key == pubkey))
    }

    #[must_use]
    pub fn link_pubkeys(&self, admin: &str) -> Vec<String> {
        let mut keys = self
            .links
            .iter()
            .filter(|link| link.admin == admin)
            .filter_map(|link| link.pubkey().ok())
            .collect::<Vec<_>>();
        keys.sort();
        keys.dedup();
        keys
    }

    pub fn record_pending(&mut self, link: ShareJoinLink, requested_at: i64) {
        self.pending
            .retain(|item| item.link.share_id != link.share_id);
        self.pending.push(PendingShareJoin { link, requested_at });
    }

    pub fn finish_pending(&mut self, share_id: NostrIdentityId) -> bool {
        let before = self.pending.len();
        self.pending.retain(|item| item.link.share_id != share_id);
        before != self.pending.len()
    }
}
