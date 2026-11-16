#[allow(clippy::wildcard_imports)]
use super::*;
use iris_drive_core::provider::rename_provider_path as shared_rename_provider_path;
use iris_drive_core::provider::{
    ProviderListEntry, compose_provider_path, normalize_provider_document_path,
    normalize_provider_parent_path, normalize_provider_path,
    provider_entry_is_probable_os_placeholder, provider_file_probable_os_placeholder_family,
    provider_list_summary, provider_write_is_probable_os_placeholder, sanitized_provider_file_name,
    split_provider_path, unique_provider_path,
};
pub(crate) use iris_drive_core::provider::{
    create_provider_dir, delete_provider_path, write_provider_file,
};

use crate::provider_staging::{
    ProviderStagedRoot, read_provider_staging, unix_now_seconds, write_provider_staging,
};

mod commands;
mod provider_cache;
mod provider_retry;
pub(crate) use commands::*;
use provider_cache::hydrate_provider_cache;
use provider_retry::{ProviderCommandState, prepare_provider_command, retry_provider_operation};

pub(crate) fn cmd_drives(config_dir: &std::path::Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    if config.drives.is_empty() {
        println!("(no drives — run `idrive init`)");
        return Ok(());
    }
    for d in &config.drives {
        println!(
            "{:<24}  {:<7}  {:<32}  {}",
            d.drive_id,
            drive_role_label(d.role),
            short_scope_id(&d.root_scope_id),
            d.display_name,
        );
    }
    Ok(())
}

fn short_scope_id(scope_id: &str) -> String {
    if scope_id.len() <= 16 {
        return scope_id.to_owned();
    }
    format!("{}…{}", &scope_id[..8], &scope_id[scope_id.len() - 8..])
}

pub(crate) fn cmd_import(config_dir: &std::path::Path, source_dir: &std::path::Path) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let mut daemon = Daemon::open(config_dir)
            .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
        let report = daemon
            .import_source_dir(source_dir)
            .await
            .with_context(|| format!("importing {}", source_dir.display()))?;
        let reported_source = report.source_dir.as_deref().unwrap_or(source_dir);
        println!(
            "{}",
            json!({
                "source_dir": reported_source.display().to_string(),
                "root_cid": report.root_cid,
                "drive_iris_to_url": drive_iris_to_url_for_primary_drive(daemon.config()),
                "files_iris_to_url": drive_iris_to_url_for_primary_drive(daemon.config()),
                "snapshot_url": drive_iris_to_snapshot_url_for_root(&report.root_cid),
                "permalink_url": drive_iris_to_snapshot_url_for_root(&report.root_cid),
                "local_gateway": local_gateway_urls_for_root(
                    Some(&report.root_cid),
                    DEFAULT_GATEWAY_PORT,
                    daemon.config().local_nhash_resolver_enabled,
                    crate::status::current_app_key_npub(daemon.config()).as_deref(),
                ),
                "file_count": report.file_count,
                "top_level_entries": report.top_level_entries,
                "blocks_dir": daemon.blocks_dir().display().to_string(),
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}

pub(crate) fn cmd_list(config_dir: &std::path::Path, at: usize) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let daemon = Daemon::open(config_dir)
            .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
        let config = daemon.config();
        let drive = config
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .ok_or_else(|| anyhow::anyhow!("primary drive missing"))?;
        let account = config
            .profile
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no account; run `idrive init` first"))?;
        let authorized = authorized_app_key_pubkeys(account);
        if at == 0 {
            let (projected, recovering) = daemon.provider_view().await?;
            print_drive_list(
                drive,
                at,
                authorized.len(),
                projected.app_key_roots_present,
                &projected.view,
                recovering,
            );
            return Ok::<_, anyhow::Error>(());
        }

        let merge_app_keys = iris_drive_core::projection::merge_app_key_pubkeys(account, drive);

        // Fetch each trusted AppKey root's tree + tombstones from htree.
        // With `--at N`, this AppKey's own root walks back N revisions
        // via the `.hashtree/prev` chain; other AppKeys' roots stay at their
        // current state.
        let mut snapshots_data = Vec::new();
        for app_key_pubkey in &merge_app_keys {
            let Some(root) = drive.app_key_roots.get(app_key_pubkey) else {
                continue; // AppKey has not published its root yet
            };
            let mut cid = Cid::parse(&root.root_cid)
                .with_context(|| format!("parsing root CID for AppKey {app_key_pubkey}"))?;
            if at > 0 && *app_key_pubkey == account.app_key_pubkey {
                cid = iris_drive_core::history::revision_at(daemon.tree(), &cid, at)
                    .await
                    .with_context(|| format!("revision -{at} not in chain"))?;
            }
            let (files, tombstones) = walk_app_key_tree(daemon.tree(), &cid).await?;
            snapshots_data.push((app_key_pubkey.clone(), root.clone(), files, tombstones));
        }

        let merge_app_key_refs: Vec<&str> = merge_app_keys.iter().map(String::as_str).collect();
        let snapshots: Vec<AppKeySnapshot> = snapshots_data
            .iter()
            .map(|(pk, root, files, tombs)| AppKeySnapshot {
                app_key_pubkey: pk.as_str(),
                root,
                files: files.clone(),
                tombstones: tombs.clone(),
                path_kind_replacements: None,
            })
            .collect();

        let view = merge_drives(&merge_app_key_refs, &snapshots);

        print_drive_list(drive, at, authorized.len(), snapshots.len(), &view, false);
        Ok::<_, anyhow::Error>(())
    })
}

fn print_drive_list(
    drive: &Drive,
    at: usize,
    authorized_app_key_count: usize,
    app_key_roots_present: usize,
    view: &iris_drive_core::merge::MergedView,
    recovering: bool,
) {
    println!(
        "{}",
        json!({
            "drive_id": drive.drive_id,
            "at_revision": at,
            "waiting_for_source_blocks": recovering,
            "authorized_app_keys": authorized_app_key_count,
            "app_key_roots_present": app_key_roots_present,
            "files": view
                .files
                .iter()
                .map(|e| json!({
                    "path": e.path,
                    "size": e.size,
                    "sha256": e.whole_file_hash.unwrap_or(e.hash).map(|byte| format!("{byte:02x}")).join(""),
                    "source_app_key_pubkey": (!e.source_app_key_pubkey.is_empty()).then_some(&e.source_app_key_pubkey),
                    "modified_at": e.modified_at,
                    "published_at": (!e.source_app_key_pubkey.is_empty()).then_some(e.published_at),
                }))
                .collect::<Vec<_>>(),
            "suppressed_by_tombstone": (!recovering).then_some(&view.suppressed_by_tombstone),
        })
    );
}

#[allow(clippy::too_many_lines)]
pub(crate) fn cmd_provider(config_dir: &std::path::Path, command: ProviderCmd) -> Result<()> {
    if let ProviderCmd::NormalizePath { path } = command {
        let path = normalize_provider_document_path(&path)?;
        let (parent_path, display_name) = split_provider_path(&path)?;
        println!(
            "{}",
            json!({
                "parent_path": parent_path,
                "display_name": display_name,
                "path": path,
                "error": "",
            })
        );
        return Ok(());
    }
    if let ProviderCmd::ComposePath {
        parent_path,
        display_name,
    } = command
    {
        let (parent_path, display_name, path) = compose_provider_path(&parent_path, &display_name)?;
        println!(
            "{}",
            json!({
                "parent_path": parent_path,
                "display_name": display_name,
                "path": path,
                "error": "",
            })
        );
        return Ok(());
    }

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let ProviderCommandState {
            daemon,
            mutation_lock,
            staged_root,
            visible,
            visible_view,
        } = retry_provider_operation("provider preflight", || {
            prepare_provider_command(config_dir, &command)
        })
        .await?;
        let visible_view = visible_view.as_ref();
        let tombstone_base_root = if provider_command_is_mutation(&command) {
            match staged_root.as_ref() {
                Some(staged) => staged
                    .tombstone_base_root()?
                    .or_else(|| Some(visible.root_cid.clone())),
                None => Some(visible.root_cid.clone()),
            }
        } else {
            None
        };
        let phase = std::time::Instant::now();
        let provider =
            HashTreeProviderFs::open(daemon.tree_handle(), visible.root_cid.clone()).await?;
        tracing::debug!(
            elapsed_ms = phase.elapsed().as_millis(),
            "provider command opened provider root"
        );

        match command {
            ProviderCmd::List => {
                let phase = std::time::Instant::now();
                let visible_view =
                    visible_view.expect("list command should have prebuilt merged view");
                let modified_at_by_path = provider_modified_at_index(visible_view);
                let entries =
                    provider_entries(daemon.tree(), &visible.root_cid, &modified_at_by_path)
                        .await?;
                let summary = provider_list_summary(provider.anchor().await.as_str(), &entries);
                tracing::debug!(
                    elapsed_ms = phase.elapsed().as_millis(),
                    "provider command listed entries"
                );
                println!(
                    "{}",
                    json!({
                        "anchor": provider.anchor().await.as_str(),
                        "root_cid": visible.root_cid.to_string(),
                        "file_count": summary.file_count,
                        "top_level_entries": top_level_entry_count(&entries),
                        "visible_file_bytes": summary.visible_file_bytes,
                        "share_action_paths": iris_drive_core::share_folders::share_action_folders(
                            daemon.config(),
                            &summary.directory_paths,
                        ),
                        "directory_paths": summary.directory_paths,
                        "change_key": summary.change_key,
                        "entries": entries,
                    })
                );
            }
            ProviderCmd::ResolvePath {
                base_root_cid: _,
                parent_path,
                display_name,
                excluding_path,
            } => {
                let parent_path = normalize_provider_parent_path(&parent_path)?;
                let display_name = sanitized_provider_file_name(&display_name);
                let excluding_path = excluding_path
                    .as_deref()
                    .map(normalize_provider_path)
                    .transpose()?;
                let modified_at_by_path = visible_view
                    .map(provider_modified_at_index)
                    .unwrap_or_default();
                let entries =
                    provider_entries(daemon.tree(), &visible.root_cid, &modified_at_by_path)
                        .await?;
                let path = unique_provider_path(
                    &entries,
                    &parent_path,
                    &display_name,
                    excluding_path.as_deref(),
                );
                let (resolved_parent_path, resolved_display_name) = split_provider_path(&path)?;
                println!(
                    "{}",
                    json!({
                        "parent_path": resolved_parent_path,
                        "display_name": resolved_display_name,
                        "path": path,
                        "root_cid": visible.root_cid.to_string(),
                        "error": "",
                    })
                );
            }
            ProviderCmd::NormalizePath { .. } | ProviderCmd::ComposePath { .. } => {
                unreachable!("handled before opening provider state")
            }
            ProviderCmd::Read { path, output } => {
                let path = normalize_provider_path(&path)?;
                let item = provider.item(&path).await?;
                if item.kind == ItemKind::Directory {
                    anyhow::bail!("cannot read directory: {path}");
                }
                let bytes = provider.read(&path, 0, item.size).await?;
                if let Some(parent) = output.parent() {
                    std::fs::create_dir_all(parent)
                        .with_context(|| format!("creating {}", parent.display()))?;
                }
                std::fs::write(&output, bytes)
                    .with_context(|| format!("writing {}", output.display()))?;
                println!(
                    "{}",
                    json!({
                        "path": path,
                        "output": output.display().to_string(),
                        "size": item.size,
                    })
                );
            }
            ProviderCmd::HydrateCache { dir } => {
                let phase = std::time::Instant::now();
                let modified_at_by_path = BTreeMap::new();
                let entries =
                    provider_entries(daemon.tree(), &visible.root_cid, &modified_at_by_path)
                        .await?;
                let report = hydrate_provider_cache(&provider, &entries, &dir).await?;
                tracing::debug!(
                    elapsed_ms = phase.elapsed().as_millis(),
                    "provider command hydrated private cache"
                );
                println!(
                    "{}",
                    json!({
                        "target_dir": dir.display().to_string(),
                        "file_count": report.file_count,
                        "directory_count": report.directory_count,
                        "written": report.written,
                        "updated": report.updated,
                        "unchanged": report.unchanged,
                        "skipped": report.skipped,
                        "changed": report.written + report.updated,
                    })
                );
            }
            ProviderCmd::Reimport {
                path,
                source,
                directory,
            } => {
                use iris_drive_core::provider::reimport::{Reimport, reimport_item};

                let path = normalize_provider_path(&path)?;
                let outcome = reimport_item(
                    daemon.tree(),
                    &provider,
                    &path,
                    directory,
                    source.as_deref(),
                    || {
                        iris_drive_core::shared_content::ensure_shared_path_writable(
                            daemon.config(),
                            &path,
                        )
                    },
                )
                .await?;
                if matches!(outcome, Reimport::Created) {
                    stage_provider_mutation(
                        &daemon,
                        mutation_lock,
                        &provider,
                        &path,
                        tombstone_base_root,
                        provider_mutation_tombstone_paths(
                            staged_root.as_ref(),
                            BTreeSet::from([path.clone()]),
                        ),
                    )
                    .await?;
                }
                println!("{}", json!(outcome));
            }
            ProviderCmd::Write {
                base_root_cid: _,
                path,
                source,
            } => {
                let path = normalize_provider_path(&path)?;
                let bytes = std::fs::read(&source)
                    .with_context(|| format!("reading {}", source.display()))?;
                if provider_file_probable_os_placeholder_family(&path, bytes.len() as u64).is_some()
                {
                    let entries =
                        provider_entries(daemon.tree(), &visible.root_cid, &BTreeMap::new())
                            .await?;
                    if provider_write_is_probable_os_placeholder(&entries, &path, &bytes) {
                        anyhow::bail!("refusing probable FileProvider placeholder copy: {path}");
                    }
                }
                let phase = std::time::Instant::now();
                write_provider_file(&provider, &path, &bytes).await?;
                tracing::debug!(
                    elapsed_ms = phase.elapsed().as_millis(),
                    "provider command wrote file"
                );
                let report = stage_provider_mutation(
                    &daemon,
                    mutation_lock,
                    &provider,
                    &path,
                    tombstone_base_root.clone(),
                    provider_mutation_tombstone_paths(
                        staged_root.as_ref(),
                        BTreeSet::from([path.clone()]),
                    ),
                )
                .await?;
                println!("{report}");
            }
            ProviderCmd::Mkdir {
                base_root_cid: _,
                path,
            } => {
                let path = normalize_provider_path(&path)?;
                let phase = std::time::Instant::now();
                create_provider_dir(&provider, &path).await?;
                tracing::debug!(
                    elapsed_ms = phase.elapsed().as_millis(),
                    "provider command created directory"
                );
                let report = stage_provider_mutation(
                    &daemon,
                    mutation_lock,
                    &provider,
                    &path,
                    tombstone_base_root.clone(),
                    provider_mutation_tombstone_paths(
                        staged_root.as_ref(),
                        BTreeSet::from([path.clone()]),
                    ),
                )
                .await?;
                println!("{report}");
            }
            ProviderCmd::Delete {
                base_root_cid: _,
                path,
            } => {
                let path = normalize_provider_path(&path)?;
                let deleted_paths = provider_delete_tombstone_paths(&provider, &path).await?;
                let phase = std::time::Instant::now();
                delete_provider_path(&provider, &path).await?;
                tracing::debug!(
                    elapsed_ms = phase.elapsed().as_millis(),
                    "provider command deleted path"
                );
                let report = stage_provider_mutation(
                    &daemon,
                    mutation_lock,
                    &provider,
                    &path,
                    tombstone_base_root.clone(),
                    provider_mutation_tombstone_paths(staged_root.as_ref(), deleted_paths),
                )
                .await?;
                println!("{report}");
            }
            ProviderCmd::Rename {
                base_root_cid: _,
                old_path,
                new_path,
            } => {
                let old_path = normalize_provider_path(&old_path)?;
                let new_path = normalize_provider_path(&new_path)?;
                let phase = std::time::Instant::now();
                rename_provider_path(&provider, &old_path, &new_path).await?;
                tracing::debug!(
                    elapsed_ms = phase.elapsed().as_millis(),
                    "provider command renamed path"
                );
                let report = stage_provider_mutation(
                    &daemon,
                    mutation_lock,
                    &provider,
                    &new_path,
                    tombstone_base_root.clone(),
                    provider_mutation_tombstone_paths(
                        staged_root.as_ref(),
                        BTreeSet::from([old_path.clone(), new_path.clone()]),
                    ),
                )
                .await?;
                println!("{report}");
            }
        }

        Ok::<_, anyhow::Error>(())
    })
}

fn provider_command_is_mutation(command: &ProviderCmd) -> bool {
    matches!(
        command,
        ProviderCmd::Write { .. }
            | ProviderCmd::Reimport { .. }
            | ProviderCmd::Mkdir { .. }
            | ProviderCmd::Delete { .. }
            | ProviderCmd::Rename { .. }
    )
}

fn provider_command_needs_merged_view(command: &ProviderCmd) -> bool {
    matches!(
        command,
        ProviderCmd::List
            | ProviderCmd::ResolvePath {
                base_root_cid: None,
                ..
            }
    )
}

fn provider_command_base_root_cid(command: &ProviderCmd) -> Result<Option<Cid>> {
    let raw = match command {
        ProviderCmd::Write { base_root_cid, .. }
        | ProviderCmd::Mkdir { base_root_cid, .. }
        | ProviderCmd::Delete { base_root_cid, .. }
        | ProviderCmd::Rename { base_root_cid, .. }
        | ProviderCmd::ResolvePath { base_root_cid, .. } => base_root_cid.as_deref(),
        _ => None,
    };
    raw.map(Cid::parse)
        .transpose()
        .context("parsing provider base root cid")
}

async fn provider_entries(
    tree: &HashTree<FsBlobStore>,
    root: &Cid,
    modified_at_by_path: &BTreeMap<String, i64>,
) -> Result<Vec<ProviderListEntry>> {
    let mut entries = Vec::new();
    let mut stack = vec![(String::new(), root.clone())];
    while let Some((parent, dir_cid)) = stack.pop() {
        let mut children = tree.list_directory_required(&dir_cid).await?;
        children.sort_by(|a, b| a.name.cmp(&b.name));
        for child in children {
            let path = if parent.is_empty() {
                child.name.clone()
            } else {
                format!("{parent}/{}", child.name)
            };
            let cid = Cid {
                hash: child.hash,
                key: child.key,
            };
            let kind = match child.link_type {
                LinkType::Dir | LinkType::Fanout => {
                    stack.push((path.clone(), cid.clone()));
                    "directory"
                }
                LinkType::Blob | LinkType::File => "file",
            };
            entries.push(ProviderListEntry {
                modified_at: modified_at_by_path.get(&path).copied(),
                path,
                parent_path: parent.clone(),
                display_name: child.name,
                kind,
                size: child.size,
                version: cid.to_string(),
            });
        }
    }
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let all_entries = entries.clone();
    entries.retain(|entry| !provider_entry_is_probable_os_placeholder(&all_entries, entry));
    Ok(entries)
}

fn provider_modified_at_index(
    view: &iris_drive_core::projection::PrimaryMergedView,
) -> BTreeMap<String, i64> {
    let mut index = BTreeMap::new();
    for entry in &view.view.files {
        let Some(modified_at) = entry.modified_at else {
            continue;
        };
        remember_provider_modified_at(&mut index, &entry.path, modified_at);
        let mut path = entry.path.as_str();
        while let Some((parent, _name)) = path.rsplit_once('/') {
            remember_provider_modified_at(&mut index, parent, modified_at);
            path = parent;
        }
    }
    index
}

fn remember_provider_modified_at(index: &mut BTreeMap<String, i64>, path: &str, modified_at: i64) {
    if path.is_empty() || modified_at < 946_684_800 {
        return;
    }
    index
        .entry(path.to_owned())
        .and_modify(|existing| *existing = (*existing).max(modified_at))
        .or_insert(modified_at);
}

fn top_level_entry_count(entries: &[ProviderListEntry]) -> usize {
    entries
        .iter()
        .filter(|entry| entry.parent_path.is_empty())
        .count()
}

fn provider_mutation_tombstone_paths(
    staged: Option<&ProviderStagedRoot>,
    changed_paths: BTreeSet<String>,
) -> BTreeSet<String> {
    let mut tombstone_paths = staged
        .map(|staged| staged.tombstone_paths.clone())
        .unwrap_or_default();
    tombstone_paths.extend(changed_paths);
    tombstone_paths
}

async fn provider_delete_tombstone_paths(
    provider: &HashTreeProviderFs<FsBlobStore>,
    path: &str,
) -> Result<BTreeSet<String>> {
    let mut tombstone_paths = BTreeSet::from([path.to_string()]);
    let mut stack = vec![path.to_string()];
    while let Some(current) = stack.pop() {
        let item = match provider.item(&current).await {
            Ok(item) => item,
            Err(hashtree_provider::ProviderError::NotFound) => continue,
            Err(error) => return Err(error.into()),
        };
        if item.kind != ItemKind::Directory {
            continue;
        }
        for child in provider.read_dir(&current).await? {
            tombstone_paths.insert(child.id.clone());
            stack.push(child.id);
        }
    }
    Ok(tombstone_paths)
}

pub(crate) async fn rename_provider_path(
    provider: &HashTreeProviderFs<FsBlobStore>,
    old_path: &str,
    new_path: &str,
) -> Result<()> {
    if old_path == new_path {
        return Ok(());
    }
    shared_rename_provider_path(provider, old_path, new_path).await
}

async fn stage_provider_mutation(
    daemon: &Daemon,
    mutation_lock: Option<ConfigMutationLock>,
    provider: &HashTreeProviderFs<FsBlobStore>,
    changed_path: &str,
    tombstone_base_root: Option<Cid>,
    tombstone_paths: BTreeSet<String>,
) -> Result<Value> {
    let phase = std::time::Instant::now();
    let root = provider.current_root().await;
    tracing::debug!(
        elapsed_ms = phase.elapsed().as_millis(),
        "provider command read current root"
    );
    let phase = std::time::Instant::now();
    let report = provider_root_report(daemon.tree(), &root).await?;
    let staged = ProviderStagedRoot {
        root_cid: root.to_string(),
        tombstone_base_root_cid: tombstone_base_root.as_ref().map(ToString::to_string),
        tombstone_paths: tombstone_paths.clone(),
        updated_at: unix_now_seconds(),
    };
    write_provider_staging(daemon.config_dir(), &staged)?;
    drop(mutation_lock);
    let wake_payload = json!({
        "root_cid": report.root_cid,
        "file_count": report.file_count,
        "top_level_entries": report.top_level_entries,
        "staged": true,
    });
    let woke = match wake_provider_root_publisher(daemon.config_dir(), Some(wake_payload)).await {
        Ok(woke) => woke,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "provider root wake failed; importing staged root immediately"
            );
            false
        }
    };
    let committed = if woke {
        None
    } else {
        // A foreground publisher and other provider writes may be running now.
        // Each attempt imports the current stage exactly once under its own lock.
        retry_provider_operation("provider import", || {
            crate::daemon::import_staged_provider_root(daemon.config_dir())
        })
        .await?
    };
    let (root_cid, file_count, top_level_entries) = if let Some(report) = committed {
        let wake_payload = json!({
            "root_cid": report.root_cid,
            "file_count": report.file_count,
            "top_level_entries": report.top_level_entries,
        });
        if let Err(error) =
            wake_provider_root_publisher(daemon.config_dir(), Some(wake_payload)).await
        {
            tracing::warn!(
                error = %error,
                "provider root wake failed after immediate provider import"
            );
        }
        (
            report.root_cid,
            u64::try_from(report.file_count).unwrap_or(u64::MAX),
            report.top_level_entries,
        )
    } else {
        (report.root_cid, report.file_count, report.top_level_entries)
    };
    iris_drive_core::paths::touch_provider_root_signal_in(daemon.config_dir())
        .context("signaling provider root change")?;
    tracing::debug!(
        elapsed_ms = phase.elapsed().as_millis(),
        staged = woke,
        "provider command staged provider root"
    );
    Ok(json!({
            "path": changed_path,
            "root_cid": root_cid,
            "file_count": file_count,
            "top_level_entries": top_level_entries,
            "staged": woke,
    }))
}

async fn wake_provider_root_publisher(config_dir: &Path, payload: Option<Value>) -> Result<bool> {
    use tokio::io::AsyncWriteExt as _;

    let wake_path = iris_drive_core::paths::provider_root_wake_path_in(config_dir);
    let raw = match tokio::fs::read_to_string(&wake_path).await {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "reading provider root wake endpoint {}",
                    wake_path.display()
                )
            });
        }
    };
    let value: Value = serde_json::from_str(&raw).with_context(|| {
        format!(
            "parsing provider root wake endpoint {}",
            wake_path.display()
        )
    })?;
    let port = value
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .ok_or_else(|| anyhow::anyhow!("provider root wake endpoint missing port"))?;
    let mut stream = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        tokio::net::TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port)),
    )
    .await
    .context("provider root wake timed out")?
    .with_context(|| format!("connecting to provider root wake endpoint on port {port}"))?;
    if let Some(payload) = payload {
        let bytes = serde_json::to_vec(&payload)?;
        stream
            .write_all(&bytes)
            .await
            .context("writing provider root wake payload")?;
        let _ = stream.shutdown().await;
    }
    Ok(true)
}

struct ProviderRootReport {
    root_cid: String,
    file_count: u64,
    top_level_entries: usize,
}

async fn provider_root_report(
    tree: &HashTree<FsBlobStore>,
    root: &Cid,
) -> Result<ProviderRootReport> {
    let entries = provider_entries(tree, root, &BTreeMap::new()).await?;
    let summary = provider_list_summary(&root.to_string(), &entries);
    Ok(ProviderRootReport {
        root_cid: root.to_string(),
        file_count: summary.file_count,
        top_level_entries: top_level_entry_count(&entries),
    })
}

#[cfg(test)]
mod provider_mutation_tests {
    use hashtree_provider::ProviderFs;

    use super::{rename_provider_path, write_provider_file};

    #[tokio::test]
    async fn rename_same_path_is_a_noop() {
        let store = tempfile::tempdir().unwrap();
        let tree = std::sync::Arc::new(hashtree_core::HashTree::new(
            hashtree_core::HashTreeConfig::new(std::sync::Arc::new(
                hashtree_fs::FsBlobStore::new(store.path()).unwrap(),
            ))
            .public(),
        ));
        let provider = hashtree_provider::HashTreeProviderFs::fresh(tree)
            .await
            .unwrap();
        write_provider_file(&provider, "same.txt", b"preserved")
            .await
            .unwrap();

        rename_provider_path(&provider, "same.txt", "same.txt")
            .await
            .unwrap();

        let path = "same.txt".to_owned();
        let item = provider.item(&path).await.unwrap();
        assert_eq!(
            provider.read(&path, 0, item.size).await.unwrap(),
            b"preserved"
        );
    }
}

#[cfg(test)]
mod tests;
