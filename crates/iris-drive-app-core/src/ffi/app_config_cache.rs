use std::collections::BTreeMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use iris_drive_core::AppConfig;
#[cfg(any(test, target_os = "ios", target_os = "android"))]
use iris_drive_core::paths::config_path_in;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfigFileFingerprint {
    len: u64,
    modified: Option<std::time::SystemTime>,
    content_hash: Option<u64>,
}

#[derive(Clone)]
pub(super) struct RuntimeConfigCacheEntry {
    fingerprint: ConfigFileFingerprint,
    pub(super) config: AppConfig,
}

pub(super) static NATIVE_RUNTIME_CONFIG_CACHE: LazyLock<
    Mutex<BTreeMap<PathBuf, RuntimeConfigCacheEntry>>,
> = LazyLock::new(|| Mutex::new(BTreeMap::new()));

#[cfg(any(test, target_os = "ios", target_os = "android"))]
#[derive(Debug, Default)]
pub(super) struct NativeAppConfigCache {
    fingerprint: Option<ConfigFileFingerprint>,
    config: Option<AppConfig>,
}

#[cfg(any(test, target_os = "ios", target_os = "android"))]
impl NativeAppConfigCache {
    pub(super) fn load_with_change(
        &mut self,
        config_dir: &Path,
    ) -> Result<(AppConfig, bool), String> {
        let config_path = config_path_in(config_dir);
        let fingerprint = config_file_fingerprint(&config_path)
            .map_err(|error| format!("reading config metadata: {error}"))?;
        if self.fingerprint.as_ref() == Some(&fingerprint)
            && let Some(config) = self.config.as_ref()
        {
            return Ok((config.clone(), false));
        }

        let config = AppConfig::load_or_default(&config_path)
            .map_err(|error| format!("loading config: {error}"))?;
        self.fingerprint = Some(fingerprint);
        self.config = Some(config.clone());
        Ok((config, true))
    }
}

pub(crate) fn load_native_runtime_config_cached(config_path: &Path) -> Result<AppConfig, String> {
    let fingerprint = config_file_fingerprint(config_path)
        .map_err(|error| format!("reading config metadata: {error}"))?;
    if let Ok(cache) = NATIVE_RUNTIME_CONFIG_CACHE.lock()
        && let Some(entry) = cache.get(config_path)
        && entry.fingerprint == fingerprint
    {
        return Ok(entry.config.clone());
    }

    let config = AppConfig::load_or_default(config_path)
        .map_err(|error| format!("loading config: {error}"))?;
    if let Ok(mut cache) = NATIVE_RUNTIME_CONFIG_CACHE.lock() {
        cache.insert(
            config_path.to_path_buf(),
            RuntimeConfigCacheEntry {
                fingerprint,
                config: config.clone(),
            },
        );
    }
    Ok(config)
}

#[cfg(all(not(test), target_os = "android"))]
pub(crate) fn invalidate_native_runtime_config_cache(config_path: &Path) {
    if let Ok(mut cache) = NATIVE_RUNTIME_CONFIG_CACHE.lock() {
        cache.remove(config_path);
    }
}

fn config_file_fingerprint(path: &Path) -> std::io::Result<ConfigFileFingerprint> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(ConfigFileFingerprint {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            content_hash: Some(config_file_content_hash(path)?),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(ConfigFileFingerprint {
            len: 0,
            modified: None,
            content_hash: None,
        }),
        Err(error) => Err(error),
    }
}

fn config_file_content_hash(path: &Path) -> std::io::Result<u64> {
    let bytes = std::fs::read(path)?;
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    Ok(hasher.finish())
}
