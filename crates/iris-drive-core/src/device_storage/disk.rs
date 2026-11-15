//! How much room Drive has on this device: what it stores, and the free
//! space on the disk it stores on. Native Drive has no storage limit of its
//! own, so it may grow into all of the free space.

use std::path::Path;

/// Free bytes on the disk holding `path`, for this user.
#[cfg(unix)]
#[must_use]
pub fn free_bytes(path: &Path) -> Option<u64> {
    let stats = rustix::fs::statvfs(path).ok()?;
    stats.f_bavail.checked_mul(stats.f_frsize)
}

/// Free bytes on the disk holding `path`, for this user. Asked of
/// PowerShell, as the workspace allows no `unsafe` for the Win32 call.
#[cfg(windows)]
#[must_use]
pub fn free_bytes(path: &Path) -> Option<u64> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let quoted = path.display().to_string().replace('\'', "''");
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            &format!(
                "([System.IO.DriveInfo]::new([System.IO.Path]::GetPathRoot('{quoted}'))).AvailableFreeSpace"
            ),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    String::from_utf8_lossy(&output.stdout).trim().parse().ok()
}

#[cfg(not(any(unix, windows)))]
#[must_use]
pub fn free_bytes(_path: &Path) -> Option<u64> {
    None
}

/// Everything in the block store, in bytes.
#[must_use]
pub fn stored_bytes(blocks_dir: &Path) -> u64 {
    hashtree_fs::FsBlobStore::new(blocks_dir)
        .and_then(|store| store.stats())
        .map_or(0, |stats| stats.total_bytes)
}
