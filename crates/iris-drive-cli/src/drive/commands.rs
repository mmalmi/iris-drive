#[allow(clippy::wildcard_imports)]
use super::*;
use iris_drive_core::relay_config::{dedupe_relay_urls, normalize_relay_url};

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
            let url = normalize_relay_url(&url)?;
            let before = config.relays.clone();
            dedupe_relay_urls(&mut config.relays)?;
            if !config.relays.iter().any(|relay| relay == &url) {
                config.relays.push(url);
            }
            if config.relays != before {
                config.save(config_path_in(config_dir))?;
            }
        }
        RelaysCmd::Update { old_url, new_url } => {
            let old_url = normalize_relay_url(&old_url)?;
            let new_url = normalize_relay_url(&new_url)?;
            let before = config.relays.clone();
            dedupe_relay_urls(&mut config.relays)?;
            let mut changed = false;
            for relay in &mut config.relays {
                if relay == &old_url {
                    relay.clone_from(&new_url);
                    changed = true;
                }
            }
            dedupe_relay_urls(&mut config.relays)?;
            if changed || config.relays != before {
                config.save(config_path_in(config_dir))?;
            }
        }
        RelaysCmd::Remove { url } => {
            let url = normalize_relay_url(&url)?;
            let original = config.relays.clone();
            dedupe_relay_urls(&mut config.relays)?;
            let before_len = config.relays.len();
            config.relays.retain(|s| s != &url);
            if config.relays.len() != before_len || config.relays != original {
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
            .profile
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
            anyhow::anyhow!("no root for this device yet - run `idrive import <dir>` first")
        })?;
    let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
        .context("loading device key")?;
    let event = iris_drive_core::nostr_events::build_drive_root_event(
        device.keys(),
        &state.root_scope_id(),
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
