use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use hashtree_updater::{
    ProductAssetPolicy, SecureNostrBlossomConfig, SecureNostrBlossomSelection,
    SecureNostrBlossomUpdater, build_secure_nostr_blossom_updater, current_archive_target,
    dedupe_nonempty, download_product_selection, env_csv, platform_app_asset_suffixes,
    preferred_product_asset, product_result_from_selection, select_product_update,
    selected_download_path as shared_selected_download_path, update_ref_from_override,
};
pub use hashtree_updater::{
    ProductUpdateMode, ProductUpdateResult, SECURE_SOURCE_NAME, UpdateAsset, UpdateAutoCheckPolicy,
    UpdateManifest,
};

use crate::config::{AppConfig, DEFAULT_BLOSSOM_SERVERS, DEFAULT_RELAYS};
use crate::paths::config_path_in;

pub const HTREE_UPDATE_REF: &str = "htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-drive/latest";
pub const HTREE_MANIFEST_URL: &str = "https://upload.iris.to/npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-drive/latest/release.json";
pub const LEGACY_HTREE_SOURCE_NAME: &str = "legacy-htree-url";

const UPDATE_CONNECT_TIMEOUT_SECS: u64 = 4;
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

struct LegacySelection {
    manifest: UpdateManifest,
    asset: UpdateAsset,
    asset_url: String,
    update_available: bool,
}

enum UpdateSelection {
    Secure(Box<SecureNostrBlossomSelection>),
    Legacy(LegacySelection),
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
) -> Result<UpdateSelection> {
    let secure = tokio::time::timeout(
        Duration::from_secs(UPDATE_MANIFEST_TIMEOUT_SECS),
        secure_selection(current_version, mode, config),
    )
    .await;

    let selection = match secure {
        Ok(Ok(selection)) => selection,
        Ok(Err(error)) => {
            return legacy_selection(current_version, mode)
                .await
                .map(UpdateSelection::Legacy)
                .with_context(|| format!("secure hashtree update check failed: {error}"));
        }
        Err(_) => {
            return legacy_selection(current_version, mode)
                .await
                .map(UpdateSelection::Legacy)
                .context("secure hashtree update check timed out");
        }
    };

    if !selection.update_available
        && let Ok(legacy) = legacy_selection(current_version, mode).await
        && legacy.update_available
    {
        return Ok(UpdateSelection::Legacy(legacy));
    }

    Ok(UpdateSelection::Secure(Box::new(selection)))
}

fn result_from_selection(
    current_version: &str,
    selection: &UpdateSelection,
    path: Option<&Path>,
) -> ProductUpdateResult {
    match selection {
        UpdateSelection::Secure(selection) => product_result_from_selection(
            current_version,
            selection.as_ref(),
            SECURE_SOURCE_NAME,
            true,
            path,
        ),
        UpdateSelection::Legacy(selection) => ProductUpdateResult {
            available: selection.update_available,
            current_version: current_version.to_string(),
            latest_version: selection.manifest.effective_version(),
            tag: manifest_display_tag(&selection.manifest),
            asset: selection.asset.name.clone(),
            source: LEGACY_HTREE_SOURCE_NAME.to_string(),
            verified: false,
            path: path.map(|value| value.display().to_string()),
            ..ProductUpdateResult::default()
        },
    }
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

async fn legacy_selection(
    current_version: &str,
    mode: ProductUpdateMode,
) -> Result<LegacySelection> {
    let manifest = fetch_manifest(HTREE_MANIFEST_URL).await?;
    let asset = preferred_product_asset(&manifest, mode, &asset_policy()).ok_or_else(|| {
        anyhow::anyhow!(
            "release {} has no {} asset for {}",
            manifest_display_tag(&manifest),
            asset_policy().noun(mode),
            current_target()
        )
    })?;
    let update_available = version_is_newer(&manifest_display_tag(&manifest), current_version);
    let asset_url = manifest_asset_url(HTREE_MANIFEST_URL, &asset.path);
    Ok(LegacySelection {
        manifest,
        asset,
        asset_url,
        update_available,
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
    selection: &UpdateSelection,
    download_dir: Option<&Path>,
) -> Result<PathBuf> {
    match selection {
        UpdateSelection::Secure(selection) => {
            download_product_selection(selection.as_ref(), download_dir, &asset_policy())
                .await
                .with_context(|| {
                    format!(
                        "failed to download verified hashtree asset {}",
                        selection.asset.name
                    )
                })
        }
        UpdateSelection::Legacy(selection) => {
            let destination = shared_selected_download_path(
                download_dir,
                &selection.asset.name,
                asset_policy().download_file_name_fallback(),
            )
            .with_context(|| {
                format!(
                    "failed to choose update download path for {}",
                    selection.asset.name
                )
            })?;
            download_asset(&selection.asset_url, &destination).await?;
            Ok(destination)
        }
    }
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

async fn fetch_manifest(url: &str) -> Result<UpdateManifest> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(UPDATE_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(UPDATE_MANIFEST_TIMEOUT_SECS))
        .build()
        .context("building update HTTP client")?;
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("fetching update manifest {url}"))?
        .error_for_status()
        .with_context(|| format!("fetching update manifest {url}"))?;
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("reading update manifest {url}"))?;
    serde_json::from_slice(&bytes).context("parsing update manifest")
}

async fn download_asset(url: &str, destination: &Path) -> Result<()> {
    if let Some(parent) = destination.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(UPDATE_CONNECT_TIMEOUT_SECS))
        .timeout(Duration::from_secs(UPDATE_DOWNLOAD_TIMEOUT_SECS))
        .build()
        .context("building update HTTP client")?;
    let response = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("downloading update asset {url}"))?
        .error_for_status()
        .with_context(|| format!("downloading update asset {url}"))?;
    let bytes = response
        .bytes()
        .await
        .with_context(|| format!("reading update asset {url}"))?;
    tokio::fs::write(destination, bytes)
        .await
        .with_context(|| format!("writing {}", destination.display()))?;
    Ok(())
}

fn manifest_display_tag(manifest: &UpdateManifest) -> String {
    if let Some(tag) = manifest
        .tag
        .as_deref()
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
    {
        return tag.to_string();
    }
    let version = manifest.effective_version();
    if version.is_empty() {
        String::new()
    } else {
        format!("v{version}")
    }
}

fn manifest_asset_url(manifest_url: &str, path: &str) -> String {
    if path.starts_with("http://") || path.starts_with("https://") || path.starts_with("file://") {
        return path.to_string();
    }
    let base = manifest_url
        .rsplit_once('/')
        .map(|(base, _)| base)
        .unwrap_or(manifest_url);
    format!("{}/{}", base, path.trim_start_matches('/'))
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

    #[test]
    fn resolves_relative_legacy_manifest_asset_urls() {
        assert_eq!(
            manifest_asset_url(
                "https://example.invalid/releases/iris-drive/latest/release.json",
                "assets/iris-drive.dmg",
            ),
            "https://example.invalid/releases/iris-drive/latest/assets/iris-drive.dmg"
        );
    }
}
