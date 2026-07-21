use std::path::Path;

use iris_drive_core::AppConfig;
use iris_drive_core::paths::config_path_in;

use super::config_file_content_hash;

#[derive(Debug, Clone, PartialEq, Eq)]
struct NativeConfigFileFingerprint {
    len: u64,
    modified: Option<std::time::SystemTime>,
    content_hash: Option<u64>,
}

#[derive(Debug, Default)]
pub(super) struct NativeAppConfigCache {
    fingerprint: Option<NativeConfigFileFingerprint>,
    config: Option<AppConfig>,
}

impl NativeAppConfigCache {
    pub(super) fn load_with_change(
        &mut self,
        config_dir: &Path,
    ) -> Result<(AppConfig, bool), String> {
        let config_path = config_path_in(config_dir);
        let fingerprint = native_config_file_fingerprint(&config_path)
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

fn native_config_file_fingerprint(path: &Path) -> std::io::Result<NativeConfigFileFingerprint> {
    match std::fs::metadata(path) {
        Ok(metadata) => Ok(NativeConfigFileFingerprint {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            content_hash: Some(config_file_content_hash(path)?),
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(NativeConfigFileFingerprint {
                len: 0,
                modified: None,
                content_hash: None,
            })
        }
        Err(error) => Err(error),
    }
}
