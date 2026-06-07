use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow};
use hashtree_blossom::{BlossomClient, BlossomStore};
use hashtree_core::{HashTree, HashTreeConfig};
use hashtree_resolver::nostr::{NostrResolverConfig, NostrRootResolver};
use hashtree_updater::{
    DownloadOptions, HashtreeUpdater, UpdateAsset, UpdateCheckOptions, UpdateManifest, UpdateRef,
    UpdateTarget,
};
use iris_drive_core::config::{AppConfig, DEFAULT_BLOSSOM_SERVERS, DEFAULT_RELAYS};
use iris_drive_core::updater::preferred_app_asset;
use serde::Serialize;
use serde_json::Value;

use super::{UpdateArgs, config_path_in, load_daemon_status};

const PRODUCT_VERSION: &str = env!("CARGO_PKG_VERSION");
const DEFAULT_UPDATE_REF: &str = "htree://npub1xdhnr9mrv47kkrn95k6cwecearydeh8e895990n3acntwvmgk2dsdeeycm/releases%2Firis-drive/latest";
const UPDATE_MANIFEST_TIMEOUT_SECS: u64 = 8;
const UPDATE_DOWNLOAD_TIMEOUT_SECS: u64 = 180;
const SECURE_SOURCE_NAME: &str = "hashtree-nostr-blossom";
const DEFAULT_UPDATE_BLOSSOM_READ_SERVERS: &[&str] = &[
    "https://cdn.iris.to",
    "https://hashtree.iris.to",
    "https://upload.iris.to",
    "https://blossom.primal.net",
];

type SecureUpdater = HashtreeUpdater<NostrRootResolver, BlossomStore>;

#[derive(Debug, Serialize)]
struct UpdateJson {
    available: bool,
    current_version: String,
    latest_version: String,
    tag: String,
    asset: String,
    source: &'static str,
    verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    root_cid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    release_cid: Option<String>,
}

pub(crate) fn cmd_update(config_dir: &Path, args: UpdateArgs) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building update runtime")?;
    let config = AppConfig::load_or_default(config_path_in(config_dir))
        .with_context(|| format!("reading {}", config_path_in(config_dir).display()))?;
    let daemon_status = load_daemon_status(config_dir);
    runtime.block_on(run_update(&config, daemon_status.as_ref(), args))
}

async fn run_update(
    config: &AppConfig,
    daemon_status: Option<&Value>,
    args: UpdateArgs,
) -> Result<()> {
    let updater = build_secure_updater(config, daemon_status).await?;
    let reference = update_ref(args.reference.as_deref())?;
    let mut check = updater
        .check(UpdateCheckOptions {
            reference,
            current_version: PRODUCT_VERSION.to_string(),
            target: UpdateTarget::new(current_target()),
            ..UpdateCheckOptions::default()
        })
        .await
        .context("resolving signed hashtree release")?;
    let noun = if args.app {
        "Iris Drive app"
    } else {
        "idrive CLI"
    };
    let asset = if args.app {
        preferred_app_asset(&check.manifest)
    } else {
        preferred_cli_asset_for_target(&check.manifest, current_target())
    }
    .ok_or_else(|| {
        anyhow!(
            "release {} has no {noun} asset for {}",
            display_manifest_tag(&check.manifest),
            current_target()
        )
    })?;
    check.asset = Some(asset.clone());
    let tag = display_manifest_tag(&check.manifest);
    let available = check.update_available;

    if args.check {
        print_update_check(&check, &asset, available, &tag, args.json, None)?;
        return Ok(());
    }

    if !available && !args.force {
        print_up_to_date(&check, &tag, args.json)?;
        return Ok(());
    }

    let temp_dir = create_temp_dir("idrive-update")?;
    let destination = selected_download_path(args.download_dir.as_deref(), &asset.name, &temp_dir)?;
    let downloaded = updater
        .download(&check, DownloadOptions::default(), None)
        .await
        .with_context(|| format!("downloading verified hashtree asset {}", asset.name))?;
    write_downloaded_asset(&destination, &downloaded.bytes)?;

    if args.download_only || args.app {
        print_downloaded(&check, &asset, available, &tag, &destination, args.json)?;
        return Ok(());
    }

    install_cli_archive(&destination, &temp_dir, args.path.as_deref())?;
    let _ = fs::remove_dir_all(&temp_dir);
    println!(
        "updated idrive at {} from {PRODUCT_VERSION} to {tag}",
        args.path
            .as_deref()
            .map_or_else(current_exe_display, |path| path.display().to_string())
    );
    Ok(())
}

async fn build_secure_updater(
    config: &AppConfig,
    daemon_status: Option<&Value>,
) -> Result<SecureUpdater> {
    let resolver = NostrRootResolver::new(NostrResolverConfig {
        relays: update_relays(config),
        resolve_timeout: Duration::from_secs(UPDATE_MANIFEST_TIMEOUT_SECS),
        secret_key: None,
    })
    .await
    .context("connecting to Nostr release relays")?;
    let blossom = BlossomClient::new_empty(nostr::Keys::generate())
        .with_read_servers(blossom_read_servers_for(
            daemon_status,
            &config.blossom_servers,
            split_env_csv("IRIS_DRIVE_UPDATE_BLOSSOM_SERVERS"),
        ))
        .with_timeout(Duration::from_secs(UPDATE_DOWNLOAD_TIMEOUT_SECS));
    let store = Arc::new(BlossomStore::new(blossom));
    let tree = HashTree::new(HashTreeConfig::new(store).public());
    Ok(HashtreeUpdater::new(resolver, tree))
}

fn update_ref(override_ref: Option<&str>) -> Result<UpdateRef> {
    let raw = override_ref
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(DEFAULT_UPDATE_REF);
    UpdateRef::parse(raw).with_context(|| format!("invalid update hashtree ref: {raw}"))
}

fn update_relays(config: &AppConfig) -> Vec<String> {
    split_env_csv("IRIS_DRIVE_UPDATE_RELAYS").unwrap_or_else(|| {
        let values = if config.relays.is_empty() {
            DEFAULT_RELAYS
                .iter()
                .map(|value| (*value).to_string())
                .collect()
        } else {
            config.relays.clone()
        };
        dedupe(values)
    })
}

fn blossom_read_servers_for(
    daemon_status: Option<&Value>,
    config_servers: &[String],
    env_override: Option<Vec<String>>,
) -> Vec<String> {
    if let Some(override_servers) = env_override {
        return dedupe(override_servers);
    }

    let mut servers = Vec::new();
    if let Some(base_url) = embedded_hashtree_base_url(daemon_status) {
        servers.push(base_url.to_string());
    }
    servers.extend(config_servers.iter().cloned());
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

fn embedded_hashtree_base_url(status: Option<&Value>) -> Option<&str> {
    let status = status?;
    if !status
        .get("fresh")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return None;
    }
    status
        .get("embedded_hashtree")
        .and_then(|embedded| embedded.get("base_url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
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

fn preferred_cli_asset_for_target(manifest: &UpdateManifest, target: &str) -> Option<UpdateAsset> {
    let tag = display_manifest_tag(manifest);
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

fn archive_extension_for_target(target: &str) -> &'static str {
    if target.contains("windows") {
        ".zip"
    } else {
        ".tar.gz"
    }
}

fn display_manifest_tag(manifest: &UpdateManifest) -> String {
    manifest
        .tag
        .clone()
        .filter(|tag| !tag.trim().is_empty())
        .unwrap_or_else(|| format!("v{}", manifest.effective_version()))
}

fn current_target() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => "unsupported",
    }
}

fn print_update_check(
    check: &hashtree_updater::UpdateCheck,
    asset: &UpdateAsset,
    available: bool,
    tag: &str,
    json_output: bool,
    path: Option<&Path>,
) -> Result<()> {
    if json_output {
        print_update_json(&UpdateJson {
            available,
            current_version: PRODUCT_VERSION.to_string(),
            latest_version: tag.trim_start_matches('v').to_string(),
            tag: tag.to_string(),
            asset: asset.name.clone(),
            source: SECURE_SOURCE_NAME,
            verified: true,
            path: path.map(|value| value.display().to_string()),
            root_cid: Some(check.root_cid.to_string()),
            release_cid: Some(check.release_cid.to_string()),
        })?;
        return Ok(());
    }

    if available {
        println!("update available: {PRODUCT_VERSION} -> {tag}");
    } else {
        println!("idrive {PRODUCT_VERSION} is up to date");
    }
    println!("asset={}", asset.name);
    println!("source={SECURE_SOURCE_NAME}");
    println!("verified=true");
    println!("root_cid={}", check.root_cid);
    println!("release_cid={}", check.release_cid);
    if let Some(path) = path {
        println!("path={}", path.display());
    }
    Ok(())
}

fn print_downloaded(
    check: &hashtree_updater::UpdateCheck,
    asset: &UpdateAsset,
    available: bool,
    tag: &str,
    path: &Path,
    json_output: bool,
) -> Result<()> {
    if json_output {
        print_update_json(&UpdateJson {
            available,
            current_version: PRODUCT_VERSION.to_string(),
            latest_version: tag.trim_start_matches('v').to_string(),
            tag: tag.to_string(),
            asset: asset.name.clone(),
            source: SECURE_SOURCE_NAME,
            verified: true,
            path: Some(path.display().to_string()),
            root_cid: Some(check.root_cid.to_string()),
            release_cid: Some(check.release_cid.to_string()),
        })?;
        return Ok(());
    }
    println!("downloaded {}", asset.name);
    println!("path={}", path.display());
    println!("source={SECURE_SOURCE_NAME}");
    println!("verified=true");
    Ok(())
}

fn print_up_to_date(
    check: &hashtree_updater::UpdateCheck,
    tag: &str,
    json_output: bool,
) -> Result<()> {
    if json_output {
        print_update_json(&UpdateJson {
            available: false,
            current_version: PRODUCT_VERSION.to_string(),
            latest_version: tag.trim_start_matches('v').to_string(),
            tag: tag.to_string(),
            asset: String::new(),
            source: SECURE_SOURCE_NAME,
            verified: true,
            path: None,
            root_cid: Some(check.root_cid.to_string()),
            release_cid: Some(check.release_cid.to_string()),
        })?;
        return Ok(());
    }
    println!("idrive {PRODUCT_VERSION} is up to date");
    Ok(())
}

fn print_update_json(output: &UpdateJson) -> Result<()> {
    println!("{}", serde_json::to_string(output)?);
    Ok(())
}

fn selected_download_path(
    download_dir: Option<&Path>,
    asset_name: &str,
    temp_dir: &Path,
) -> Result<PathBuf> {
    let file_name = safe_file_name(asset_name);
    let parent = download_dir.unwrap_or(temp_dir);
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    Ok(parent.join(file_name))
}

fn write_downloaded_asset(destination: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    }
    fs::write(destination, bytes)
        .with_context(|| format!("writing verified update to {}", destination.display()))
}

fn install_cli_archive(
    archive_path: &Path,
    temp_dir: &Path,
    destination: Option<&Path>,
) -> Result<()> {
    extract_archive(archive_path, temp_dir)?;
    let binary = find_idrive_binary(temp_dir)?;
    let destination = destination.map(Path::to_path_buf).map_or_else(
        || std::env::current_exe().context("resolving current executable"),
        Ok,
    )?;
    install_binary(&binary, &destination)
}

fn extract_archive(archive_path: &Path, destination: &Path) -> Result<()> {
    let mut command = Command::new("tar");
    if archive_path
        .file_name()
        .and_then(|value| value.to_str())
        .is_some_and(is_compressed_tar_name)
    {
        command.arg("-xzf");
    } else {
        command.arg("-xf");
    }
    let output = command
        .arg(archive_path)
        .arg("-C")
        .arg(destination)
        .output()
        .with_context(|| format!("extracting {}", archive_path.display()))?;
    if !output.status.success() {
        return Err(anyhow!(
            "{}",
            command_error("archive extraction failed", &output)
        ));
    }
    Ok(())
}

fn is_compressed_tar_name(name: &str) -> bool {
    name.get(name.len().saturating_sub(".tar.gz".len())..)
        .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".tar.gz"))
        || Path::new(name)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("tgz"))
}

fn find_idrive_binary(root: &Path) -> Result<PathBuf> {
    let binary_name = if cfg!(target_os = "windows") {
        "idrive.exe"
    } else {
        "idrive"
    };
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path).with_context(|| format!("reading {}", path.display()))? {
            let entry = entry?;
            let path = entry.path();
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file()
                && path.file_name().and_then(|value| value.to_str()) == Some(binary_name)
            {
                return Ok(path);
            }
        }
    }
    Err(anyhow!("downloaded archive did not contain {binary_name}"))
}

fn install_binary(source: &Path, destination: &Path) -> Result<()> {
    let parent = install_parent(destination)?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let temp_path = parent.join(format!(
        ".idrive-update-{}-{}{}",
        std::process::id(),
        unix_timestamp(),
        std::env::consts::EXE_SUFFIX
    ));
    if temp_path.exists() {
        let _ = fs::remove_file(&temp_path);
    }
    fs::copy(source, &temp_path)
        .with_context(|| format!("copying {} to {}", source.display(), temp_path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(0o755))
            .with_context(|| format!("marking {} executable", temp_path.display()))?;
    }
    #[cfg(target_os = "windows")]
    if destination.exists() {
        fs::remove_file(destination)
            .with_context(|| format!("removing {}", destination.display()))?;
    }
    fs::rename(&temp_path, destination).with_context(|| {
        format!(
            "moving {} into {}",
            temp_path.display(),
            destination.display()
        )
    })
}

fn install_parent(destination: &Path) -> Result<&Path> {
    if destination.as_os_str().is_empty() {
        return Err(anyhow!("install path must not be empty"));
    }
    if destination.is_dir() {
        return Err(anyhow!(
            "install path points to a directory: {}",
            destination.display()
        ));
    }
    destination.parent().ok_or_else(|| {
        anyhow!(
            "install path must include parent directory: {}",
            destination.display()
        )
    })
}

fn current_exe_display() -> String {
    std::env::current_exe().map_or_else(
        |_| "<current executable>".to_string(),
        |path| path.display().to_string(),
    )
}

fn create_temp_dir(prefix: &str) -> Result<PathBuf> {
    let dir = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}",
        std::process::id(),
        unix_timestamp()
    ));
    if dir.exists() {
        let _ = fs::remove_dir_all(&dir);
    }
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    Ok(dir)
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
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
        "idrive-update".to_string()
    } else {
        out
    }
}

fn command_error(context: &str, output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let detail = stderr.trim();
    if detail.is_empty() {
        format!("{context}: {}", stdout.trim())
    } else {
        format!("{context}: {detail}")
    }
}

#[cfg(test)]
mod tests {
    use hashtree_updater::{UpdateAsset, UpdateManifest};
    use serde_json::json;

    use super::*;

    #[test]
    fn preferred_cli_asset_ignores_desktop_artifacts_for_same_target() {
        let manifest = UpdateManifest {
            tag: Some("v1.2.3".to_string()),
            assets: vec![
                UpdateAsset {
                    name: "iris-drive-v1.2.3-linux-x64.AppImage".to_string(),
                    path: "assets/iris-drive-v1.2.3-linux-x64.AppImage".to_string(),
                    ..UpdateAsset::default()
                },
                UpdateAsset {
                    name: "idrive-v1.2.3-x86_64-unknown-linux-musl.tar.gz".to_string(),
                    path: "assets/idrive-v1.2.3-x86_64-unknown-linux-musl.tar.gz".to_string(),
                    ..UpdateAsset::default()
                },
            ],
            ..UpdateManifest::default()
        };

        let asset = preferred_cli_asset_for_target(&manifest, "x86_64-unknown-linux-musl")
            .expect("idrive CLI asset");

        assert_eq!(asset.name, "idrive-v1.2.3-x86_64-unknown-linux-musl.tar.gz");
    }

    #[test]
    fn blossom_servers_prefer_running_embedded_hashtree_then_config_then_defaults() {
        let status = json!({
            "fresh": true,
            "embedded_hashtree": {
                "base_url": "http://127.0.0.1:18432"
            }
        });
        let servers = blossom_read_servers_for(
            Some(&status),
            &[
                "https://upload.iris.to".to_string(),
                "https://backup.example".to_string(),
            ],
            None,
        );

        assert_eq!(servers[0], "http://127.0.0.1:18432");
        assert_eq!(
            servers
                .iter()
                .filter(|server| *server == "https://upload.iris.to")
                .count(),
            1
        );
        assert!(servers.contains(&"https://backup.example".to_string()));
        assert!(servers.contains(&"https://cdn.iris.to".to_string()));
    }
}
