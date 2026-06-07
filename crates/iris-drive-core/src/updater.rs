use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use hashtree_updater::{
    ProductAssetPolicy, SecureNostrBlossomConfig, SecureNostrBlossomSelection,
    SecureNostrBlossomUpdater, build_secure_nostr_blossom_updater, current_archive_target,
    dedupe_nonempty, download_product_selection, env_csv, platform_app_asset_suffixes,
    preferred_product_asset, product_result_from_selection, select_product_update,
    update_ref_from_override,
};
pub use hashtree_updater::{
    ProductUpdateMode, ProductUpdateResult, SECURE_SOURCE_NAME, UpdateAsset, UpdateAutoCheckPolicy,
    UpdateManifest,
};

use crate::config::{AppConfig, DEFAULT_BLOSSOM_SERVERS, DEFAULT_RELAYS};
use crate::paths::config_path_in;

pub const HTREE_UPDATE_REF: &str = "htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-drive/latest";

const UPDATE_MANIFEST_TIMEOUT_SECS: u64 = 8;
const UPDATE_DOWNLOAD_TIMEOUT_SECS: u64 = 180;
const DEFAULT_UPDATE_BLOSSOM_READ_SERVERS: &[&str] = &[
    "https://cdn.iris.to",
    "https://hashtree.iris.to",
    "https://upload.iris.to",
    "https://blossom.primal.net",
];

#[derive(Clone, Debug, Default)]
pub struct ProductUpdateConfig {
    pub relays: Vec<String>,
    pub blossom_servers: Vec<String>,
    pub embedded_hashtree_base_url: Option<String>,
    pub update_ref: Option<String>,
}

#[must_use]
pub fn product_update_config_for_dir(config_dir: &Path) -> ProductUpdateConfig {
    let config = AppConfig::load_or_default(config_path_in(config_dir)).unwrap_or_default();
    ProductUpdateConfig {
        relays: config.relays,
        blossom_servers: config.blossom_servers,
        embedded_hashtree_base_url: None,
        update_ref: std::env::var("IRIS_DRIVE_UPDATE_HTREE_REF")
            .ok()
            .filter(|value| !value.trim().is_empty()),
    }
}

pub fn check_product_update_blocking(
    current_version: &str,
    mode: ProductUpdateMode,
    config: ProductUpdateConfig,
) -> Result<ProductUpdateResult> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start update runtime")?;
    runtime.block_on(check_product_update(current_version, mode, config))
}

pub async fn check_product_update(
    current_version: &str,
    mode: ProductUpdateMode,
    config: ProductUpdateConfig,
) -> Result<ProductUpdateResult> {
    let selection = secure_selection(current_version, mode, config).await?;
    Ok(product_result_from_selection(
        current_version,
        &selection,
        SECURE_SOURCE_NAME,
        true,
        None,
    ))
}

pub fn download_product_update_blocking(
    current_version: &str,
    mode: ProductUpdateMode,
    config: ProductUpdateConfig,
    download_dir: Option<&Path>,
) -> Result<ProductUpdateResult> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to start update runtime")?;
    runtime.block_on(download_product_update(
        current_version,
        mode,
        config,
        download_dir,
    ))
}

pub async fn download_product_update(
    current_version: &str,
    mode: ProductUpdateMode,
    config: ProductUpdateConfig,
    download_dir: Option<&Path>,
) -> Result<ProductUpdateResult> {
    let selection = secure_selection(current_version, mode, config).await?;
    let destination = download_selection(&selection, download_dir).await?;
    Ok(product_result_from_selection(
        current_version,
        &selection,
        SECURE_SOURCE_NAME,
        true,
        Some(&destination),
    ))
}

async fn secure_selection(
    current_version: &str,
    mode: ProductUpdateMode,
    config: ProductUpdateConfig,
) -> Result<SecureNostrBlossomSelection> {
    let updater = build_secure_updater(&config).await?;
    let reference = update_ref_from_override(
        config.update_ref.as_deref(),
        Some("IRIS_DRIVE_UPDATE_HTREE_REF"),
        HTREE_UPDATE_REF,
    )?;
    select_product_update(updater, reference, current_version, mode, &asset_policy())
        .await
        .with_context(|| {
            format!(
                "failed to resolve signed hashtree release for {}",
                asset_policy().noun(mode)
            )
        })
}

async fn build_secure_updater(config: &ProductUpdateConfig) -> Result<SecureNostrBlossomUpdater> {
    build_secure_nostr_blossom_updater(SecureNostrBlossomConfig {
        relays: update_relays(&config.relays),
        blossom_read_servers: blossom_read_servers(config),
        manifest_timeout: Duration::from_secs(UPDATE_MANIFEST_TIMEOUT_SECS),
        download_timeout: Duration::from_secs(UPDATE_DOWNLOAD_TIMEOUT_SECS),
    })
    .await
    .context("failed to connect to Nostr release relays")
}

fn update_relays(config_relays: &[String]) -> Vec<String> {
    env_csv("IRIS_DRIVE_UPDATE_RELAYS").unwrap_or_else(|| {
        let values = if config_relays.is_empty() {
            DEFAULT_RELAYS
                .iter()
                .map(|value| (*value).to_string())
                .collect()
        } else {
            config_relays.to_vec()
        };
        dedupe_nonempty(values)
    })
}

fn blossom_read_servers(config: &ProductUpdateConfig) -> Vec<String> {
    if let Some(override_servers) = env_csv("IRIS_DRIVE_UPDATE_BLOSSOM_SERVERS") {
        return override_servers;
    }

    let mut servers = Vec::new();
    if let Some(base_url) = config
        .embedded_hashtree_base_url
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        servers.push(base_url.to_string());
    }
    servers.extend(config.blossom_servers.iter().cloned());
    servers.extend(
        DEFAULT_BLOSSOM_SERVERS
            .iter()
            .map(|value| (*value).to_string()),
    );
    servers.extend(
        DEFAULT_UPDATE_BLOSSOM_READ_SERVERS
            .iter()
            .map(|value| (*value).to_string()),
    );
    dedupe_nonempty(servers)
}

async fn download_selection(
    selection: &SecureNostrBlossomSelection,
    download_dir: Option<&Path>,
) -> Result<PathBuf> {
    download_product_selection(selection, download_dir, &asset_policy())
        .await
        .with_context(|| {
            format!(
                "failed to download verified hashtree asset {}",
                selection.asset.name
            )
        })
}

#[must_use]
pub fn preferred_cli_asset(manifest: &UpdateManifest) -> Option<UpdateAsset> {
    preferred_product_asset(manifest, ProductUpdateMode::Cli, &asset_policy())
}

#[must_use]
pub fn preferred_app_asset(manifest: &UpdateManifest) -> Option<UpdateAsset> {
    preferred_product_asset(manifest, ProductUpdateMode::App, &asset_policy())
}

#[must_use]
pub fn current_target() -> &'static str {
    current_archive_target()
}

fn asset_policy() -> ProductAssetPolicy {
    ProductAssetPolicy::new("idrive", "idrive CLI", "Iris Drive app")
        .with_app_asset_suffixes(platform_app_asset_suffixes().iter().copied())
        .with_download_file_name_fallback("iris-drive-update")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_cli_asset_ignores_desktop_artifacts_for_same_target() {
        let manifest = UpdateManifest {
            tag: Some("v1.2.3".to_string()),
            assets: vec![
                UpdateAsset {
                    name: "iris-drive-v1.2.3-linux-x64.deb".to_string(),
                    path: "assets/iris-drive-v1.2.3-linux-x64.deb".to_string(),
                    ..UpdateAsset::default()
                },
                UpdateAsset {
                    name: format!(
                        "idrive-v1.2.3-{}{}",
                        current_target(),
                        hashtree_updater::archive_extension_for_target(current_target())
                    ),
                    path: "assets/idrive.tar.gz".to_string(),
                    ..UpdateAsset::default()
                },
            ],
            ..UpdateManifest::default()
        };

        let asset = preferred_cli_asset(&manifest).expect("idrive CLI asset");

        assert!(asset.name.starts_with("idrive-v1.2.3-"));
    }

    #[test]
    fn preferred_app_asset_uses_current_platform_artifacts_only() {
        let suffixes = platform_app_asset_suffixes();
        if suffixes.is_empty() {
            return;
        }
        let wanted = format!("iris-drive-v1.2.3{}", suffixes[0]);
        let manifest = UpdateManifest {
            tag: Some("v1.2.3".to_string()),
            assets: vec![
                UpdateAsset {
                    name: "idrive-v1.2.3-x86_64-unknown-linux-gnu.tar.gz".to_string(),
                    path: "assets/idrive.tar.gz".to_string(),
                    ..UpdateAsset::default()
                },
                UpdateAsset {
                    name: wanted.clone(),
                    path: format!("assets/{wanted}"),
                    ..UpdateAsset::default()
                },
            ],
            ..UpdateManifest::default()
        };

        let asset = preferred_app_asset(&manifest).expect("app asset");

        assert_eq!(asset.name, wanted);
    }
}
