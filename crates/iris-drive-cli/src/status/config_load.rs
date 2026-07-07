use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DaemonStatusConfigFingerprint {
    exists: bool,
    modified_nanos: Option<u128>,
    len: u64,
}

#[derive(Clone)]
struct DaemonStatusConfigCacheEntry {
    fingerprint: DaemonStatusConfigFingerprint,
    config: AppConfig,
}

static DAEMON_STATUS_CONFIG_CACHE: std::sync::LazyLock<
    std::sync::Mutex<BTreeMap<PathBuf, DaemonStatusConfigCacheEntry>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(BTreeMap::new()));

pub(super) fn daemon_status_config(
    config_dir: &Path,
) -> std::result::Result<AppConfig, iris_drive_core::config::ConfigError> {
    let path = config_path_in(config_dir);
    let fingerprint = daemon_status_config_fingerprint(&path)?;
    if let Ok(cache) = DAEMON_STATUS_CONFIG_CACHE.lock()
        && let Some(entry) = cache.get(&path)
        && entry.fingerprint == fingerprint
    {
        return Ok(entry.config.clone());
    }

    let config = status_config_from_path(&path)?;
    if let Ok(mut cache) = DAEMON_STATUS_CONFIG_CACHE.lock() {
        cache.insert(
            path,
            DaemonStatusConfigCacheEntry {
                fingerprint,
                config: config.clone(),
            },
        );
    }
    Ok(config)
}

pub(super) fn status_config_from_path(
    path: &Path,
) -> std::result::Result<AppConfig, iris_drive_core::config::ConfigError> {
    let mut config = AppConfig::load_or_default_cached_profile(path)?;
    if daemon_status_config_needs_roster_sidecar(&config) {
        config = AppConfig::load_or_default(path)?;
    }
    if let Some(config_dir) = path.parent() {
        hydrate_status_profile_with_local_keys(&mut config, config_dir);
    }
    hydrate_daemon_status_profile_cache(&mut config);
    Ok(config)
}

fn daemon_status_config_fingerprint(
    path: &Path,
) -> std::result::Result<DaemonStatusConfigFingerprint, iris_drive_core::config::ConfigError> {
    match std::fs::metadata(path) {
        Ok(metadata) => {
            let modified_nanos = metadata
                .modified()
                .ok()
                .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|duration| duration.as_nanos());
            Ok(DaemonStatusConfigFingerprint {
                exists: true,
                modified_nanos,
                len: metadata.len(),
            })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(DaemonStatusConfigFingerprint {
                exists: false,
                modified_nanos: None,
                len: 0,
            })
        }
        Err(error) => Err(error.into()),
    }
}

fn hydrate_status_profile_with_local_keys(config: &mut AppConfig, config_dir: &Path) {
    let Some(profile) = config.profile.clone() else {
        return;
    };
    if let Ok(profile) = iris_drive_core::Profile::load(profile, config_dir) {
        config.profile = Some(profile.state);
    }
}

fn daemon_status_config_needs_roster_sidecar(config: &AppConfig) -> bool {
    config.profile.as_ref().is_some_and(|profile| {
        !profile.has_profile_roster_evidence()
            && (profile.app_keys.is_none() || profile.profile_roster_projection.is_none())
    })
}

fn hydrate_daemon_status_profile_cache(config: &mut AppConfig) {
    let Some(profile) = config.profile.as_mut() else {
        return;
    };
    if profile.has_profile_roster_evidence()
        && (profile.app_keys.is_none() || profile.profile_roster_projection.is_none())
    {
        profile.sync_app_keys_from_profile();
    }
}
