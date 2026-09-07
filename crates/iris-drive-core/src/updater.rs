use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use hashtree_updater::{
    ProductAssetPolicy, SecureNostrBlossomConfig, SecureNostrBlossomSelection,
    SecureNostrBlossomUpdater, UpdateEventCache, UpdateRef,
    build_secure_nostr_blossom_updater_with_events, current_archive_target, dedupe_nonempty,
    download_product_selection, env_csv, platform_app_asset_suffixes, preferred_product_asset,
    product_result_from_selection, select_product_update, update_ref_from_override,
};
pub use hashtree_updater::{
    ProductUpdateMode, ProductUpdateResult, SECURE_SOURCE_NAME, UpdateAsset, UpdateAutoCheckPolicy,
    UpdateManifest,
};

use crate::config::{AppConfig, DEFAULT_BLOSSOM_SERVERS, DEFAULT_RELAYS};
use crate::paths::config_path_in;
use crate::update_announcement::{load_update_event_cache, persist_update_event_cache};

pub const HTREE_UPDATE_REF: &str = "htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-drive/latest";

const UPDATE_MANIFEST_TIMEOUT_SECS: u64 = 8;
const UPDATE_DOWNLOAD_TIMEOUT_SECS: u64 = 180;
const DEFAULT_UPDATE_BLOSSOM_READ_SERVERS: &[&str] = &[
    "https://cdn.iris.to",
    "https://upload.iris.to",
    "https://blossom.primal.net",
];

#[derive(Clone, Debug, Default)]
pub struct ProductUpdateConfig {
    pub relays: Vec<String>,
    pub blossom_servers: Vec<String>,
    pub embedded_hashtree_base_url: Option<String>,
    pub update_ref: Option<String>,
    pub config_dir: Option<PathBuf>,
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
        config_dir: Some(config_dir.to_path_buf()),
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
    let selection = select_update(current_version, mode, config).await?;
    Ok(result_from_selection(current_version, &selection, None))
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
    let selection = select_update(current_version, mode, config).await?;
    let destination = download_selection(&selection, download_dir).await?;
    Ok(result_from_selection(
        current_version,
        &selection,
        Some(&destination),
    ))
}

async fn select_update(
    current_version: &str,
    mode: ProductUpdateMode,
    config: ProductUpdateConfig,
) -> Result<SecureNostrBlossomSelection> {
    tokio::time::timeout(
        Duration::from_secs(UPDATE_MANIFEST_TIMEOUT_SECS),
        secure_selection(current_version, mode, config),
    )
    .await
    .context("signed hashtree update check timed out")?
    .context("signed hashtree update check failed")
}

fn result_from_selection(
    current_version: &str,
    selection: &SecureNostrBlossomSelection,
    path: Option<&Path>,
) -> ProductUpdateResult {
    product_result_from_selection(current_version, selection, SECURE_SOURCE_NAME, true, path)
}

async fn secure_selection(
    current_version: &str,
    mode: ProductUpdateMode,
    config: ProductUpdateConfig,
) -> Result<SecureNostrBlossomSelection> {
    let reference = product_update_reference(config.update_ref.as_deref())?;
    let updater = build_secure_updater(&config, &reference).await?;
    select_product_update(updater, reference, current_version, mode, &asset_policy())
        .await
        .with_context(|| {
            format!(
                "failed to resolve signed hashtree release for {}",
                asset_policy().noun(mode)
            )
        })
}

pub(crate) fn product_update_reference(override_ref: Option<&str>) -> Result<UpdateRef> {
    update_ref_from_override(
        override_ref,
        Some("IRIS_DRIVE_UPDATE_HTREE_REF"),
        HTREE_UPDATE_REF,
    )
    .map_err(Into::into)
}

async fn build_secure_updater(
    config: &ProductUpdateConfig,
    reference: &UpdateRef,
) -> Result<SecureNostrBlossomUpdater> {
    let relays = update_relays(&config.relays);
    let mut event_cache = match config.config_dir.as_deref() {
        Some(config_dir) => {
            load_update_event_cache(config_dir, reference).unwrap_or_else(|error| {
                tracing::warn!(error, "ignoring invalid cached update announcement");
                UpdateEventCache::new(reference).expect("already validated update reference")
            })
        }
        None => UpdateEventCache::new(reference).context("building update event filter")?,
    };
    let relay_refresh_succeeded = match nostr_pubsub_relay::RelayEventBus::new(
        relays.clone(),
        Duration::from_secs(UPDATE_MANIFEST_TIMEOUT_SECS),
    )
    .await
    {
        Ok(provider) => match event_cache.refresh(&provider).await {
            Ok(advanced) => {
                if advanced && let Some(config_dir) = config.config_dir.as_deref() {
                    persist_update_event_cache(config_dir, &event_cache)
                        .map_err(anyhow::Error::msg)?;
                }
                true
            }
            Err(error) => {
                tracing::warn!(%error, "nostr-pubsub update query failed");
                false
            }
        },
        Err(error) => {
            tracing::warn!(%error, "nostr-pubsub relay provider failed");
            false
        }
    };
    let resolver_relays = if relay_refresh_succeeded {
        Vec::new()
    } else {
        relays
    };
    build_secure_nostr_blossom_updater_with_events(
        SecureNostrBlossomConfig {
            relays: resolver_relays,
            blossom_read_servers: blossom_read_servers(config),
            manifest_timeout: Duration::from_secs(UPDATE_MANIFEST_TIMEOUT_SECS),
            download_timeout: Duration::from_secs(UPDATE_DOWNLOAD_TIMEOUT_SECS),
        },
        event_cache.resolver_events(),
    )
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
    let workspace = create_update_workspace(download_dir)?;
    let destination =
        download_product_selection(selection, Some(workspace.path()), &asset_policy())
            .await
            .with_context(|| {
                format!(
                    "failed to download verified hashtree asset {}",
                    selection.asset.name
                )
            })?;
    let _ = workspace.keep();
    Ok(destination)
}

/// Create a unique, private directory for verified update files.
pub fn create_update_workspace(parent: Option<&Path>) -> Result<tempfile::TempDir> {
    let mut builder = tempfile::Builder::new();
    builder.prefix("iris-drive-update-");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(std::fs::Permissions::from_mode(0o700));
    }
    match parent {
        Some(parent) => {
            std::fs::create_dir_all(parent).context("creating update download directory")?;
            builder.tempdir_in(parent)
        }
        None => builder.tempdir(),
    }
    .context("creating private update workspace")
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

#[must_use]
pub fn version_is_newer(candidate: &str, current: &str) -> bool {
    let left = version_parts(candidate);
    let right = version_parts(current);
    for index in 0..left.len().max(right.len()) {
        let left_value = left.get(index).copied().unwrap_or_default();
        let right_value = right.get(index).copied().unwrap_or_default();
        if left_value != right_value {
            return left_value > right_value;
        }
    }
    false
}

fn version_parts(value: &str) -> Vec<u32> {
    value
        .trim_matches(|ch: char| ch == 'v' || ch == 'V' || ch.is_whitespace())
        .split(|ch: char| !ch.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u32>().unwrap_or_default())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn invalid_signed_reference_fails_without_starting_an_unsigned_update() {
        use futures::FutureExt;

        let config = ProductUpdateConfig {
            update_ref: Some("not-a-signed-update-reference".to_string()),
            ..ProductUpdateConfig::default()
        };
        let check = check_product_update("0.0.0", ProductUpdateMode::Cli, config.clone())
            .now_or_never()
            .expect("an invalid signed reference must fail before any network request");
        assert!(check.is_err());

        let destination = tempfile::tempdir().unwrap();
        let download = download_product_update(
            "0.0.0",
            ProductUpdateMode::Cli,
            config,
            Some(destination.path()),
        )
        .now_or_never()
        .expect("downloads must also reject an invalid signed reference immediately");
        assert!(download.is_err());
        assert_eq!(std::fs::read_dir(destination.path()).unwrap().count(), 0);
    }

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

    #[test]
    fn compares_semver_like_update_tags() {
        assert!(version_is_newer("v0.2.28", "0.2.27"));
        assert!(!version_is_newer("v0.2.27", "0.2.27"));
        assert!(!version_is_newer("v0.2.26", "0.2.27"));
    }
}
