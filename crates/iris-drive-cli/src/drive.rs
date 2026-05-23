#[allow(clippy::wildcard_imports)]
use super::*;

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
            short_pubkey(&d.owner_pubkey),
            d.display_name,
        );
    }
    Ok(())
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
            .account
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no account; run `idrive init` first"))?;
        let authorized = authorized_device_pubkeys(account);

        // Fetch each authorized device's tree + tombstones from htree.
        // With `--at N`, this device's own root walks back N revisions
        // via the `.hashtree/prev` chain; other devices' roots stay at their
        // current state.
        let mut snapshots_data = Vec::new();
        for device_pubkey in &authorized {
            let Some(root) = drive.device_roots.get(device_pubkey) else {
                continue; // device hasn't published its root yet
            };
            let mut cid = Cid::parse(&root.root_cid)
                .with_context(|| format!("parsing root CID for device {device_pubkey}"))?;
            if at > 0 && *device_pubkey == account.device_pubkey {
                cid = iris_drive_core::history::revision_at(daemon.tree(), &cid, at)
                    .await
                    .with_context(|| format!("revision -{at} not in chain"))?;
            }
            let (files, tombstones) = walk_device_tree(daemon.tree(), &cid).await?;
            snapshots_data.push((device_pubkey.clone(), root.clone(), files, tombstones));
        }

        let authorized_refs: Vec<&str> = authorized.iter().map(String::as_str).collect();
        let snapshots: Vec<DeviceSnapshot> = snapshots_data
            .iter()
            .map(|(pk, root, files, tombs)| DeviceSnapshot {
                device_pubkey: pk.as_str(),
                root,
                files: files.clone(),
                tombstones: tombs.clone(),
            })
            .collect();

        let view = merge_drives(&authorized_refs, &snapshots);

        println!(
            "{}",
            json!({
                "drive_id": drive.drive_id,
                "at_revision": at,
                "authorized_devices": authorized.len(),
                "device_roots_present": snapshots.len(),
                "files": view
                    .files
                    .iter()
                    .map(|e| json!({
                        "path": e.path,
                        "size": e.size,
                        "sha256": e.whole_file_hash.unwrap_or(e.hash).map(|byte| format!("{byte:02x}")).join(""),
                        "source_device": e.source_device,
                        "published_at": e.published_at,
                    }))
                    .collect::<Vec<_>>(),
                "suppressed_by_tombstone": view.suppressed_by_tombstone,
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}

pub(crate) async fn walk_device_tree(
    tree: &HashTree<hashtree_fs::FsBlobStore>,
    root: &Cid,
) -> Result<(Vec<DeviceFileEntry>, Vec<DeviceTombstone>)> {
    iris_drive_core::merge::walk_device_tree(tree, root)
        .await
        .map_err(anyhow::Error::from)
}

pub(crate) fn cmd_relays(config_dir: &std::path::Path, sub: Option<RelaysCmd>) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    match sub.unwrap_or(RelaysCmd::List) {
        RelaysCmd::List => {}
        RelaysCmd::Add { url } => {
            let url = normalize_relay_url(&url);
            if !config.relays.contains(&url) {
                config.relays.push(url);
                config.save(config_path_in(config_dir))?;
            }
        }
        RelaysCmd::Update { old_url, new_url } => {
            let old_url = normalize_relay_url(&old_url);
            let new_url = normalize_relay_url(&new_url);
            let mut changed = false;
            for relay in &mut config.relays {
                if relay == &old_url {
                    relay.clone_from(&new_url);
                    changed = true;
                }
            }
            dedupe_relays(&mut config.relays);
            if changed {
                config.save(config_path_in(config_dir))?;
            }
        }
        RelaysCmd::Remove { url } => {
            let url = normalize_relay_url(&url);
            let before = config.relays.len();
            config.relays.retain(|s| s != &url);
            if config.relays.len() != before {
                config.save(config_path_in(config_dir))?;
            }
        }
        RelaysCmd::Reset => {
            config.relays = iris_drive_core::config::DEFAULT_RELAYS
                .iter()
                .map(|s| (*s).to_string())
                .collect();
            config.save(config_path_in(config_dir))?;
        }
    }
    println!("{}", serde_json::to_string_pretty(&config.relays)?);
    Ok(())
}

pub(crate) fn normalize_relay_url(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
        trimmed.to_string()
    } else {
        format!("wss://{trimmed}")
    }
}

pub(crate) fn dedupe_relays(relays: &mut Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    relays.retain(|relay| seen.insert(relay.clone()));
}

pub(crate) fn cmd_history(config_dir: &std::path::Path, limit: usize) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let daemon = Daemon::open(config_dir)
            .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
        let config = daemon.config();
        let account = config
            .account
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("no account; run `idrive init` first"))?;
        let drive = config
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .ok_or_else(|| anyhow::anyhow!("primary drive missing"))?;
        let Some(root_ref) = drive.device_roots.get(&account.device_pubkey) else {
            println!("{}", json!({"revisions": [], "note": "no imports yet"}));
            return Ok::<_, anyhow::Error>(());
        };
        let root_cid = Cid::parse(&root_ref.root_cid)
            .with_context(|| format!("parsing root cid {}", root_ref.root_cid))?;
        let chain = iris_drive_core::history::walk_history(daemon.tree(), &root_cid, limit)
            .await
            .context("walking history chain")?;

        let mut revisions = Vec::new();
        for (idx, cid) in chain.iter().enumerate() {
            // Count user-visible top-level entries (skip the .hashtree meta dir).
            let entries = daemon
                .tree()
                .list_directory(cid)
                .await
                .with_context(|| format!("listing rev {idx}"))?;
            let user_visible = entries
                .iter()
                .filter(|e| e.name != iris_drive_core::merge::META_DIR)
                .count();
            revisions.push(json!({
                "rev": idx,
                "root_cid": cid.to_string(),
                "top_level_entries": user_visible,
            }));
        }
        println!(
            "{}",
            json!({
                "device_pubkey": account.device_pubkey,
                "limit": limit,
                "chain_length": revisions.len(),
                "revisions": revisions,
            })
        );
        Ok(())
    })
}

pub(crate) fn cmd_event_app_keys(config_dir: &std::path::Path) -> Result<()> {
    let state = load_account_state(config_dir)?;
    let snap = state
        .app_keys
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no AppKeys snapshot yet (run `idrive init` first)"))?;
    if !state.has_owner_signing_authority {
        return Err(anyhow::anyhow!(
            "this device does not have owner-signing authority — only owner-capable installs can publish AppKeys"
        ));
    }
    let account = Account::load(state.clone(), config_dir).context("loading account")?;
    let owner_keys = account
        .owner_key
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("owner key missing on disk"))?
        .keys();
    let event = iris_drive_core::nostr_events::build_app_keys_event(owner_keys, snap)
        .context("building AppKeys event")?;
    println!("{}", serde_json::to_string_pretty(&event)?);
    Ok(())
}

pub(crate) fn cmd_event_drive_root(config_dir: &std::path::Path) -> Result<()> {
    let state = load_account_state(config_dir)?;
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let drive = config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .ok_or_else(|| anyhow::anyhow!("primary drive missing"))?;
    let root_ref = drive
        .device_roots
        .get(&state.device_pubkey)
        .ok_or_else(|| {
            anyhow::anyhow!("no root for this device yet — run `idrive import <dir>` first")
        })?;
    let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
        .context("loading device key")?;
    let event = iris_drive_core::nostr_events::build_drive_root_event(
        device.keys(),
        &state.owner_pubkey,
        &drive.drive_id,
        root_ref,
        &authorized_device_pubkeys(&state),
    )
    .context("building drive-root event")?;
    println!("{}", serde_json::to_string_pretty(&event)?);
    Ok(())
}

pub(crate) fn cmd_index(dir: &std::path::Path) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())));
        let cid = index_dir(&tree, dir)
            .await
            .with_context(|| format!("indexing {}", dir.display()))?;
        let listing = tree
            .list_directory(&cid)
            .await
            .context("listing top-level entries")?;
        println!(
            "{}",
            json!({
                "dir": dir.display().to_string(),
                "root_cid": cid.to_string(),
                "top_level_entries": listing.len(),
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}
