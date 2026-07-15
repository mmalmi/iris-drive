#[cfg(target_os = "linux")]
use std::path::{Path, PathBuf};
#[cfg(target_os = "linux")]
use std::process::Command;
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

    pub(crate) fn current_visible_root(&self) -> Cid {
        self.fs.current_root()
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

    pub(crate) fn current_visible_root(&self) -> Cid {
        self.fs.current_root()
    }

    pub(crate) async fn refresh_from_config_if_current(
        &self,
        config_dir: &Path,
        expected_current: &Cid,
    ) -> Result<MountRefreshOutcome> {
        let daemon = Daemon::open(config_dir).context("opening daemon for mount refresh")?;
        let visible = primary_merged_root(daemon.tree(), daemon.config())
            .await
            .context("building visible mount root")?;
        let replaced = self
            .fs
            .replace_root_if_current(expected_current, visible.root_cid.clone())
            .map_err(|error| anyhow::anyhow!("refreshing mounted root: {error}"))?;
        if !replaced {
            return Ok(MountRefreshOutcome::Dirty(self.current_visible_root()));
        }
        Ok(MountRefreshOutcome::Refreshed(visible))
    }
}

#[cfg(target_os = "linux")]
pub(crate) enum MountRefreshOutcome {
    Refreshed(PrimaryMergedRoot),
    Dirty(Cid),
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
    if current_app_key_root_missing(daemon.config()) {
        daemon
            .import_visible_root(visible.root_cid.clone())
            .await
            .context("recording initial empty mounted root")?;
        visible = primary_merged_root(daemon.tree(), daemon.config())
            .await
            .context("rebuilding initial visible mount root")?;
    }
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
fn current_app_key_root_missing(config: &AppConfig) -> bool {
    let Some(account) = config.profile.as_ref() else {
        return false;
    };
    config
        .drive(PRIMARY_DRIVE_ID)
        .and_then(|drive| drive.app_key_roots.get(&account.app_key_pubkey))
        .is_none()
}

#[cfg(target_os = "linux")]
fn prepare_mountpoint(mountpoint: &Path) -> Result<()> {
    match std::fs::metadata(mountpoint) {
        Ok(metadata) => {
            if !metadata.is_dir() {
                anyhow::bail!(
                    "mountpoint exists but is not a directory: {}",
                    mountpoint.display()
                );
            }
            if let Err(error) = std::fs::read_dir(mountpoint) {
                if is_disconnected_mount(&error) {
                    unmount_stale_mountpoint(mountpoint)?;
                } else {
                    return Err(error)
                        .with_context(|| format!("reading mountpoint {}", mountpoint.display()));
                }
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir_all(mountpoint)
                .with_context(|| format!("creating mountpoint {}", mountpoint.display()))
        }
        Err(error) if is_disconnected_mount(&error) => {
            unmount_stale_mountpoint(mountpoint)?;
            std::fs::create_dir_all(mountpoint)
                .with_context(|| format!("creating mountpoint {}", mountpoint.display()))
        }
        Err(error) => {
            Err(error).with_context(|| format!("checking mountpoint {}", mountpoint.display()))
        }
    }
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

#[cfg(target_os = "linux")]
fn is_disconnected_mount(error: &std::io::Error) -> bool {
    const ENOTCONN: i32 = 107;
    error.raw_os_error() == Some(ENOTCONN)
        || error
            .to_string()
            .contains("Transport endpoint is not connected")
}

#[cfg(target_os = "linux")]
fn unmount_stale_mountpoint(mountpoint: &Path) -> Result<()> {
    let mut attempts = Vec::new();
    for program in ["fusermount3", "fusermount", "umount"] {
        let mut command = Command::new(program);
        if program.starts_with("fusermount") {
            command.arg("-u");
        }
        command.arg(mountpoint);

        match command.status() {
            Ok(status) if status.success() => return Ok(()),
            Ok(status) => attempts.push(format!("{program} exited with {status}")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                attempts.push(format!("{program} not found"));
            }
            Err(error) => attempts.push(format!("{program}: {error}")),
        }
    }

    anyhow::bail!(
        "failed to unmount stale Iris Drive mount {} ({})",
        mountpoint.display(),
        attempts.join("; ")
    )
}

#[cfg(not(target_os = "linux"))]
pub(crate) struct IrisDriveMount;

#[cfg(not(target_os = "linux"))]
#[derive(Clone)]
pub(crate) struct IrisDriveMountHandle;

#[cfg(not(target_os = "linux"))]
#[allow(clippy::unused_self)]
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

    pub(crate) fn current_visible_root(&self) -> hashtree_core::Cid {
        hashtree_core::Cid {
            hash: [0; 32],
            key: None,
        }
    }
}

#[cfg(not(target_os = "linux"))]
#[allow(clippy::unused_self)]
impl IrisDriveMountHandle {
    pub(crate) fn mountpoint(&self) -> &std::path::Path {
        std::path::Path::new("")
    }

    pub(crate) fn current_visible_root(&self) -> hashtree_core::Cid {
        hashtree_core::Cid {
            hash: [0; 32],
            key: None,
        }
    }

    #[allow(clippy::unused_async)]
    pub(crate) async fn refresh_from_config_if_current(
        &self,
        _config_dir: &std::path::Path,
        _expected_current: &hashtree_core::Cid,
    ) -> anyhow::Result<MountRefreshOutcome> {
        anyhow::bail!("Iris Drive mount mode is not supported on this platform yet")
    }
}

#[cfg(not(target_os = "linux"))]
#[allow(dead_code)]
pub(crate) enum MountRefreshOutcome {
    Refreshed(iris_drive_core::PrimaryMergedRoot),
    Dirty(hashtree_core::Cid),
}

#[cfg(not(target_os = "linux"))]
#[allow(clippy::unused_async)]
pub(crate) async fn start_iris_drive_mount(
    _config_dir: &std::path::Path,
    _mountpoint: std::path::PathBuf,
) -> anyhow::Result<IrisDriveMount> {
    anyhow::bail!("Iris Drive mount mode is not supported on this platform yet")
}
