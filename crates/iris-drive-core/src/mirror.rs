//! Device-local, one-way folder mirrors. Only verified file contents enter My Drive.
use crate::{AppConfig, config_lock::ConfigMutationLock, paths::config_path_in};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};
use uuid::Uuid;

mod copy;
mod source;

pub const INTERVAL_SECONDS: u64 = 300;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Mirror {
    pub id: Uuid,
    pub source: PathBuf,
    pub destination: String,
    // An app-scoped bookmark is consumed only by the Mac app and never published.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bookmark: Option<String>,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct Report {
    pub finished_at: u64,
    pub copied: u64,
    pub unchanged: u64,
    pub pending: u64,
    pub issues: Vec<String>,
}

impl Report {
    fn issue(&mut self, path: &str, error: impl std::fmt::Display) {
        self.pending += 1;
        if self.issues.len() < 25 {
            self.issues.push(format!("{path}: {error}"));
        }
    }
}

#[derive(Default, Serialize, Deserialize)]
struct Checkpoint {
    files: BTreeMap<String, source::Verified>,
}

#[derive(Serialize)]
pub struct Status {
    #[serde(flatten)]
    pub mirror: Mirror,
    pub last_run: Option<Report>,
    pub running: bool,
    pub due: bool,
}

fn read<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(
            serde_json::from_slice(&bytes).context("reading saved mirror state")?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn save(path: &Path, value: &impl Serialize) -> Result<()> {
    crate::atomic_write(path, &serde_json::to_vec(value)?)?;
    Ok(())
}

fn owner_dir(config: &Path) -> Result<PathBuf> {
    let cfg = AppConfig::load_or_default_cached_profile(config_path_in(config))?;
    let profile = cfg.profile.context("Set up Iris Drive first")?;
    ensure!(
        profile.is_authorized(),
        "Link this device before mirroring folders"
    );
    Ok(config
        .join("mirrors")
        .join(profile.profile_id.to_string())
        .join(profile.app_key_pubkey))
}

fn registry(dir: &Path) -> Result<Vec<Mirror>> {
    Ok(read(&dir.join("folders.json"))?.unwrap_or_default())
}

fn acquire_run(dir: &Path) -> Result<Option<fs::File>> {
    fs::create_dir_all(dir)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("run.lock"))?;
    match file.try_lock() {
        Ok(()) => Ok(Some(file)),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(error)) => Err(error.into()),
    }
}

fn under(path: &str, ancestor: &str) -> bool {
    path == ancestor
        || path
            .strip_prefix(ancestor)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn overlaps(a: &str, b: &str) -> bool {
    under(a, b) || under(b, a)
}

pub(crate) fn ensure_private(config: &AppConfig, dest: &str) -> Result<()> {
    let profile = config.profile.as_ref().context("Set up Iris Drive first")?;
    for folder in &config.shared_folders {
        let received = crate::sharing::shared_with_me_mount_path(folder, &config.shared_folders);
        let shared_source =
            folder.owner_profile_id == profile.profile_id && overlaps(dest, &folder.source_path);
        ensure!(
            !(overlaps(dest, &received) || shared_source),
            "Choose a personal destination outside shared folders"
        );
    }
    ensure!(
        !config
            .share_shortcuts
            .iter()
            .any(|s| overlaps(dest, &s.path)),
        "Choose a destination outside shared shortcuts"
    );
    Ok(())
}

/// Store a staged local original in bounded chunks and verify it against the source.
pub(crate) async fn verified_original(
    daemon: &crate::Daemon,
    source: &Path,
) -> Result<(hashtree_core::Cid, u64, [u8; 32], Option<i64>)> {
    let file = source::verify_file(daemon.tree(), fs::File::open(source)?, None).await?;
    Ok((
        hashtree_core::Cid::parse(&file.cid)?,
        file.size(),
        file.digest,
        file.modified(),
    ))
}

pub fn list(config: &Path) -> Result<Vec<Status>> {
    if !config.join("mirrors").try_exists()? {
        return Ok(Vec::new());
    }
    let owner = owner_dir(config)?;
    registry(&owner)?
        .into_iter()
        .map(|mirror| {
            let dir = owner.join(mirror.id.to_string());
            let running = acquire_run(&dir)?.is_none();
            let last_run = read(&dir.join("report.json"))?;
            Ok(Status {
                mirror,
                running,
                due: due(last_run.as_ref()),
                last_run,
            })
        })
        .collect()
}

pub fn add(
    config: &Path,
    path: &Path,
    dest: Option<&str>,
    bookmark: Option<String>,
) -> Result<Mirror> {
    let source = path.canonicalize().context("opening source folder")?;
    ensure!(source.is_dir(), "Choose a source folder");
    source::validate_location(config, &source)?;
    let name = dest
        .or_else(|| source.file_name()?.to_str())
        .context("Choose a destination in My Drive")?;
    let destination = crate::provider::normalize_provider_path(name)?;
    ensure!(
        !crate::indexer::path_has_ignored_component(&destination),
        "Choose a visible destination in My Drive"
    );
    let _lock = ConfigMutationLock::acquire_blocking(config)?;
    let owner = owner_dir(config)?;
    ensure_private(
        &AppConfig::load_or_default_cached_profile(config_path_in(config))?,
        &destination,
    )?;
    // Source paths and app-scoped bookmarks stay private to this local account.
    let mut directory = fs::DirBuilder::new();
    directory.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        directory.mode(0o700);
    }
    directory.create(&owner)?;
    let mut mirrors = registry(&owner)?;
    ensure!(
        !mirrors
            .iter()
            .any(|m| overlaps(&m.destination.to_lowercase(), &destination.to_lowercase())),
        "Another mirror uses this destination"
    );
    let mirror = Mirror {
        id: Uuid::new_v4(),
        source,
        destination,
        bookmark,
    };
    mirrors.push(mirror.clone());
    save(&owner.join("folders.json"), &mirrors)?;
    Ok(mirror)
}

fn find(dir: &Path, dest: &str) -> Result<Mirror> {
    registry(dir)?
        .into_iter()
        .find(|m| m.destination == dest)
        .context("No mirror with that destination")
}

pub fn remove(config: &Path, dest: &str) -> Result<()> {
    let _lock = ConfigMutationLock::acquire_blocking(config)?;
    let owner = owner_dir(config)?;
    let mut mirrors = registry(&owner)?;
    let mirror = mirrors
        .iter()
        .find(|m| m.destination == dest)
        .context("No mirror with that destination")?;
    let dir = owner.join(mirror.id.to_string());
    let _run = acquire_run(&dir)?.context("Mirror is running. Try again when it finishes")?;
    mirrors.retain(|m| m.destination != dest);
    save(&owner.join("folders.json"), &mirrors)?;
    for name in ["files.json", "report.json"] {
        let path = dir.join(name);
        if path.try_exists()? {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn due(last: Option<&Report>) -> bool {
    last.is_none_or(|r| {
        now() < r.finished_at || now().saturating_sub(r.finished_at) >= INTERVAL_SECONDS
    })
}

pub async fn run(config: &Path, dest: &str, scheduled: bool) -> Result<Option<Report>> {
    run_with_access(config, dest, scheduled, None, None).await
}

/// Resolve Mac security-scoped access in the calling process. Bind the selection
/// to its registration even if a destination was removed and added again.
pub async fn run_with_access(
    config: &Path,
    dest: &str,
    scheduled: bool,
    expected: Option<Uuid>,
    access_error: Option<&str>,
) -> Result<Option<Report>> {
    let owner = owner_dir(config)?;
    let mirror = find(&owner, dest)?;
    ensure!(
        expected.is_none_or(|id| id == mirror.id),
        "Mirror registration changed"
    );
    let dir = owner.join(mirror.id.to_string());
    let Some(_run) = acquire_run(&dir)? else {
        ensure!(scheduled, "Mirror is already running");
        return Ok(None);
    };
    ensure!(
        find(&owner, dest)?.id == mirror.id,
        "Mirror registration changed"
    );
    let report_path = dir.join("report.json");
    if scheduled && !due(read::<Report>(&report_path)?.as_ref()) {
        return Ok(None);
    }
    let mut report = Report::default();
    if let Some(error) = access_error {
        report.issue(&mirror.destination, error);
    } else if let Err(error) = copy::run(config, &owner, &dir, &mirror, &mut report).await {
        report.issue(&mirror.destination, format!("{error:#}"));
    }
    report.finished_at = now();
    save(&report_path, &report)?;
    Ok(Some(report))
}
