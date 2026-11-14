//! The `created_at` this config last signed for each drive-root address.
//!
//! Drive roots are replaceable events, and NIP-01 breaks a `created_at` tie
//! by keeping the lower event id. A changed root signed in the same second as
//! the previous one would therefore lose about half the time; for a device
//! approval that means the new device never gets its root-key wrap. The CLI,
//! daemon and apps publish from separate processes, so the clock lives in the
//! config directory behind a file lock.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

const CLOCK_FILE: &str = "drive-root-clock.json";
const LOCK_FILE: &str = "drive-root-clock.lock";

#[derive(Serialize, Deserialize)]
struct Signed {
    created_at: u64,
    version: String,
}

/// Reserve the `created_at` for a drive-root event at `address`.
///
/// A new `version` always signs after the last second used for the address.
/// The same version may reuse that second, so rebuilding an unchanged root
/// does not run the clock ahead.
pub(crate) fn reserve(
    config_dir: &Path,
    address: &str,
    version: &str,
    wanted: u64,
) -> std::io::Result<u64> {
    std::fs::create_dir_all(config_dir)?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(config_dir.join(LOCK_FILE))?;
    // The OS releases the lock when `lock` drops or the process exits.
    lock.lock()?;

    let path = config_dir.join(CLOCK_FILE);
    let mut clock: BTreeMap<String, Signed> = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(std::io::Error::other)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
        Err(error) => return Err(error),
    };
    let created_at = match clock.get(address) {
        Some(last) if last.version == version => wanted.max(last.created_at),
        Some(last) => wanted.max(last.created_at.saturating_add(1)),
        None => wanted,
    };
    clock.insert(
        address.to_owned(),
        Signed {
            created_at,
            version: version.to_owned(),
        },
    );
    let bytes = serde_json::to_vec(&clock).map_err(std::io::Error::other)?;
    crate::atomic_file::atomic_write(&path, &bytes)?;
    Ok(created_at)
}

#[cfg(test)]
mod tests {
    use super::reserve;

    #[test]
    fn a_changed_root_in_the_same_second_signs_a_second_later() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path();

        assert_eq!(reserve(config, "a", "root|x", 100).unwrap(), 100);
        assert_eq!(reserve(config, "a", "root|x,y", 100).unwrap(), 101);
        // Rebuilding the unchanged root keeps its second.
        assert_eq!(reserve(config, "a", "root|x,y", 100).unwrap(), 101);
        // Each address has its own clock.
        assert_eq!(reserve(config, "b", "root|x", 100).unwrap(), 100);
        // Wall time past the clock wins.
        assert_eq!(reserve(config, "a", "next|x,y", 200).unwrap(), 200);
    }

    #[test]
    fn concurrent_publishers_never_share_a_second() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().to_path_buf();
        let seconds: Vec<u64> = (0..8)
            .map(|n| {
                let config = config.clone();
                std::thread::spawn(move || reserve(&config, "a", &format!("root-{n}"), 100))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().unwrap().unwrap())
            .collect();

        let mut sorted = seconds.clone();
        sorted.sort_unstable();
        assert_eq!(sorted, (100..108).collect::<Vec<_>>(), "{seconds:?}");
    }
}
