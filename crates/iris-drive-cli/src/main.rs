use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hashtree_core::{Cid, HashTree, HashTreeConfig, MemoryStore};
use iris_drive_core::{
    account::Account,
    config::AppConfig,
    daemon::Daemon,
    index_dir,
    merge::{merge_drives, DeviceFileEntry, DeviceSnapshot, DeviceTombstone},
    paths::{config_path_in, default_config_dir, key_path_in},
    AccountState, Drive, DriveRole,
};
use nostr_sdk::nips::nip19::FromBech32;
use nostr_sdk::PublicKey;
use serde_json::json;

#[derive(Debug, Parser)]
#[command(name = "idrive", version, about = "Iris Drive CLI / daemon")]
struct Cli {
    /// Override the config dir (default: OS config dir / iris-drive).
    #[arg(long, env = "IRIS_DRIVE_CONFIG_DIR", global = true)]
    config_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// **Create** flow: generate a fresh owner key + a fresh device key
    /// on this machine. Single-device default; this install has owner
    /// signing authority and the `AppKeys` roster lists this one device.
    Init {
        /// Don't error if config already exists; print the existing state.
        #[arg(long)]
        force: bool,
        /// Human-readable device label (e.g. "Mac mini").
        #[arg(long)]
        label: Option<String>,
    },
    /// **Restore** flow: import an existing owner `nsec` onto this
    /// device. A fresh device key is generated; this install has owner
    /// signing authority.
    Restore {
        /// Owner secret key as nsec1... or 64-char hex.
        nsec: String,
        /// Human-readable device label.
        #[arg(long)]
        label: Option<String>,
    },
    /// **Link** flow: turn this install into a secondary device under an
    /// existing owner. Generates a fresh device key; does NOT receive
    /// the owner key. The device waits in `awaiting_approval` until the
    /// owner approves it from an owner-capable device.
    Link {
        /// Owner pubkey as npub1... or 64-char hex.
        owner: String,
        /// Human-readable device label.
        #[arg(long)]
        label: Option<String>,
    },
    /// Approve a pending device by adding it to the `AppKeys` roster.
    /// Only usable on devices with owner signing authority.
    Approve {
        /// Device pubkey to authorize (npub1... or 64-char hex).
        device: String,
        /// Optional device label to record alongside.
        #[arg(long)]
        label: Option<String>,
    },
    /// Print the current `AppKeys` roster as JSON.
    Roster,
    /// Rotate the drive content key (DCK) without changing the roster.
    /// Useful for periodic key freshness. Owner-only.
    RotateDck,
    /// Print daemon and sync status as JSON.
    Status,
    /// List configured drives.
    Drives,
    /// Show the local identity (owner + device pubkeys + auth state).
    Whoami,
    /// Index a local directory into an in-memory hashtree and print the
    /// root CID + summary. Useful for hands-on sanity checks against the
    /// indexer.
    Index {
        /// Directory to index.
        dir: PathBuf,
    },
    /// Index a local directory into the persistent on-disk store and
    /// stamp the resulting root CID onto the primary drive. Survives
    /// across daemon restarts (blocks live under <config-dir>/blocks/).
    Import {
        /// Working directory to import.
        dir: PathBuf,
    },
    /// List the merged view of the primary drive — files across every
    /// authorized device's tree with LWW resolution applied. On a
    /// single-device install this is just that device's tree.
    List,
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    let Some(config_dir) = cli.config_dir.clone().or_else(default_config_dir) else {
        eprintln!(
            "error: could not determine a config dir; set --config-dir or IRIS_DRIVE_CONFIG_DIR"
        );
        return ExitCode::from(2);
    };

    let result = match cli.command {
        Command::Init { force, label } => cmd_init(&config_dir, force, label),
        Command::Restore { nsec, label } => cmd_restore(&config_dir, &nsec, label),
        Command::Link { owner, label } => cmd_link(&config_dir, &owner, label),
        Command::Approve { device, label } => cmd_approve(&config_dir, &device, label),
        Command::Roster => cmd_roster(&config_dir),
        Command::RotateDck => cmd_rotate_dck(&config_dir),
        Command::Status => cmd_status(&config_dir),
        Command::Drives => cmd_drives(&config_dir),
        Command::Whoami => cmd_whoami(&config_dir),
        Command::Index { dir } => cmd_index(&dir),
        Command::Import { dir } => cmd_import(&config_dir, &dir),
        Command::List => cmd_list(&config_dir),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_init(config_dir: &std::path::Path, force: bool, label: Option<String>) -> Result<()> {
    if already_initialized(config_dir) && !force {
        eprintln!("iris-drive already initialized at {}", config_dir.display());
        eprintln!("use --force to print the existing state instead of erroring");
        return Err(anyhow::anyhow!("already initialized"));
    }
    let account = Account::create(config_dir, label).context("creating account")?;
    finish_account_init(config_dir, &account)
}

fn cmd_restore(config_dir: &std::path::Path, nsec: &str, label: Option<String>) -> Result<()> {
    if already_initialized(config_dir) {
        return Err(anyhow::anyhow!(
            "already initialized; remove {} first if you really want to overwrite",
            config_dir.display()
        ));
    }
    let account = Account::restore(config_dir, nsec, label).context("restoring account")?;
    finish_account_init(config_dir, &account)
}

fn cmd_link(config_dir: &std::path::Path, owner: &str, label: Option<String>) -> Result<()> {
    if already_initialized(config_dir) {
        return Err(anyhow::anyhow!(
            "already initialized; remove {} first if you really want to overwrite",
            config_dir.display()
        ));
    }
    let owner_hex = normalize_pubkey(owner).context("parsing owner pubkey")?;
    let account = Account::link(config_dir, owner_hex, label).context("linking device")?;
    finish_account_init(config_dir, &account)
}

fn finish_account_init(config_dir: &std::path::Path, account: &Account) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    config.account = Some(account.state.clone());
    if config.drive("main").is_none() {
        config.upsert_drive(Drive::primary(&account.state.owner_pubkey));
    }
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "config_dir": config_dir.display().to_string(),
            "owner_npub": account_npub(&account.state.owner_pubkey),
            "device_npub": account_npub(&account.state.device_pubkey),
            "has_owner_signing_authority": account.state.has_owner_signing_authority,
            "authorization_state": authorization_state_label(&account.state),
            "drives": config.drives.iter().map(|d| &d.drive_id).collect::<Vec<_>>(),
        })
    );
    Ok(())
}

fn cmd_approve(
    config_dir: &std::path::Path,
    device: &str,
    label: Option<String>,
) -> Result<()> {
    let device_hex = normalize_pubkey(device).context("parsing device pubkey")?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let mut account = Account::load(state, config_dir).context("loading account")?;
    let snap = account
        .approve_device(device_hex, label)
        .context("approving device")?;
    let device_count = snap.devices.len();
    config.account = Some(account.state.clone());
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "approved_device_npub": account_npub_or_self(device, &account.state),
            "roster_size": device_count,
        })
    );
    Ok(())
}

fn cmd_roster(config_dir: &std::path::Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let snap = state.app_keys.as_ref();
    println!(
        "{}",
        json!({
            "owner_npub": account_npub(&state.owner_pubkey),
            "current_device_npub": account_npub(&state.device_pubkey),
            "authorization_state": authorization_state_label(&state),
            "app_keys": snap.map(|s| json!({
                "created_at": s.created_at,
                "dck_generation": s.dck_generation,
                "devices": s.devices.iter().map(|d| json!({
                    "pubkey": d.pubkey,
                    "npub": account_npub(&d.pubkey),
                    "added_at": d.added_at,
                    "label": d.label,
                    "is_current_device": d.pubkey == state.device_pubkey,
                    "has_dck_wrap": s.wrapped_dck.contains_key(&d.pubkey),
                })).collect::<Vec<_>>(),
            })),
        })
    );
    Ok(())
}

fn cmd_rotate_dck(config_dir: &std::path::Path) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let mut account = Account::load(state, config_dir).context("loading account")?;
    let snap = account.rotate_dck().context("rotating DCK")?;
    let dck_gen = snap.dck_generation;
    config.account = Some(account.state.clone());
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "dck_generation": dck_gen,
            "device_wrap_count": account
                .state
                .app_keys
                .as_ref()
                .map_or(0, |s| s.wrapped_dck.len()),
        })
    );
    Ok(())
}

fn cmd_status(config_dir: &std::path::Path) -> Result<()> {
    let initialized = already_initialized(config_dir);
    let config = AppConfig::load_or_default(config_path_in(config_dir))
        .with_context(|| format!("reading config at {}", config_path_in(config_dir).display()))?;
    let account_block = config.account.as_ref().map(|state| {
        json!({
            "owner_npub": account_npub(&state.owner_pubkey),
            "device_npub": account_npub(&state.device_pubkey),
            "has_owner_signing_authority": state.has_owner_signing_authority,
            "authorization_state": authorization_state_label(state),
            "roster_size": state.app_keys.as_ref().map_or(0, |s| s.devices.len()),
        })
    });
    println!(
        "{}",
        json!({
            "initialized": initialized,
            "config_dir": config_dir.display().to_string(),
            "pubkey_npub": config.account.as_ref().map(|s| account_npub(&s.device_pubkey)),
            "account": account_block,
            "drives": config.drives.iter().map(|d| json!({
                "drive_id": d.drive_id,
                "display_name": d.display_name,
                "owner_pubkey": d.owner_pubkey,
                "role": drive_role_label(d.role),
                "last_root_cid": d.last_root_cid,
            })).collect::<Vec<_>>(),
        })
    );
    Ok(())
}

fn cmd_drives(config_dir: &std::path::Path) -> Result<()> {
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

fn cmd_whoami(config_dir: &std::path::Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    println!(
        "{}",
        json!({
            "owner_npub": account_npub(&state.owner_pubkey),
            "device_npub": account_npub(&state.device_pubkey),
            "has_owner_signing_authority": state.has_owner_signing_authority,
            "authorization_state": authorization_state_label(&state),
        })
    );
    Ok(())
}

fn cmd_import(config_dir: &std::path::Path, working_dir: &std::path::Path) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let mut daemon = Daemon::open(config_dir)
            .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
        let report = daemon
            .import_working_dir(working_dir)
            .await
            .with_context(|| format!("importing {}", working_dir.display()))?;
        println!(
            "{}",
            json!({
                "working_dir": report.working_dir.display().to_string(),
                "root_cid": report.root_cid,
                "top_level_entries": report.top_level_entries,
                "blocks_dir": daemon.blocks_dir().display().to_string(),
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}

fn cmd_list(config_dir: &std::path::Path) -> Result<()> {
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
        let authorized = account
            .app_keys
            .as_ref()
            .map(|s| {
                s.devices
                    .iter()
                    .map(|d| d.pubkey.clone())
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // Fetch each authorized device's tree + tombstones from htree.
        let mut snapshots_data = Vec::new();
        for device_pubkey in &authorized {
            let Some(root) = drive.device_roots.get(device_pubkey) else {
                continue; // device hasn't published its root yet
            };
            let cid = Cid::parse(&root.root_cid)
                .with_context(|| format!("parsing root CID for device {device_pubkey}"))?;
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
                "authorized_devices": authorized.len(),
                "device_roots_present": snapshots.len(),
                "files": view
                    .files
                    .iter()
                    .map(|e| json!({
                        "path": e.path,
                        "size": e.size,
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

async fn walk_device_tree(
    tree: &HashTree<hashtree_fs::FsBlobStore>,
    root: &Cid,
) -> Result<(Vec<DeviceFileEntry>, Vec<DeviceTombstone>)> {
    iris_drive_core::merge::walk_device_tree(tree, root)
        .await
        .map_err(anyhow::Error::from)
}

fn cmd_index(dir: &std::path::Path) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let tree = HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public());
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

fn already_initialized(config_dir: &std::path::Path) -> bool {
    // An install is "initialized" when both a device key and a non-empty
    // config (with account) exist. Owner key may or may not be present
    // depending on flow (link installs don't have one).
    key_path_in(config_dir).exists()
        && config_path_in(config_dir).exists()
        && AppConfig::load_or_default(config_path_in(config_dir))
            .ok()
            .and_then(|c| c.account)
            .is_some()
}

fn normalize_pubkey(input: &str) -> Result<String> {
    let trimmed = input.trim();
    if trimmed.starts_with("npub1") {
        let pk = PublicKey::from_bech32(trimmed).context("parsing npub")?;
        Ok(pk.to_hex())
    } else if trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(trimmed.to_string())
    } else {
        Err(anyhow::anyhow!(
            "expected npub1... or 64-char hex pubkey, got {trimmed}"
        ))
    }
}

fn account_npub(hex: &str) -> String {
    use nostr_sdk::nips::nip19::ToBech32;
    PublicKey::from_hex(hex)
        .ok()
        .and_then(|pk| pk.to_bech32().ok())
        .unwrap_or_else(|| hex.to_string())
}

fn account_npub_or_self(input: &str, state: &AccountState) -> String {
    normalize_pubkey(input).map_or_else(|_| account_npub(&state.device_pubkey), |h| account_npub(&h))
}

fn authorization_state_label(state: &AccountState) -> &'static str {
    use iris_drive_core::DeviceAuthorizationState as S;
    match state.authorization_state {
        S::Authorized => "authorized",
        S::AwaitingApproval => "awaiting_approval",
        S::Revoked => "revoked",
    }
}

fn drive_role_label(role: DriveRole) -> &'static str {
    match role {
        DriveRole::Owner => "owner",
        DriveRole::Editor => "editor",
        DriveRole::Reader => "reader",
    }
}

fn short_pubkey(pk: &str) -> String {
    if pk.len() > 14 {
        format!("{}…{}", &pk[..6], &pk[pk.len() - 6..])
    } else {
        pk.to_string()
    }
}

