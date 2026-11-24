use std::path::{Path, PathBuf};

/// `FileEventBus` holds an exclusive writer lock next to its snapshot until
/// shutdown. `RunningBoard` permits that lock in the process's temporary folder.
pub(super) fn snapshot_path(shared: &Path, private_temp: &Path) -> std::io::Result<PathBuf> {
    let name = hex::encode(hashtree_core::sha256(shared.as_os_str().as_encoded_bytes()));
    let directory = private_temp.join("iris-drive-peer-announcements");
    let destination = directory.join(format!("{name}.json"));
    if destination.try_exists()? {
        return Ok(destination);
    }

    // Import the previous version's signed snapshot without ever opening its
    // shared writer lock. FileEventBus validates the imported events on open.
    let source = match std::fs::File::open(shared) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(destination),
        Err(error) => return Err(error),
    };
    let limit = nostr_pubsub::MAX_RETAINED_SNAPSHOT_BYTES as u64;
    std::fs::create_dir_all(directory.as_path())?;
    let mut seed = tempfile::NamedTempFile::new_in(directory)?;
    if std::io::copy(&mut std::io::Read::take(source, limit + 1), &mut seed)? > limit {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "peer announcement snapshot exceeds byte limit",
        ));
    }
    // Simultaneous opens may both import, but neither can replace a live store.
    match seed.persist_noclobber(&destination) {
        Ok(_) => Ok(destination),
        Err(e) if e.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(destination),
        Err(e) => Err(e.error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr_pubsub::{EventRetentionPolicy, FileEventBus, Filter};

    #[tokio::test]
    async fn running_ios_peer_cache_keeps_its_writer_out_of_the_app_group() {
        let shared = tempfile::tempdir().unwrap();
        let private = tempfile::tempdir().unwrap();
        let legacy = shared.path().join("scope.json");
        let old_snapshot = br#"{"version":1,"events":[]}"#;
        std::fs::write(&legacy, old_snapshot).unwrap();
        let path = snapshot_path(&legacy, private.path()).unwrap();
        assert!(path.starts_with(private.path()));
        assert_eq!(std::fs::read(&path).unwrap(), old_snapshot);
        let store = FileEventBus::open(&path, EventRetentionPolicy::new(256, vec![Filter::new()]))
            .await
            .unwrap();
        assert!(!legacy.with_extension("json.lock").exists());
        let writer = std::fs::File::open(path.with_extension("json.lock")).unwrap();
        assert!(matches!(
            writer.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        std::fs::write(&legacy, b"superseded").unwrap();
        assert_eq!(snapshot_path(&legacy, private.path()).unwrap(), path);
        assert_eq!(std::fs::read(&path).unwrap(), old_snapshot);
        store.close().await;
        // close() drops this process's only lock descriptor, but flock belongs
        // to the open file description: a child that another test is spawning
        // shares it until its exec. Closing must still release the lock.
        let released_by = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            match writer.try_lock() {
                Ok(()) => break,
                Err(std::fs::TryLockError::WouldBlock)
                    if std::time::Instant::now() < released_by =>
                {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
                Err(error) => panic!("closed peer cache kept its writer lock: {error:?}"),
            }
        }
    }
}
