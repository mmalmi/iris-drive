use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::json;

use crate::config::AppConfig;
use crate::paths::key_path_in;

pub struct EmbeddedHashtreeHost {
    runtime: hashtree_embedded::HostDaemonRuntime,
    status: hashtree_embedded::HostDaemonStatus,
}

impl EmbeddedHashtreeHost {
    pub fn start(config_dir: &Path, config: &AppConfig) -> Result<Self> {
        let state_root = embedded_hashtree_state_root_in(config_dir);
        let embedded_config_dir = state_root.join("config");
        std::fs::create_dir_all(&embedded_config_dir)
            .with_context(|| format!("creating {}", embedded_config_dir.display()))?;
        let settings = embedded_browser_settings(config);
        let settings_path = embedded_config_dir.join("browser_settings.json");
        std::fs::write(&settings_path, serde_json::to_vec_pretty(&settings)?)
            .with_context(|| format!("writing {}", settings_path.display()))?;
        let device_key_path = key_path_in(config_dir);
        if device_key_path.exists() {
            std::fs::copy(&device_key_path, embedded_config_dir.join("keys")).with_context(
                || {
                    format!(
                        "copying Iris Drive app key from {}",
                        device_key_path.display()
                    )
                },
            )?;
        }

        let runtime = hashtree_embedded::HostDaemonRuntime::start(
            hashtree_embedded::HostDaemonOptions::new(state_root),
        )?;
        let status = runtime.status();
        Ok(Self { runtime, status })
    }

    #[must_use]
    pub fn status(&self) -> &hashtree_embedded::HostDaemonStatus {
        &self.status
    }

    #[must_use]
    pub fn status_payload(&self) -> serde_json::Value {
        json!({
            "base_url": self.status.base_url.clone(),
            "self_npub": self.status.self_npub.clone(),
            "config_dir": self.status.config_dir.display().to_string(),
            "data_dir": self.status.data_dir.display().to_string(),
        })
    }
}

impl Drop for EmbeddedHashtreeHost {
    fn drop(&mut self) {
        self.runtime.shutdown();
    }
}

pub(crate) fn embedded_browser_settings(config: &AppConfig) -> serde_json::Value {
    json!({
        "storageMaxSizeGb": 1,
        "socialGraphDbMaxSizeGb": 1,
        "spamboxDbMaxSizeGb": 0,
        "nostrRelays": embedded_browser_nostr_relays(config),
        "blossomReadServers": config.blossom_servers.clone(),
        "blossomWriteServers": config.blossom_servers.clone(),
        "enableWebrtc": false,
        "enableMulticast": false,
        "enableFips": false,
        "enableFipsUdp": false,
        "enableFipsWebrtc": false,
        "fetchFromFipsPeers": false,
        "socialGraphCrawlDepth": 0,
        "syncEnabled": false,
        "syncOwn": false,
        "syncFollowed": false,
        "publicWrites": false,
        "publicPlaintextReads": false,
        "allowedNpubs": [crate::gateway::IRIS_SITES_PORTAL_NPUB],
    })
}

pub(crate) fn embedded_browser_nostr_relays(config: &AppConfig) -> Vec<String> {
    let mut relays = config.relays.clone();
    for relay in hashtree_resolver::nostr::NostrResolverConfig::default().relays {
        if !relays.iter().any(|existing| same_relay(existing, &relay)) {
            relays.push(relay);
        }
    }
    relays
}

pub(crate) fn same_relay(left: &str, right: &str) -> bool {
    left.trim()
        .trim_end_matches('/')
        .eq_ignore_ascii_case(right.trim().trim_end_matches('/'))
}

#[must_use]
pub fn embedded_hashtree_state_root_in(config_dir: &Path) -> PathBuf {
    if config_dir.file_name().and_then(|name| name.to_str()) == Some("Config")
        && let Some(app_data_dir) = config_dir.parent()
    {
        return app_data_dir.join("Hashtree");
    }
    config_dir.join("Hashtree")
}
