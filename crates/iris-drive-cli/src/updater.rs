use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};
use iris_drive_core::config::AppConfig;
use iris_drive_core::updater::{
    ProductUpdateConfig, ProductUpdateMode, ProductUpdateResult, check_product_update,
    create_update_workspace, download_product_update,
};
use serde_json::Value;

use super::{UpdateArgs, config_path_in, load_daemon_status};

const PRODUCT_VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn cmd_update(config_dir: &Path, args: UpdateArgs) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building update runtime")?;
    let config = AppConfig::load_or_default(config_path_in(config_dir))
        .with_context(|| format!("reading {}", config_path_in(config_dir).display()))?;
    let daemon_status = load_daemon_status(config_dir);
    runtime.block_on(run_update(
        config_dir,
        &config,
        daemon_status.as_ref(),
        args,
    ))
}

async fn run_update(
    config_dir: &Path,
    config: &AppConfig,
    daemon_status: Option<&Value>,
    args: UpdateArgs,
) -> Result<()> {
    let mode = if args.app {
        ProductUpdateMode::App
    } else {
        ProductUpdateMode::Cli
    };
    let update_config =
        product_update_config(config_dir, config, daemon_status, args.reference.as_deref());
    let check = check_product_update(PRODUCT_VERSION, mode, update_config.clone())
        .await
        .context("resolving signed hashtree release")?;

    if args.check {
        print_update_check(&check, args.json)?;
        return Ok(());
    }

    if !check.available && !args.force {
        print_up_to_date(&check, args.json)?;
        return Ok(());
    }

    let temp_dir = create_update_workspace(None)?;
    let download_dir = args.download_dir.as_deref().unwrap_or(temp_dir.path());
    let downloaded =
        download_product_update(PRODUCT_VERSION, mode, update_config, Some(download_dir))
            .await
            .with_context(|| format!("downloading verified hashtree asset {}", check.asset))?;
    let destination = downloaded
        .path
        .as_deref()
        .map(PathBuf::from)
        .ok_or_else(|| anyhow!("downloaded update did not report a path"))?;

    if args.download_only || args.app {
        print_downloaded(&downloaded, args.json)?;
        if args.download_dir.is_none() {
            // The returned archive remains available for the app or caller to install.
            let _ = temp_dir.keep();
        }
        return Ok(());
    }

    install_cli_archive(&destination, temp_dir.path(), args.path.as_deref())?;
    println!(
        "updated idrive at {} from {PRODUCT_VERSION} to {}",
        args.path
            .as_deref()
            .map_or_else(current_exe_display, |path| path.display().to_string()),
        downloaded.tag
    );
    Ok(())
}

fn product_update_config(
    config_dir: &Path,
    config: &AppConfig,
    daemon_status: Option<&Value>,
    reference: Option<&str>,
) -> ProductUpdateConfig {
    ProductUpdateConfig {
        relays: config.relays.clone(),
        blossom_servers: config.blossom_servers.clone(),
        embedded_hashtree_base_url: embedded_hashtree_base_url(daemon_status).map(str::to_string),
        update_ref: reference.map(str::to_string),
        config_dir: Some(config_dir.to_path_buf()),
    }
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

fn print_update_check(result: &ProductUpdateResult, json_output: bool) -> Result<()> {
    if json_output {
        print_update_json(result)?;
        return Ok(());
    }

    if result.available {
        println!("update available: {PRODUCT_VERSION} -> {}", result.tag);
    } else {
        println!("idrive {PRODUCT_VERSION} is up to date");
    }
    println!("asset={}", result.asset);
    println!("source={}", result.source);
    println!("verified={}", result.verified);
    if let Some(root_cid) = &result.root_cid {
        println!("root_cid={root_cid}");
    }
    if let Some(release_cid) = &result.release_cid {
        println!("release_cid={release_cid}");
    }
    if let Some(path) = &result.path {
        println!("path={path}");
    }
    Ok(())
}

fn print_downloaded(result: &ProductUpdateResult, json_output: bool) -> Result<()> {
    if json_output {
        print_update_json(result)?;
        return Ok(());
    }
    println!("downloaded {}", result.asset);
    if let Some(path) = &result.path {
        println!("path={path}");
    }
    println!("source={}", result.source);
    println!("verified={}", result.verified);
    Ok(())
}

fn print_up_to_date(result: &ProductUpdateResult, json_output: bool) -> Result<()> {
    if json_output {
        print_update_json(result)?;
        return Ok(());
    }
    println!("idrive {PRODUCT_VERSION} is up to date");
    Ok(())
}

fn print_update_json(output: &ProductUpdateResult) -> Result<()> {
    println!("{}", serde_json::to_string(output)?);
    Ok(())
}

fn install_cli_archive(
    archive_path: &Path,
    temp_dir: &Path,
    destination: Option<&Path>,
) -> Result<()> {
    // Archive directory modes must not weaken the enclosing private workspace.
    let extraction = temp_dir.join("extracted");
    fs::create_dir(&extraction).context("creating update extraction directory")?;
    extract_archive(archive_path, &extraction)?;
    let binary = find_idrive_binary(&extraction)?;
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
    let mut staged = tempfile::Builder::new()
        .prefix(".idrive-update-")
        .suffix(std::env::consts::EXE_SUFFIX)
        .tempfile_in(parent)
        .context("creating private update staging file")?;
    let mut source_file =
        fs::File::open(source).with_context(|| format!("opening {}", source.display()))?;
    std::io::copy(&mut source_file, staged.as_file_mut())
        .context("copying verified update into staging file")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        staged
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o755))
            .context("marking staged update executable")?;
    }
    staged
        .persist(destination)
        .with_context(|| format!("installing update at {}", destination.display()))?;
    Ok(())
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
    use serde_json::json;

    use super::*;

    #[test]
    fn update_workspaces_are_unique_and_preserve_existing_downloads() {
        let first = create_update_workspace(None).unwrap();
        let marker = first.path().join("verified-download");
        fs::write(&marker, b"verified content").unwrap();
        let second = create_update_workspace(None).unwrap();
        assert_ne!(
            first.path(),
            second.path(),
            "concurrent updates need separate workspaces"
        );
        assert_eq!(fs::read(marker).unwrap(), b"verified content");
    }

    #[cfg(unix)]
    #[test]
    fn update_workspace_is_private() {
        use std::os::unix::fs::PermissionsExt;

        let directory = create_update_workspace(None).unwrap();
        let permissions = fs::metadata(directory.path()).unwrap().permissions().mode();
        assert_eq!(permissions & 0o077, 0);
    }

    #[test]
    fn installs_archive_over_existing_binary() {
        let fixture = create_update_workspace(None).unwrap();
        let package = fixture.path().join("package");
        fs::create_dir(&package).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&package, fs::Permissions::from_mode(0o777)).unwrap();
        }
        let name = format!("idrive{}", std::env::consts::EXE_SUFFIX);
        fs::write(package.join(&name), b"verified replacement").unwrap();
        let archive = fixture.path().join("update.tar.gz");
        let output = Command::new("tar")
            .arg("-czf")
            .arg(&archive)
            .arg("-C")
            .arg(&package)
            .arg(".")
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", command_error("tar", &output));
        let destination = fixture.path().join(&name);
        fs::write(&destination, b"old binary").unwrap();
        let extraction = create_update_workspace(None).unwrap();

        install_cli_archive(&archive, extraction.path(), Some(&destination)).unwrap();

        assert_eq!(fs::read(&destination).unwrap(), b"verified replacement");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(destination).unwrap().permissions().mode() & 0o777,
                0o755
            );
            assert_eq!(
                fs::metadata(extraction.path())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700,
                "archive directory permissions must not weaken the private workspace"
            );
        }
    }

    #[test]
    fn failed_install_preserves_binary_and_cleans_staging() {
        let directory = create_update_workspace(None).unwrap();
        let destination = directory.path().join("idrive");
        fs::write(&destination, b"existing binary").unwrap();

        assert!(install_binary(&directory.path().join("missing"), &destination).is_err());

        assert_eq!(fs::read(destination).unwrap(), b"existing binary");
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }

    #[test]
    fn product_update_config_includes_running_embedded_hashtree_url() {
        let status = json!({
            "fresh": true,
            "embedded_hashtree": {
                "base_url": "http://127.0.0.1:18432"
            }
        });
        let app_config = AppConfig {
            blossom_servers: vec!["https://backup.example".to_string()],
            ..AppConfig::default()
        };
        let config_dir = Path::new("/tmp/iris-drive-test");
        let update_config =
            product_update_config(config_dir, &app_config, Some(&status), Some("htree://test"));

        assert_eq!(
            update_config.embedded_hashtree_base_url.as_deref(),
            Some("http://127.0.0.1:18432")
        );
        assert_eq!(
            update_config.blossom_servers,
            vec!["https://backup.example"]
        );
        assert_eq!(update_config.update_ref.as_deref(), Some("htree://test"));
        assert_eq!(update_config.config_dir.as_deref(), Some(config_dir));
    }
}
