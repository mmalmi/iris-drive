#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::sync::Arc;
#[cfg(target_os = "linux")]
use std::thread::JoinHandle;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
use anyhow::{Context, Result};
#[cfg(target_os = "linux")]
use hashtree_core::Cid;
#[cfg(target_os = "linux")]
use hashtree_fs::FsBlobStore;
#[cfg(target_os = "linux")]
use hashtree_fuse::{FsError as FuseFsError, HashtreeFuse, RootPublisher};
#[cfg(target_os = "linux")]
use iris_drive_core::config::AppConfig;
#[cfg(target_os = "linux")]
use iris_drive_core::daemon::Daemon;
#[cfg(target_os = "linux")]
use iris_drive_core::paths::config_path_in;
#[cfg(target_os = "linux")]
use iris_drive_core::{PRIMARY_DRIVE_ID, PrimaryMergedRoot, primary_merged_root};
#[cfg(target_os = "linux")]
use tokio::sync::mpsc;

#[cfg(target_os = "linux")]
const MOUNT_READY_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(target_os = "linux")]
const MOUNT_READY_POLL_INTERVAL: Duration = Duration::from_millis(50);

#[cfg(target_os = "linux")]
pub(crate) struct IrisDriveMount {
    mountpoint: PathBuf,
    fs: HashtreeFuse<FsBlobStore>,
    updates: Option<mpsc::UnboundedReceiver<Cid>>,
    _thread: JoinHandle<Result<(), String>>,
}

#[cfg(target_os = "linux")]
#[derive(Clone)]
pub(crate) struct IrisDriveMountHandle {
    mountpoint: PathBuf,
    fs: HashtreeFuse<FsBlobStore>,
}

#[cfg(target_os = "linux")]
struct ChannelRootPublisher {
    tx: mpsc::UnboundedSender<Cid>,
}

#[cfg(target_os = "linux")]
impl RootPublisher for ChannelRootPublisher {
    fn publish(&self, cid: &Cid) -> Result<(), FuseFsError> {
        self.tx
            .send(cid.clone())
            .map_err(|_| FuseFsError::Publish("iris-drive mount update worker stopped".into()))
    }
}

#[cfg(target_os = "linux")]
impl IrisDriveMount {
    pub(crate) fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }

    pub(crate) fn handle(&self) -> IrisDriveMountHandle {
        IrisDriveMountHandle {
            mountpoint: self.mountpoint.clone(),
            fs: self.fs.clone(),
        }
    }

    pub(crate) fn take_updates(&mut self) -> mpsc::UnboundedReceiver<Cid> {
        self.updates
            .take()
            .expect("mount update receiver already taken")
    }
}

#[cfg(target_os = "linux")]
impl IrisDriveMountHandle {
    pub(crate) fn mountpoint(&self) -> &Path {
        &self.mountpoint
    }

    pub(crate) async fn refresh_from_config(&self, config_dir: &Path) -> Result<PrimaryMergedRoot> {
        let daemon = Daemon::open(config_dir).context("opening daemon for mount refresh")?;
        let visible = primary_merged_root(daemon.tree(), daemon.config())
            .await
            .context("building visible mount root")?;
        self.fs
            .replace_root(visible.root_cid.clone())
            .map_err(|error| anyhow::anyhow!("refreshing mounted root: {error}"))?;
        Ok(visible)
    }
}

#[cfg(target_os = "linux")]
pub(crate) async fn start_iris_drive_mount(
    config_dir: &Path,
    mountpoint: PathBuf,
) -> Result<IrisDriveMount> {
    prepare_mountpoint(&mountpoint)?;

    let mut daemon = Daemon::open(config_dir).context("opening daemon for mount")?;
    let mut visible = primary_merged_root(daemon.tree(), daemon.config())
        .await
        .context("building initial visible mount root")?;
    if current_device_root_missing(daemon.config()) {
        daemon
            .import_visible_root(visible.root_cid.clone())
            .await
            .context("recording initial empty mounted root")?;
        visible = primary_merged_root(daemon.tree(), daemon.config())
            .await
            .context("rebuilding initial visible mount root")?;
    }
    remember_mountpoint(config_dir, &mountpoint)?;

    let (tx, updates) = mpsc::unbounded_channel();
    let publisher = Arc::new(ChannelRootPublisher { tx });
    let fs = HashtreeFuse::new_with_publisher(
        daemon.tree().get_store().clone(),
        visible.root_cid.clone(),
        Some(publisher),
    )
    .map_err(|error| anyhow::anyhow!("opening hashtree FUSE filesystem: {error}"))?;

    let thread_fs = fs.clone();
    let thread_mountpoint = mountpoint.clone();
    let thread = std::thread::Builder::new()
        .name("iris-drive-fuse".into())
        .spawn(move || {
            thread_fs
                .mount(&thread_mountpoint, &[])
                .map_err(|error| error.to_string())
        })
        .context("spawning hashtree FUSE mount thread")?;

    wait_for_mountpoint_ready(&mountpoint)?;

    Ok(IrisDriveMount {
        mountpoint,
        fs,
        updates: Some(updates),
        _thread: thread,
    })
}

#[cfg(target_os = "linux")]
fn current_device_root_missing(config: &AppConfig) -> bool {
    let Some(account) = config.account.as_ref() else {
        return false;
    };
    config
        .drive(PRIMARY_DRIVE_ID)
        .and_then(|drive| drive.device_roots.get(&account.device_pubkey))
        .is_none()
}

#[cfg(target_os = "linux")]
fn remember_mountpoint(config_dir: &Path, mountpoint: &Path) -> Result<()> {
    let path = config_path_in(config_dir);
    let mut config = AppConfig::load_or_default(&path)?;
    let Some(drive) = config.drive(PRIMARY_DRIVE_ID).cloned() else {
        return Ok(());
    };
    if drive.working_dir.as_deref() == Some(mountpoint) {
        return Ok(());
    }
    let mut updated = drive;
    updated.working_dir = Some(mountpoint.to_path_buf());
    config.upsert_drive(updated);
    config.save(path)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn prepare_mountpoint(mountpoint: &Path) -> Result<()> {
    if mountpoint.exists() {
        if !mountpoint.is_dir() {
            anyhow::bail!(
                "mountpoint exists but is not a directory: {}",
                mountpoint.display()
            );
        }
        return Ok(());
    }
    std::fs::create_dir_all(mountpoint)
        .with_context(|| format!("creating mountpoint {}", mountpoint.display()))
}

#[cfg(target_os = "linux")]
fn wait_for_mountpoint_ready(mountpoint: &Path) -> Result<()> {
    let deadline = Instant::now() + MOUNT_READY_TIMEOUT;
    let mut last_error = None;

    loop {
        match std::fs::read_dir(mountpoint) {
            Ok(_) => return Ok(()),
            Err(error) => {
                let text = error.to_string();
                if Instant::now() >= deadline {
                    anyhow::bail!(
                        "timed out waiting for Iris Drive mount {} to become readable: {}",
                        mountpoint.display(),
                        last_error.unwrap_or(text)
                    );
                }
                last_error = Some(text);
            }
        }
        std::thread::sleep(MOUNT_READY_POLL_INTERVAL);
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) struct IrisDriveMount;

#[cfg(not(target_os = "linux"))]
#[derive(Clone)]
pub(crate) struct IrisDriveMountHandle;

#[cfg(not(target_os = "linux"))]
impl IrisDriveMount {
    pub(crate) fn mountpoint(&self) -> &std::path::Path {
        std::path::Path::new("")
    }

    pub(crate) fn handle(&self) -> IrisDriveMountHandle {
        IrisDriveMountHandle
    }

    pub(crate) fn take_updates(
        &mut self,
    ) -> tokio::sync::mpsc::UnboundedReceiver<hashtree_core::Cid> {
        let (_tx, rx) = tokio::sync::mpsc::unbounded_channel();
        rx
    }
}

#[cfg(not(target_os = "linux"))]
impl IrisDriveMountHandle {
    pub(crate) fn mountpoint(&self) -> &std::path::Path {
        std::path::Path::new("")
    }

    pub(crate) async fn refresh_from_config(
        &self,
        _config_dir: &std::path::Path,
    ) -> anyhow::Result<iris_drive_core::PrimaryMergedRoot> {
        anyhow::bail!("Iris Drive mount mode is not supported on this platform yet")
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) async fn start_iris_drive_mount(
    _config_dir: &std::path::Path,
    _mountpoint: std::path::PathBuf,
) -> anyhow::Result<IrisDriveMount> {
    anyhow::bail!("Iris Drive mount mode is not supported on this platform yet")
}
