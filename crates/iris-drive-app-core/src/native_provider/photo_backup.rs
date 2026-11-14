use anyhow::{Context, Result, bail, ensure};
use iris_drive_core::{
    Daemon,
    config_lock::ConfigMutationLock,
    paths::provider_root_staging_path_in,
    photo_backup::{PhotoOriginal, check_profile},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

pub(crate) fn import_json(
    dir: &str,
    profile: &str,
    resource: &str,
    name: &str,
    source: &str,
) -> Value {
    let result = super::native_provider_runtime().and_then(|rt| {
        rt.block_on(import(
            Path::new(dir),
            profile,
            resource,
            name,
            Path::new(source),
        ))
    });
    match result {
        Ok(path) => json!({"ok": true, "path": path}),
        Err(error) => json!({"ok": false, "error": format!("{error:#}")}),
    }
}

async fn import(
    dir: &Path,
    profile: &str,
    resource: &str,
    name: &str,
    source: &Path,
) -> Result<String> {
    ensure!(
        !resource.is_empty() && !name.is_empty(),
        "Missing photo identifier or name"
    );
    let daemon = Daemon::open(dir)?;
    check_profile(daemon.config(), profile)?;
    let original = PhotoOriginal::stage(&daemon, source).await?;
    let _lock = ConfigMutationLock::acquire(dir).await?;
    let mut daemon = Daemon::open(dir)?;
    check_profile(daemon.config(), profile)?;
    ensure!(
        !provider_root_staging_path_in(dir).try_exists()?,
        "A file change is pending; try again"
    );
    let (_, projection, _) = daemon.provider_projection().await?;
    let base = projection.root_cid;
    let (root, path) = original.add_to(&daemon, &base, resource, name).await?;
    // Queue before committing so a shutdown between the local write and the phone's
    // receipt cannot lose the upload. Only the public hash is needed for coalescing.
    iris_drive_core::atomic_write(&pending_path(dir, profile), &root.hash)?;
    if root != base {
        super::persist_provider_root(&mut daemon, root, Some(base)).await?;
    }
    Ok(path)
}

fn pending_path(dir: &Path, profile: &str) -> PathBuf {
    let id = hashtree_core::to_hex(&hashtree_core::sha256(profile.as_bytes()));
    dir.join(format!("photo-backup-{id}.pending"))
}

pub(crate) fn publish_json(dir: &str, profile: &str, allow_network: bool) -> Value {
    let result = super::native_provider_runtime()
        .and_then(|runtime| runtime.block_on(publish(Path::new(dir), profile, allow_network)));
    match result {
        Ok(published) => json!({"ok": published, "pending": !published}),
        Err(error) => json!({"ok": false, "pending": true, "error": format!("{error:#}")}),
    }
}

async fn publish(dir: &Path, profile: &str, allow_network: bool) -> Result<bool> {
    let lock = ConfigMutationLock::acquire(dir).await?;
    let daemon = Daemon::open(dir)?;
    check_profile(daemon.config(), profile)?;
    let path = pending_path(dir, profile);
    let generation = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error.into()),
    };
    if !allow_network {
        return Ok(false);
    }
    let prepared = match super::prepare_current_app_key_root_publish(&daemon).await? {
        super::PreparedProviderRootPublish::Ready(prepared) => prepared,
        super::PreparedProviderRootPublish::Unavailable(reason) => {
            bail!(
                "{}",
                reason["error"]
                    .as_str()
                    .unwrap_or("Drive is waiting to sync")
            )
        }
    };
    drop(lock);
    // Large videos legitimately take longer than the foreground File Provider deadline.
    // Each Blossom transfer uses the client's request timeout. Never clear the queue
    // until the blocks and the signed root have both been accepted.
    let result = super::publish_prepared_app_key_root(dir, *prepared).await?;
    ensure!(
        result["published_drive_root"] == true,
        "Drive is waiting to sync"
    );
    let _lock = ConfigMutationLock::acquire(dir).await?;
    let current = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(true),
        Err(error) => return Err(error.into()),
    };
    if current != generation {
        return Ok(false);
    }
    fs::remove_file(path).context("finishing photo backup")?;
    Ok(true)
}
