use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use hashtree_blossom::{BlossomClient, BlossomStore};
use hashtree_core::{HashTree, HashTreeConfig};
use hashtree_resolver::{
    Keys as HashtreeResolverKeys,
    nostr::{NostrResolverConfig, NostrRootResolver},
};
use hashtree_updater::{
    DownloadOptions, HashtreeUpdater, UpdateAsset, UpdateCheck, UpdateCheckOptions, UpdateManifest,
    UpdateRef, UpdateTarget,
};
use serde::{Deserialize, Serialize};

use crate::config::{AppConfig, DEFAULT_BLOSSOM_SERVERS, DEFAULT_RELAYS};
use crate::paths::config_path_in;

pub const HTREE_UPDATE_REF: &str = "htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-drive/latest";
pub const SECURE_SOURCE_NAME: &str = "hashtree-nostr-blossom";

const UPDATE_MANIFEST_TIMEOUT_SECS: u64 = 8;
const UPDATE_DOWNLOAD_TIMEOUT_SECS: u64 = 180;
const DEFAULT_UPDATE_BLOSSOM_READ_SERVERS: &[&str] = &[
    "https://cdn.iris.to",
    "https://hashtree.iris.to",
    "https://upload.iris.to",
    "https://blossom.primal.net",
];

type SecureUpdater = HashtreeUpdater<NostrRootResolver, BlossomStore>;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProductUpdateMode {
    Cli,
    App,
}

impl ProductUpdateMode {
    #[must_use]
    pub fn noun(self) -> &'static str {
        match self {
            Self::Cli => "idrive CLI",
            Self::App => "Iris Drive app",
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ProductUpdateConfig {
    pub relays: Vec<String>,
    pub blossom_servers: Vec<String>,
    pub embedded_hashtree_base_url: Option<String>,
    pub update_ref: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProductUpdateResult {
    pub available: bool,
    pub current_version: String,
    pub latest_version: String,
    pub tag: String,
    pub asset: String,
    pub source: String,
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root_cid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_cid: Option<String>,
}

struct SecureSelection {
    updater: SecureUpdater,
    check: UpdateCheck,
    asset: UpdateAsset,
    tag: String,
    update_available: bool,
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
    let selection = secure_selection(current_version, mode, config).await?;
    let destination = download_selection(&selection, download_dir).await?;
    Ok(result_from_selection(
        current_version,
        &selection,
        Some(&destination),
    ))
}

async fn secure_selection(
    current_version: &str,
    mode: ProductUpdateMode,
    config: ProductUpdateConfig,
) -> Result<SecureSelection> {
    let updater = build_secure_updater(&config).await?;
    let mut check = updater
        .check(UpdateCheckOptions {
            reference: secure_update_ref(config.update_ref.as_deref())?,
            current_version: current_version.to_string(),
            target: UpdateTarget::new(current_target()),
            ..UpdateCheckOptions::default()
        })
        .await
        .context("failed to resolve signed hashtree release")?;
    let asset = preferred_asset(&check.manifest, mode).ok_or_else(|| {
        anyhow!(
            "release {} has no {} asset for {}",
            display_manifest_tag(&check.manifest),
            mode.noun(),
            current_target()
        )
    })?;
    check.asset = Some(asset.clone());
    let tag = display_manifest_tag(&check.manifest);
    let update_available = check.update_available;
    Ok(SecureSelection {
        updater,
        check,
        asset,
        tag,
        update_available,
    })
}

async fn build_secure_updater(config: &ProductUpdateConfig) -> Result<SecureUpdater> {
    let resolver = NostrRootResolver::new(NostrResolverConfig {
        relays: update_relays(&config.relays),
        resolve_timeout: Duration::from_secs(UPDATE_MANIFEST_TIMEOUT_SECS),
        secret_key: None,
    })
    .await
    .context("failed to connect to Nostr release relays")?;
    let blossom = BlossomClient::new_empty(HashtreeResolverKeys::generate())
        .with_read_servers(blossom_read_servers(config))
        .with_timeout(Duration::from_secs(UPDATE_DOWNLOAD_TIMEOUT_SECS));
    let store = Arc::new(BlossomStore::new(blossom));
    let tree = HashTree::new(HashTreeConfig::new(store).public());
    Ok(HashtreeUpdater::new(resolver, tree))
}

fn secure_update_ref(override_ref: Option<&str>) -> Result<UpdateRef> {
    let env_ref = std::env::var("IRIS_DRIVE_UPDATE_HTREE_REF")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let raw = override_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or(env_ref.as_deref())
        .unwrap_or(HTREE_UPDATE_REF);
    UpdateRef::parse(raw).with_context(|| format!("invalid update hashtree ref: {raw}"))
}

fn update_relays(config_relays: &[String]) -> Vec<String> {
    split_env_csv("IRIS_DRIVE_UPDATE_RELAYS").unwrap_or_else(|| {
        let values = if config_relays.is_empty() {
            DEFAULT_RELAYS
                .iter()
                .map(|value| (*value).to_string())
                .collect()
        } else {
            config_relays.to_vec()
        };
        dedupe(values)
    })
}

fn blossom_read_servers(config: &ProductUpdateConfig) -> Vec<String> {
    if let Some(override_servers) = split_env_csv("IRIS_DRIVE_UPDATE_BLOSSOM_SERVERS") {
        return dedupe(override_servers);
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
    dedupe(servers)
}

fn split_env_csv(name: &str) -> Option<Vec<String>> {
    let values = std::env::var(name)
        .ok()?
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .collect::<Vec<_>>();
    (!values.is_empty()).then_some(values)
}

fn dedupe(values: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    for value in values {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out.iter().any(|existing| existing == trimmed) {
            out.push(trimmed.to_string());
        }
    }
    out
}

fn result_from_selection(
    current_version: &str,
    selection: &SecureSelection,
    path: Option<&Path>,
) -> ProductUpdateResult {
    ProductUpdateResult {
        available: selection.update_available,
        current_version: current_version.to_string(),
        latest_version: selection.tag.trim_start_matches('v').to_string(),
        tag: selection.tag.clone(),
        asset: selection.asset.name.clone(),
        source: SECURE_SOURCE_NAME.to_string(),
        verified: true,
        path: path.map(|value| value.display().to_string()),
        root_cid: Some(selection.check.root_cid.to_string()),
        release_cid: Some(selection.check.release_cid.to_string()),
    }
}

async fn download_selection(
    selection: &SecureSelection,
    download_dir: Option<&Path>,
) -> Result<PathBuf> {
    let destination = selected_download_path(download_dir, &selection.asset.name)?;
    let downloaded = selection
        .updater
        .download(&selection.check, DownloadOptions::default(), None)
        .await
        .with_context(|| {
            format!(
                "failed to download verified hashtree asset {}",
                selection.asset.name
            )
        })?;
    write_downloaded_asset(&destination, &downloaded.bytes)?;
    Ok(destination)
}

fn preferred_asset(manifest: &UpdateManifest, mode: ProductUpdateMode) -> Option<UpdateAsset> {
    match mode {
        ProductUpdateMode::Cli => preferred_cli_asset(manifest),
        ProductUpdateMode::App => preferred_app_asset(manifest),
    }
}

pub fn preferred_cli_asset(manifest: &UpdateManifest) -> Option<UpdateAsset> {
    let tag = display_manifest_tag(manifest);
    let target = current_target();
    let archive_ext = archive_extension_for_target(target);
    let exact = format!("idrive-{tag}-{target}{archive_ext}");
    let unversioned = format!("idrive-{target}{archive_ext}");
    let update_target = UpdateTarget::new(target);

    manifest
        .assets
        .iter()
        .find(|asset| asset.name == exact)
        .or_else(|| {
            manifest
                .assets
                .iter()
                .find(|asset| asset.name == unversioned)
        })
        .or_else(|| {
            manifest.assets.iter().find(|asset| {
                asset.name.starts_with("idrive-")
                    && asset.name.ends_with(archive_ext)
                    && asset.matches_target_with_inference(&update_target)
            })
        })
        .cloned()
}

pub fn preferred_app_asset(manifest: &UpdateManifest) -> Option<UpdateAsset> {
    let preferred_suffixes = app_asset_suffixes_for_current_target();
    if preferred_suffixes.is_empty() {
        return None;
    }
    manifest
        .assets
        .iter()
        .find(|asset| {
            let lower = asset.name.to_ascii_lowercase();
            preferred_suffixes
                .iter()
                .any(|suffix| lower.ends_with(suffix))
        })
        .cloned()
}

fn archive_extension_for_target(target: &str) -> &'static str {
    if target.contains("windows") {
        ".zip"
    } else {
        ".tar.gz"
    }
}

fn app_asset_suffixes_for_current_target() -> &'static [&'static str] {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        &["-macos-arm64.app.tar.gz", "-macos-arm64.dmg"]
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        &["-linux-x64.appimage", "-linux-x64.deb"]
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        &["-linux-arm64.appimage", "-linux-arm64.deb"]
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        &["-windows-x64-setup.exe"]
    }
    #[cfg(all(target_os = "android", target_arch = "aarch64"))]
    {
        &["-android-arm64.apk"]
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "windows", target_arch = "x86_64"),
        all(target_os = "android", target_arch = "aarch64"),
    )))]
    {
        &[]
    }
}

fn display_manifest_tag(manifest: &UpdateManifest) -> String {
    manifest
        .tag
        .clone()
        .filter(|tag| !tag.trim().is_empty())
        .unwrap_or_else(|| format!("v{}", manifest.effective_version()))
}

#[must_use]
pub fn current_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("android", "aarch64") => "aarch64-linux-android",
        _ => "unsupported",
    }
}

fn selected_download_path(download_dir: Option<&Path>, asset_name: &str) -> Result<PathBuf> {
    let file_name = safe_file_name(asset_name);
    let parent = download_dir
        .map(Path::to_path_buf)
        .unwrap_or_else(std::env::temp_dir);
    fs::create_dir_all(&parent).with_context(|| format!("creating {}", parent.display()))?;
    Ok(parent.join(file_name))
}

fn write_downloaded_asset(destination: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(destination, bytes)
        .with_context(|| format!("writing verified update to {}", destination.display()))
}

fn safe_file_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "iris-drive-update".to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use hashtree_updater::{UpdateAsset, UpdateManifest};

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
                        archive_extension_for_target(current_target())
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
        let suffixes = app_asset_suffixes_for_current_target();
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
