use std::fs;
use std::io::Write;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static ATOMIC_WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Atomically replace a file without opening an existing destination or
/// temporary path for writing. The containing directory must be trusted.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let counter = ATOMIC_WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp_path = parent.join(format!(
        ".iris-drive.{}.{}.tmp",
        std::process::id(),
        counter
    ));

    // Only clean up a temporary file after successfully creating it ourselves.
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&tmp_path, path)?;
        sync_containing_directory(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp_path);
    }
    result
}

#[cfg(unix)]
fn sync_containing_directory(path: &Path) -> std::io::Result<()> {
    fs::File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()
}

// The Rust standard library has no portable directory-sync operation on
// Windows. The file itself is still flushed before the atomic rename.
#[cfg(not(unix))]
fn sync_containing_directory(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn atomic_write_replaces_existing_file_and_removes_temp() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("config.toml");

        atomic_write(&path, b"old").unwrap();
        atomic_write(&path, b"schema_version = 4\n").unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "schema_version = 4\n"
        );
        let temp_files = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".tmp"))
            .count();
        assert_eq!(temp_files, 0);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_supports_maximum_length_file_name() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("a".repeat(255));

        atomic_write(&path, b"contents").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"contents");
    }

    #[cfg(unix)]
    #[test]
    fn containing_directory_can_be_synced_after_atomic_rename() {
        let dir = tempdir().unwrap();
        sync_containing_directory(&dir.path().join("config.toml")).unwrap();
    }
}
