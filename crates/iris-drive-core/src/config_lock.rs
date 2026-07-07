use std::path::{Path, PathBuf};

use anyhow::Context;

use crate::daemon_liveness::process_is_running;

pub struct ConfigMutationLock {
    path: PathBuf,
}

#[derive(Debug)]
struct ConfigMutationLockTimeout {
    path: PathBuf,
}

impl std::fmt::Display for ConfigMutationLockTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "timed out waiting for config mutation lock {}",
            self.path.display()
        )
    }
}

impl std::error::Error for ConfigMutationLockTimeout {}

impl ConfigMutationLock {
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(25);
    const WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
    const STALE_AFTER: std::time::Duration = std::time::Duration::from_mins(2);

    pub async fn acquire(config_dir: &Path) -> anyhow::Result<Self> {
        Self::acquire_with_timeout(config_dir, Self::WAIT_TIMEOUT).await
    }

    pub async fn acquire_for_background<F>(
        config_dir: &Path,
        is_stale: F,
    ) -> anyhow::Result<Option<Self>>
    where
        F: FnMut() -> bool,
    {
        let retry_delays = [
            std::time::Duration::from_millis(250),
            std::time::Duration::from_secs(1),
            std::time::Duration::from_secs(2),
            std::time::Duration::from_secs(4),
        ];
        Self::acquire_for_background_with_options(
            config_dir,
            is_stale,
            Self::WAIT_TIMEOUT,
            &retry_delays,
        )
        .await
    }

    pub async fn acquire_for_background_with_options<F>(
        config_dir: &Path,
        mut is_stale: F,
        wait_timeout: std::time::Duration,
        retry_delays: &[std::time::Duration],
    ) -> anyhow::Result<Option<Self>>
    where
        F: FnMut() -> bool,
    {
        for retry_delay in std::iter::once(std::time::Duration::ZERO).chain(
            retry_delays
                .iter()
                .copied()
                .filter(|delay| !delay.is_zero()),
        ) {
            if retry_delay > std::time::Duration::ZERO {
                tokio::time::sleep(retry_delay).await;
            }
            if is_stale() {
                return Ok(None);
            }
            match Self::acquire_with_timeout(config_dir, wait_timeout).await {
                Ok(lock) => {
                    if is_stale() {
                        return Ok(None);
                    }
                    return Ok(Some(lock));
                }
                Err(error) if error.downcast_ref::<ConfigMutationLockTimeout>().is_some() => {}
                Err(error) => return Err(error),
            }
        }
        if is_stale() {
            return Ok(None);
        }
        Self::acquire_with_timeout(config_dir, wait_timeout)
            .await
            .map(Some)
    }

    async fn acquire_with_timeout(
        config_dir: &Path,
        wait_timeout: std::time::Duration,
    ) -> anyhow::Result<Self> {
        std::fs::create_dir_all(config_dir)
            .with_context(|| format!("creating config dir {}", config_dir.display()))?;
        let path = config_dir.join("config-mutation.lock");
        let started = std::time::Instant::now();

        loop {
            match Self::try_create(&path) {
                Ok(lock) => return Ok(lock),
                Err(error) if Self::lock_create_error_is_contention(&path, &error) => {
                    Self::remove_stale_lock(&path);
                    if started.elapsed() >= wait_timeout {
                        return Err(ConfigMutationLockTimeout { path }.into());
                    }
                    tokio::time::sleep(Self::POLL_INTERVAL).await;
                }
                Err(error) => {
                    return Err(error).with_context(|| {
                        format!("creating config mutation lock {}", path.display())
                    });
                }
            }
        }
    }

    #[must_use]
    pub fn lock_create_error_is_contention(_path: &Path, error: &std::io::Error) -> bool {
        error.kind() == std::io::ErrorKind::AlreadyExists
            || error.kind() == std::io::ErrorKind::PermissionDenied
    }

    fn try_create(path: &Path) -> std::io::Result<Self> {
        use std::io::Write;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        writeln!(file, "{}", std::process::id())?;
        Ok(Self {
            path: path.to_path_buf(),
        })
    }

    fn remove_stale_lock(path: &Path) {
        if let Ok(contents) = std::fs::read_to_string(path)
            && let Ok(pid) = contents.trim().parse::<u32>()
            && !process_is_running(pid)
        {
            let _ = std::fs::remove_file(path);
            return;
        }

        if let Ok(metadata) = std::fs::metadata(path)
            && let Ok(modified) = metadata.modified()
            && modified
                .elapsed()
                .is_ok_and(|elapsed| elapsed >= Self::STALE_AFTER)
        {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for ConfigMutationLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
