//! One authoritative backup endpoint per configuration directory.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::Path;

use anyhow::{Context, Result};

/// The returned file holds the OS lock until dropped, including on process
/// exit. A live owner has no expiry. Keep the file throughout endpoint shutdown,
/// and never unlink it: replacing its inode would allow two concurrent owners.
pub(crate) fn try_acquire(config_dir: &Path) -> Result<Option<File>> {
    std::fs::create_dir_all(config_dir)?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(config_dir.join("friend-backup-service.lock"))
        .context("opening friend backup service lease")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(TryLockError::WouldBlock) => Ok(None),
        Err(TryLockError::Error(error)) => Err(error).context("locking friend backup service"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lease_remains_exclusive_until_the_owner_file_is_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let owner = try_acquire(dir.path()).unwrap().unwrap();
        assert!(try_acquire(dir.path()).unwrap().is_none());
        drop(owner);
        let successor = try_acquire(dir.path()).unwrap().unwrap();
        assert!(try_acquire(dir.path()).unwrap().is_none());
        drop(successor);
    }

    #[test]
    fn separate_config_directories_have_independent_leases() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let _first_owner = try_acquire(first.path()).unwrap().unwrap();
        let _second_owner = try_acquire(second.path()).unwrap().unwrap();
    }

    #[test]
    fn lease_is_private_and_its_inode_is_never_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("friend-backup-service.lock");
        let owner = try_acquire(dir.path()).unwrap().unwrap();
        let original = std::fs::metadata(&path).unwrap();
        drop(owner);
        assert!(path.exists());
        let _successor = try_acquire(dir.path()).unwrap().unwrap();
        let current = std::fs::metadata(&path).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            assert_eq!(current.permissions().mode() & 0o777, 0o600);
            assert_eq!(original.ino(), current.ino());
        }
        #[cfg(not(unix))]
        let _ = (original, current);
    }

    #[test]
    fn inaccessible_or_invalid_lease_path_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("friend-backup-service.lock")).unwrap();
        assert!(try_acquire(dir.path()).is_err());
    }
}
