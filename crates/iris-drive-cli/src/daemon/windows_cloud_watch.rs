#[derive(Debug, Clone)]
#[cfg_attr(not(windows), allow(dead_code))]
enum WindowsCloudRootChange {
    Upsert(String),
    Delete(String),
    Rename {
        old_path: String,
        new_path: String,
    },
    ValidateLocalState,
    Rescan {
        full: bool,
        recover_cached_deletes: bool,
    },
}

#[derive(Debug)]
enum WindowsCloudImportOutcome {
    Changed {
        root_cid: String,
        paths: Vec<String>,
    },
    Unchanged,
}

#[cfg(windows)]
fn start_windows_cloud_root_watch() -> Result<(
    Option<PathBuf>,
    Option<tokio::sync::mpsc::UnboundedReceiver<WindowsCloudRootChange>>,
    Option<notify::RecommendedWatcher>,
    Option<Value>,
)> {
    use notify::{RecursiveMode, Watcher};

    let home = dirs::home_dir().context("finding Windows profile directory")?;
    let root = home.join("Iris Drive");
    std::fs::create_dir_all(&root)
        .with_context(|| format!("creating Windows Cloud Files root {}", root.display()))?;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let callback_tx = tx.clone();
    let callback_root = root.clone();
    let mut watcher = notify::recommended_watcher(move |result| match result {
        Ok(event) => {
            for change in windows_cloud_changes_from_event(&callback_root, event) {
                let _ = callback_tx.send(change);
            }
        }
        Err(error) => {
            eprintln!("windows cloud root watch error: {error:#}");
        }
    })
    .context("creating Windows Cloud Files watcher")?;
    watcher
        .watch(&root, RecursiveMode::Recursive)
        .with_context(|| format!("watching Windows Cloud Files root {}", root.display()))?;
    let _ = tx.send(WindowsCloudRootChange::Rescan {
        full: true,
        recover_cached_deletes: windows_cloud_cached_delete_recovery_enabled(),
    });
    let periodic_tx = tx.clone();
    let _ = std::thread::Builder::new()
        .name("windows-cloud-local-state-validate".to_string())
        .spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(
                    WINDOWS_CLOUD_LOCAL_STATE_VALIDATE_INTERVAL_SECS,
                ));
                if periodic_tx
                    .send(windows_cloud_periodic_validate_change())
                    .is_err()
                {
                    break;
                }
            }
        });

    Ok((
        Some(root.clone()),
        Some(rx),
        Some(watcher),
        Some(json!({
            "root": root.display().to_string(),
            "watching": true,
        })),
    ))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn windows_cloud_periodic_validate_change() -> WindowsCloudRootChange {
    WindowsCloudRootChange::ValidateLocalState
}

#[cfg(windows)]
fn windows_cloud_changes_from_event(
    root: &Path,
    event: notify::Event,
) -> Vec<WindowsCloudRootChange> {
    use notify::event::{EventKind, ModifyKind, RenameMode};

    match event.kind {
        EventKind::Modify(ModifyKind::Name(RenameMode::Both)) if event.paths.len() >= 2 => {
            match (
                windows_cloud_relative_path(root, &event.paths[0]),
                windows_cloud_relative_path(root, &event.paths[1]),
            ) {
                (Some(old_path), Some(new_path)) => {
                    vec![WindowsCloudRootChange::Rename { old_path, new_path }]
                }
                _ => Vec::new(),
            }
        }
        EventKind::Modify(ModifyKind::Name(RenameMode::From)) | EventKind::Remove(_) => event
            .paths
            .iter()
            .filter_map(|path| windows_cloud_relative_path(root, path))
            .map(WindowsCloudRootChange::Delete)
            .collect(),
        EventKind::Modify(ModifyKind::Name(RenameMode::Both))
        | EventKind::Modify(ModifyKind::Name(RenameMode::To))
        | EventKind::Modify(ModifyKind::Name(RenameMode::Any))
        | EventKind::Modify(ModifyKind::Name(RenameMode::Other))
        | EventKind::Create(_)
        | EventKind::Modify(ModifyKind::Any)
        | EventKind::Modify(ModifyKind::Data(_))
        | EventKind::Modify(ModifyKind::Metadata(_))
        | EventKind::Modify(ModifyKind::Other)
        | EventKind::Any
        | EventKind::Other => event
            .paths
            .iter()
            .filter_map(|path| windows_cloud_relative_path(root, path))
            .map(WindowsCloudRootChange::Upsert)
            .collect(),
        EventKind::Access(_) => Vec::new(),
    }
}

#[allow(clippy::too_many_lines)]
async fn import_windows_cloud_root_changes_and_publish(
    client: &nostr_sdk::Client,
    config_dir: &Path,
    sync_root: &Path,
    changes: Vec<WindowsCloudRootChange>,
    direct_roots: &mut DirectRootExchange,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<WindowsCloudImportOutcome> {
    let _config_lock = ConfigMutationLock::acquire(config_dir).await?;
    let daemon = Daemon::open(config_dir).context("opening daemon for Windows Cloud Files root")?;
    let visible = iris_drive_core::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .context("building Windows Cloud Files provider root")?;
    let provider = HashTreeProviderFs::open(daemon.tree_handle(), visible.root_cid.clone()).await?;
    let before = provider.current_root().await;
    let placeholder_paths = load_windows_cloud_provider_path_cache(config_dir);
    let previous_local_state = load_windows_cloud_local_state(config_dir);
    let protected_local_mutations = windows_cloud_protected_local_mutation_paths(&changes);
    let mut changed_paths = BTreeSet::new();
    let mut tombstone_paths = BTreeSet::new();
    for path in prune_ignored_provider_paths(&provider).await? {
        changed_paths.insert(path);
    }
    let expected_entries =
        windows_cloud_provider_expected_entries(daemon.tree(), &provider.current_root().await)
            .await?;
    let expected_paths: BTreeSet<String> = expected_entries
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    for path in windows_cloud_remove_stale_synced_local_items(
        sync_root,
        &expected_paths,
        &previous_local_state,
        &protected_local_mutations,
    ) {
        changed_paths.insert(path);
    }

    for change in changes {
        match change {
            WindowsCloudRootChange::Upsert(path) => {
                if apply_windows_cloud_upsert(&provider, sync_root, &path, &placeholder_paths)
                    .await?
                {
                    changed_paths.insert(path);
                }
            }
            WindowsCloudRootChange::Delete(path) => {
                if consume_windows_cloud_cleanup_delete_marker(config_dir, &path) {
                    continue;
                }
                if apply_windows_cloud_delete_if_local_missing(&provider, sync_root, &path).await? {
                    changed_paths.insert(path.clone());
                    tombstone_paths.insert(path);
                }
            }
            WindowsCloudRootChange::Rename { old_path, new_path } => {
                if apply_windows_cloud_rename(
                    &provider,
                    sync_root,
                    &old_path,
                    &new_path,
                    &placeholder_paths,
                )
                .await?
                {
                    changed_paths.insert(old_path.clone());
                    changed_paths.insert(new_path);
                    tombstone_paths.insert(old_path);
                }
            }
            WindowsCloudRootChange::ValidateLocalState => {}
            WindowsCloudRootChange::Rescan {
                full,
                recover_cached_deletes,
            } => {
                for path in windows_cloud_rescan_missing_cached_provider_paths(
                    sync_root,
                    &placeholder_paths,
                    recover_cached_deletes,
                )? {
                    if consume_windows_cloud_cleanup_delete_marker(config_dir, &path) {
                        continue;
                    }
                    if apply_windows_cloud_delete(&provider, &path).await? {
                        changed_paths.insert(path.clone());
                        tombstone_paths.insert(path);
                    }
                }
                let local_paths = if full {
                    windows_cloud_local_projected_paths(sync_root)?
                } else {
                    windows_cloud_recent_local_projected_paths(sync_root)?
                };
                for path in local_paths {
                    if apply_windows_cloud_upsert(&provider, sync_root, &path, &placeholder_paths)
                        .await?
                    {
                        changed_paths.insert(path);
                    }
                }
            }
        }
    }
    for path in windows_cloud_missing_previous_local_state_paths(
        sync_root,
        &previous_local_state,
        &protected_local_mutations,
    )? {
        if apply_windows_cloud_delete(&provider, &path).await? {
            changed_paths.insert(path.clone());
            tombstone_paths.insert(path);
        }
    }

    let root = provider.current_root().await;
    let current_entries = windows_cloud_provider_expected_entries(daemon.tree(), &root).await?;
    let snapshot_protected_paths = BTreeSet::new();
    drop(provider);
    drop(daemon);

    if root == before {
        write_windows_cloud_local_state(
            config_dir,
            sync_root,
            &current_entries,
            &previous_local_state,
            &snapshot_protected_paths,
        );
        return Ok(WindowsCloudImportOutcome::Unchanged);
    }

    import_mount_root_and_publish_with_tombstone_paths(
        client,
        config_dir,
        root.clone(),
        Some(before),
        Some(&tombstone_paths),
        direct_roots,
        fips_blocks,
    )
    .await
    .context("publishing Windows Cloud Files root")?;

    // Write synced local-state only after the config root has advanced. If this
    // cache wins the race against provider list, the Windows app can mistake a
    // fresh local mutation for stale projection residue and prune it.
    write_windows_cloud_local_state(
        config_dir,
        sync_root,
        &current_entries,
        &previous_local_state,
        &snapshot_protected_paths,
    );

    Ok(WindowsCloudImportOutcome::Changed {
        root_cid: root.to_string(),
        paths: changed_paths.into_iter().collect(),
    })
}

fn windows_cloud_protected_local_mutation_paths(
    changes: &[WindowsCloudRootChange],
) -> BTreeSet<String> {
    let mut protected = BTreeSet::new();
    for change in changes {
        match change {
            WindowsCloudRootChange::Upsert(path) => {
                windows_cloud_insert_path_and_ancestors(&mut protected, path);
            }
            WindowsCloudRootChange::Rename { new_path, .. } => {
                windows_cloud_insert_path_and_ancestors(&mut protected, new_path);
            }
            WindowsCloudRootChange::Delete(_)
            | WindowsCloudRootChange::ValidateLocalState
            | WindowsCloudRootChange::Rescan { .. } => {}
        }
    }
    protected
}

fn windows_cloud_insert_path_and_ancestors(paths: &mut BTreeSet<String>, path: &str) {
    let Ok(mut path) = normalize_provider_path(path) else {
        return;
    };
    while !path.is_empty() {
        paths.insert(path.clone());
        let Some((parent, _name)) = path.rsplit_once('/') else {
            break;
        };
        path = parent.to_string();
    }
}
