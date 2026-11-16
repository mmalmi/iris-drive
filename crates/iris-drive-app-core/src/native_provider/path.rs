use super::{
    BTreeMap, Path, compose_provider_path, ensure_daemon_available_for_provider_mutation,
    import_provider_mutation, json, native_provider, native_provider_runtime,
    normalize_provider_document_path, normalize_provider_parent_path, normalize_provider_path,
    optional_normalized_provider_path, provider_entries, sanitized_provider_file_name,
    split_provider_path, unique_provider_path,
};

pub(crate) fn native_provider_compose_path_json(
    parent_path: &str,
    display_name: &str,
) -> serde_json::Value {
    match run_native_provider_compose_path(parent_path, display_name) {
        Ok(value) => value,
        Err(error) => json!({
            "path": "",
            "parent_path": "",
            "display_name": "",
            "error": format!("{error:#}"),
        }),
    }
}

pub(super) fn run_native_provider_normalize_path(path: &str) -> anyhow::Result<serde_json::Value> {
    let path = normalize_provider_document_path(path)?;
    let (parent_path, display_name) = split_provider_path(&path)?;
    Ok(json!({
        "parent_path": parent_path,
        "display_name": display_name,
        "path": path,
        "error": "",
    }))
}

fn run_native_provider_compose_path(
    parent_path: &str,
    display_name: &str,
) -> anyhow::Result<serde_json::Value> {
    let (parent_path, display_name, path) = compose_provider_path(parent_path, display_name)?;
    Ok(json!({
        "parent_path": parent_path,
        "display_name": display_name,
        "path": path,
        "error": "",
    }))
}

pub(super) fn run_native_provider_resolve_path(
    data_dir: &str,
    parent_path: &str,
    display_name: &str,
    excluding_path: &str,
) -> anyhow::Result<serde_json::Value> {
    let runtime = native_provider_runtime()?;
    runtime.block_on(async {
        let parent_path = normalize_provider_parent_path(parent_path)?;
        let display_name = sanitized_provider_file_name(display_name);
        let excluding_path = optional_normalized_provider_path(excluding_path)?;
        let (_daemon, provider, _visible_root) = native_provider(data_dir).await?;
        let modified_at_by_path = BTreeMap::new();
        let entries = provider_entries(&provider, &modified_at_by_path).await?;
        let path = unique_provider_path(
            &entries,
            &parent_path,
            &display_name,
            excluding_path.as_deref(),
        );
        let (resolved_parent_path, resolved_display_name) = split_provider_path(&path)?;
        Ok(json!({
            "parent_path": resolved_parent_path,
            "display_name": resolved_display_name,
            "path": path,
            "error": "",
        }))
    })
}

pub(crate) fn native_provider_reimport(
    data_dir: &str,
    path: &str,
    source_path: Option<&Path>,
    directory: bool,
) -> anyhow::Result<serde_json::Value> {
    use iris_drive_core::provider::reimport::{Reimport, reimport_item};

    ensure_daemon_available_for_provider_mutation(data_dir)?;
    native_provider_runtime()?.block_on(async {
        let path = normalize_provider_path(path)?;
        let lock =
            iris_drive_core::config_lock::ConfigMutationLock::acquire(Path::new(data_dir)).await?;
        let (mut daemon, provider, root) = native_provider(data_dir).await?;
        let outcome = reimport_item(
            daemon.tree(),
            &provider,
            &path,
            directory,
            source_path,
            || iris_drive_core::shared_content::ensure_shared_path_writable(daemon.config(), &path),
        )
        .await?;
        if matches!(outcome, Reimport::Created) {
            import_provider_mutation(&mut daemon, &provider, &path, Some(root), lock).await?;
        }
        Ok(json!(outcome))
    })
}
