use std::path::Path;

use iris_drive_core::provider::delete_provider_path;
use serde_json::json;

use super::{
    ensure_daemon_available_for_provider_mutation, import_provider_mutation, native_provider,
    native_provider_runtime, normalize_provider_path,
};

pub(crate) fn native_provider_delete_json(data_dir: &str, path: &str) -> serde_json::Value {
    match run_native_provider_delete(data_dir, path) {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    }
}

fn run_native_provider_delete(data_dir: &str, path: &str) -> anyhow::Result<serde_json::Value> {
    ensure_daemon_available_for_provider_mutation(data_dir)?;
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let path = normalize_provider_path(path)?;
        let config_lock =
            iris_drive_core::config_lock::ConfigMutationLock::acquire(Path::new(data_dir)).await?;
        let (mut daemon, provider, visible_root) = native_provider(data_dir).await?;
        if let Some(id) = iris_drive_core::shared_owner_return::queue_shared_file_return(
            &daemon,
            &path,
            &config_lock,
        )
        .await?
        {
            return Ok(json!({
                "return_request": id,
                "error": "The file stays shared until its owner has a verified saved copy."
            }));
        }
        delete_provider_path(&provider, &path).await?;
        import_provider_mutation(
            &mut daemon,
            &provider,
            &path,
            Some(visible_root),
            config_lock,
        )
        .await
    })
}
