use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::Sender;
use std::thread;

use iris_drive_core::updater::{
    ProductUpdateMode, check_product_update_blocking, download_product_update_blocking,
    product_update_config_for_dir,
};

#[derive(Clone, Debug, Default)]
pub struct UpdateState {
    pub checking: bool,
    pub downloading: bool,
    pub available: bool,
    pub auto_check: bool,
    pub auto_install: bool,
    pub version: String,
    pub status: String,
    pub asset: Option<ReleaseAsset>,
}

#[derive(Clone, Debug)]
pub struct ReleaseAsset {
    pub name: String,
}

#[derive(Debug)]
pub enum UpdateEvent {
    Checked {
        manual: bool,
        result: Result<UpdateCheck, String>,
    },
    Downloaded(Result<PathBuf, String>),
}

#[derive(Debug)]
pub struct UpdateCheck {
    pub tag: String,
    pub asset: Option<ReleaseAsset>,
    pub newer: bool,
}

pub fn check(
    current_version: String,
    config_dir: PathBuf,
    manual: bool,
    sender: Sender<UpdateEvent>,
) {
    thread::spawn(move || {
        let result =
            check_blocking(&current_version, &config_dir).map_err(|error| error.to_string());
        let _ = sender.send(UpdateEvent::Checked { manual, result });
    });
}

pub fn download(
    current_version: String,
    config_dir: PathBuf,
    asset: ReleaseAsset,
    sender: Sender<UpdateEvent>,
) {
    thread::spawn(move || {
        let result = download_blocking(&current_version, &config_dir, &asset)
            .map_err(|error| error.to_string());
        let _ = sender.send(UpdateEvent::Downloaded(result));
    });
}

fn check_blocking(current_version: &str, config_dir: &Path) -> Result<UpdateCheck, String> {
    let result = check_product_update_blocking(
        current_version,
        ProductUpdateMode::App,
        product_update_config_for_dir(config_dir),
    )
    .map_err(|error| error.to_string())?;
    let asset = (!result.asset.trim().is_empty()).then_some(ReleaseAsset { name: result.asset });
    Ok(UpdateCheck {
        tag: result.tag,
        asset,
        newer: result.available,
    })
}

fn download_blocking(
    current_version: &str,
    config_dir: &Path,
    asset: &ReleaseAsset,
) -> Result<PathBuf, String> {
    let download_dir = update_download_dir();
    let result = download_product_update_blocking(
        current_version,
        ProductUpdateMode::App,
        product_update_config_for_dir(config_dir),
        Some(&download_dir),
    )
    .map_err(|error| error.to_string())?;
    if result.asset != asset.name {
        return Err(format!(
            "Latest release changed from {} to {}; please check again",
            asset.name, result.asset
        ));
    }
    let destination = result
        .path
        .map(PathBuf::from)
        .ok_or_else(|| "Updater did not return a downloaded file".to_string())?;
    maybe_make_executable_and_open(&destination, &asset.name)?;
    Ok(destination)
}

fn maybe_make_executable_and_open(destination: &Path, asset_name: &str) -> Result<(), String> {
    if asset_name.ends_with(".AppImage") {
        let mut permissions = fs::metadata(destination)
            .map_err(|error| format!("Downloaded update unavailable: {error}"))?
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(destination, permissions)
            .map_err(|error| format!("Could not make AppImage executable: {error}"))?;
    }

    if std::env::var("IRIS_DRIVE_UPDATE_SKIP_OPEN").ok().as_deref() != Some("1") {
        let _ = Command::new("xdg-open").arg(destination).spawn();
    }
    Ok(())
}

fn update_download_dir() -> PathBuf {
    std::env::var("IRIS_DRIVE_UPDATE_DOWNLOAD_DIR")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("IrisDriveDownloads"))
}
