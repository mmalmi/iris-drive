use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hashtree_core::{Cid, HashTree, HashTreeConfig, MemoryStore, NHashData, nhash_encode_full};
use iris_drive_core::{
    AccountState, Drive, DriveRole,
    account::Account,
    blossom_sync::{DownloadReport, UploadReport},
    config::AppConfig,
    daemon::Daemon,
    index_dir,
    merge::{DeviceFileEntry, DeviceSnapshot, DeviceTombstone, merge_drives},
    paths::{config_path_in, default_config_dir, key_path_in},
};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::FromBech32;
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
    List {
        /// Walk back N revisions on this device's tree before merging
        /// (0 = current = default, 1 = previous, ...). History comes
        /// from the `.hashtree/prev` chain stored in each directory's `TreeNode`.
        #[arg(long, default_value_t = 0)]
        at: usize,
    },
    /// Walk this device's `.hashtree/prev` revision chain and print each root
    /// CID + top-level entry count, newest-first. Blocks GC'd from
    /// the local store terminate the walk silently.
    History {
        /// Maximum number of revisions to walk back. Defaults to 50.
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Build + print Nostr events ready to broadcast to relays.
    #[command(subcommand)]
    Event(EventCmd),
    /// Print configured relay URLs.
    Relays,
    /// List or modify configured Blossom HTTP blob servers used for
    /// block replication.
    #[command(subcommand)]
    BlossomServers(BlossomServersCmd),
    /// Publish current state (`AppKeys` + this device's drive root) to
    /// all configured relays. Skips `AppKeys` on linked devices that
    /// lack owner-signing authority.
    Publish {
        /// Override config relays with these URLs.
        #[arg(long)]
        relay: Vec<String>,
        /// Per-relay connect timeout (seconds).
        #[arg(long, default_value_t = 10)]
        timeout: u64,
    },
    /// Pull latest `AppKeys` + drive-root events from relays and apply
    /// them locally. After this, `idrive list` reflects every
    /// authorized device's contribution.
    Sync {
        /// Override config relays with these URLs.
        #[arg(long)]
        relay: Vec<String>,
        /// Seconds to wait for relay responses.
        #[arg(long, default_value_t = 10)]
        timeout: u64,
    },
    /// Run a long-running subscriber + publisher. Maintains open
    /// subscriptions for `AppKeys` + drive-root events, applies each
    /// event in real time, and watches the working directory (set by
    /// the first `idrive import`) for changes — auto-publishing a new
    /// drive-root event whenever the root CID changes. fs-events
    /// trigger near-immediately (debounced); a periodic timer
    /// provides a fallback in case any events get missed. Stops on
    /// Ctrl+C.
    Daemon {
        /// Override config relays with these URLs.
        #[arg(long)]
        relay: Vec<String>,
        /// Fallback periodic re-scan in seconds, in addition to
        /// near-immediate fs-notify triggers. Set to 0 to disable the
        /// periodic fallback (still get fs-notify). Set with no
        /// `working_dir` to disable auto-publish entirely.
        #[arg(long, default_value_t = 60)]
        watch_interval: u64,
        /// Debounce window after the last fs-notify event before
        /// kicking off a re-import, in milliseconds. Lower = faster
        /// response; higher = fewer scans during bursts (e.g.,
        /// editors that save via rename-on-temp).
        #[arg(long, default_value_t = 500)]
        watch_debounce_ms: u64,
    },
}

#[derive(Debug, Subcommand)]
enum BlossomServersCmd {
    /// Print current Blossom servers as JSON.
    List,
    /// Append a server URL to the configured list.
    Add { url: String },
    /// Remove a server URL from the configured list.
    Remove { url: String },
}

#[derive(Debug, Subcommand)]
enum EventCmd {
    /// Owner-signed `AppKeys` roster event (kind 30078).
    /// Requires owner-signing authority on this install.
    AppKeys,
    /// Device-signed drive-root event (kind 30079) for the primary
    /// drive. Requires a previous `idrive import` so there's a CID
    /// to publish.
    DriveRoot,
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
        Command::List { at } => cmd_list(&config_dir, at),
        Command::History { limit } => cmd_history(&config_dir, limit),
        Command::Event(ev) => match ev {
            EventCmd::AppKeys => cmd_event_app_keys(&config_dir),
            EventCmd::DriveRoot => cmd_event_drive_root(&config_dir),
        },
        Command::Relays => cmd_relays(&config_dir),
        Command::BlossomServers(sub) => cmd_blossom_servers(&config_dir, sub),
        Command::Publish { relay, timeout } => cmd_publish(&config_dir, &relay, timeout),
        Command::Sync { relay, timeout } => cmd_sync(&config_dir, &relay, timeout),
        Command::Daemon {
            relay,
            watch_interval,
            watch_debounce_ms,
        } => cmd_daemon(&config_dir, &relay, watch_interval, watch_debounce_ms),
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

fn cmd_approve(config_dir: &std::path::Path, device: &str, label: Option<String>) -> Result<()> {
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
    let blocks_dir = config_dir.join("blocks");
    let block_stats = collect_file_stats(&blocks_dir)
        .with_context(|| format!("reading block store stats at {}", blocks_dir.display()))?;
    let current_root_cid = config
        .account
        .as_ref()
        .and_then(|state| {
            config
                .drive(iris_drive_core::PRIMARY_DRIVE_ID)
                .and_then(|drive| drive.device_roots.get(&state.device_pubkey))
                .map(|root| root.root_cid.clone())
        })
        .or_else(|| {
            config
                .drive(iris_drive_core::PRIMARY_DRIVE_ID)
                .and_then(|drive| drive.last_root_cid.clone())
        });
    let current_root_private = current_root_cid.as_deref().and_then(root_is_private);
    let files_iris_to_url = current_root_cid
        .as_deref()
        .and_then(files_iris_to_url_for_root);
    let top_level_entries = current_root_cid
        .as_deref()
        .and_then(|root| root_top_level_entries(config_dir, root));
    let peers = peer_statuses(&config);
    let authorized_device_count = peers.len();
    let published_device_roots = config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .map_or(0, |drive| drive.device_roots.len());
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
                "working_dir": d.working_dir.as_ref().map(|p| p.display().to_string()),
                "device_root_count": d.device_roots.len(),
            })).collect::<Vec<_>>(),
            "hashtree": {
                "blocks_dir": blocks_dir.display().to_string(),
                "local_block_count": block_stats.file_count,
                "local_block_bytes": block_stats.total_bytes,
                "current_root_cid": current_root_cid,
                "current_root_private": current_root_private,
                "files_iris_to_url": files_iris_to_url,
                "top_level_entries": top_level_entries,
            },
            "network": {
                "relays": config.relays,
                "blossom_servers": config.blossom_servers,
                "authorized_device_count": authorized_device_count,
                "published_device_roots": published_device_roots,
            },
            "peers": peers,
        })
    );
    Ok(())
}

#[derive(Debug, Default)]
struct FileStats {
    file_count: u64,
    total_bytes: u64,
}

fn collect_file_stats(path: &Path) -> Result<FileStats> {
    if !path.exists() {
        return Ok(FileStats::default());
    }

    let metadata = std::fs::metadata(path)?;
    if metadata.is_file() {
        return Ok(FileStats {
            file_count: 1,
            total_bytes: metadata.len(),
        });
    }
    if !metadata.is_dir() {
        return Ok(FileStats::default());
    }

    let mut stats = FileStats::default();
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = entry.metadata()?;
            if metadata.is_dir() {
                stack.push(path);
            } else if metadata.is_file() {
                stats.file_count += 1;
                stats.total_bytes += metadata.len();
            }
        }
    }
    Ok(stats)
}

fn root_is_private(root_cid: &str) -> Option<bool> {
    Cid::parse(root_cid).ok().map(|cid| cid.key.is_some())
}

fn files_iris_to_url_for_root(root_cid: &str) -> Option<String> {
    let cid = Cid::parse(root_cid).ok()?;
    let nhash = nhash_encode_full(&NHashData {
        hash: cid.hash,
        decrypt_key: cid.key,
    })
    .ok()?;
    Some(format!("https://files.iris.to/#/{nhash}"))
}

fn root_top_level_entries(config_dir: &Path, root_cid: &str) -> Option<usize> {
    let cid = Cid::parse(root_cid).ok()?;
    let daemon = Daemon::open(config_dir).ok()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    runtime.block_on(async {
        daemon
            .tree()
            .list_directory(&cid)
            .await
            .ok()
            .map(|entries| {
                entries
                    .iter()
                    .filter(|entry| entry.name != iris_drive_core::META_DIR)
                    .count()
            })
    })
}

fn peer_statuses(config: &AppConfig) -> Vec<serde_json::Value> {
    let Some(account) = config.account.as_ref() else {
        return Vec::new();
    };
    let Some(snapshot) = account.app_keys.as_ref() else {
        return Vec::new();
    };
    let primary_drive = config.drive(iris_drive_core::PRIMARY_DRIVE_ID);

    snapshot
        .devices
        .iter()
        .map(|device| {
            let root = primary_drive.and_then(|drive| drive.device_roots.get(&device.pubkey));
            let root_cid = root.map(|root| root.root_cid.clone());
            let root_private = root_cid.as_deref().and_then(root_is_private);
            json!({
                "device_pubkey": device.pubkey,
                "device_npub": account_npub(&device.pubkey),
                "label": device.label,
                "authorized": true,
                "is_current_device": device.pubkey == account.device_pubkey,
                "added_at": device.added_at,
                "has_root": root.is_some(),
                "root_cid": root_cid,
                "root_private": root_private,
                "published_at": root.map(|root| root.published_at),
                "dck_generation": root.map(|root| root.dck_generation),
            })
        })
        .collect()
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
                "files_iris_to_url": files_iris_to_url_for_root(&report.root_cid),
                "top_level_entries": report.top_level_entries,
                "blocks_dir": daemon.blocks_dir().display().to_string(),
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}

fn cmd_list(config_dir: &std::path::Path, at: usize) -> Result<()> {
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

fn cmd_relays(config_dir: &std::path::Path) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    println!("{}", serde_json::to_string_pretty(&config.relays)?);
    Ok(())
}

fn cmd_blossom_servers(config_dir: &std::path::Path, sub: BlossomServersCmd) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    match sub {
        BlossomServersCmd::List => {}
        BlossomServersCmd::Add { url } => {
            if !config.blossom_servers.contains(&url) {
                config.blossom_servers.push(url);
                config.save(config_path_in(config_dir))?;
            }
        }
        BlossomServersCmd::Remove { url } => {
            let before = config.blossom_servers.len();
            config.blossom_servers.retain(|s| s != &url);
            if config.blossom_servers.len() != before {
                config.save(config_path_in(config_dir))?;
            }
        }
    }
    println!("{}", serde_json::to_string_pretty(&config.blossom_servers)?);
    Ok(())
}

fn cmd_publish(
    config_dir: &std::path::Path,
    relay_override: &[String],
    timeout_secs: u64,
) -> Result<()> {
    use iris_drive_core::relay_sync;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let config = AppConfig::load_or_default(config_path_in(config_dir))?;
        let state = config
            .account
            .clone()
            .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
        let relays = pick_relays(&config, relay_override);
        let _ = timeout_secs; // connect timeout not used by add_relay; kept for future
        let client = relay_sync::connect(&relays)
            .await
            .context("connecting to relays")?;

        let report = publish_current_state(&client, config_dir, &config, &state).await?;

        let _ = client.disconnect().await;
        println!(
            "{}",
            json!({
                "relays": relays,
                "blossom_servers": config.blossom_servers,
                "published_app_keys": report.published_app_keys,
                "published_drive_root": report.published_drive_root,
                "blossom_upload": report.blossom_upload.map(|r| json!({
                    "total_hashes": r.total_hashes,
                    "uploaded": r.uploaded,
                    "already_present": r.already_present,
                })),
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}

#[derive(Debug, Default)]
struct PublishStateReport {
    published_app_keys: bool,
    published_drive_root: bool,
    root_cid: Option<String>,
    blossom_upload: Option<UploadReport>,
}

async fn publish_current_state(
    client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    config: &AppConfig,
    state: &AccountState,
) -> Result<PublishStateReport> {
    use iris_drive_core::relay_sync;

    let mut report = PublishStateReport::default();
    if state.has_owner_signing_authority
        && let Some(snap) = state.app_keys.as_ref()
    {
        let account = Account::load(state.clone(), config_dir).context("loading account")?;
        let owner_keys = account
            .owner_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("owner key missing on disk"))?
            .keys();
        relay_sync::publish_app_keys(client, owner_keys, snap)
            .await
            .context("publishing AppKeys")?;
        report.published_app_keys = true;
    }

    if let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)
        && let Some(root) = drive.device_roots.get(&state.device_pubkey)
    {
        let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
        relay_sync::publish_drive_root(
            client,
            device.keys(),
            &state.owner_pubkey,
            &drive.drive_id,
            root,
            &authorized_device_pubkeys(state),
        )
        .await
        .context("publishing drive root")?;
        report.published_drive_root = true;
        report.root_cid = Some(root.root_cid.clone());

        if !config.blossom_servers.is_empty() {
            let bclient = iris_drive_core::blossom_sync_client(
                device.keys().clone(),
                &config.blossom_servers,
            );
            let daemon = Daemon::open(config_dir).context("opening daemon")?;
            let cid = Cid::parse(&root.root_cid)
                .with_context(|| format!("parsing root cid {}", root.root_cid))?;
            report.blossom_upload = Some(
                iris_drive_core::blossom_sync::upload_tree(daemon.tree(), &cid, &bclient)
                    .await
                    .context("uploading tree to blossom")?,
            );
        }
    }

    Ok(report)
}

#[allow(clippy::too_many_lines)]
fn cmd_sync(
    config_dir: &std::path::Path,
    relay_override: &[String],
    timeout_secs: u64,
) -> Result<()> {
    use iris_drive_core::relay_sync;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
        let state = config
            .account
            .clone()
            .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
        let relays = pick_relays(&config, relay_override);
        let client = relay_sync::connect(&relays)
            .await
            .context("connecting to relays")?;
        let timeout = std::time::Duration::from_secs(timeout_secs);

        // 1) Pull latest AppKeys and apply.
        let mut app_keys_applied = "none";
        if let Some(ev) = relay_sync::fetch_latest_app_keys(&client, &state.owner_pubkey, timeout)
            .await
            .context("fetching AppKeys")?
        {
            let outcome = relay_sync::apply_remote_app_keys_event(&mut config, &ev)
                .context("applying AppKeys event")?;
            app_keys_applied = match outcome {
                relay_sync::AppKeysApply::NotOurOwner => "not_our_owner",
                relay_sync::AppKeysApply::Applied(d) => match d {
                    iris_drive_core::ApplyDecision::Adopted => "adopted",
                    iris_drive_core::ApplyDecision::Replaced => "replaced",
                    iris_drive_core::ApplyDecision::Merged => "merged",
                    iris_drive_core::ApplyDecision::Rejected => "rejected",
                },
            };
        }

        // 2) Pull drive roots for every authorized device.
        let authorized_devices: Vec<String> = config
            .account
            .as_ref()
            .and_then(|s| s.app_keys.as_ref())
            .map(|s| s.devices.iter().map(|d| d.pubkey.clone()).collect())
            .unwrap_or_default();
        let drive_root_events = relay_sync::fetch_drive_roots(
            &client,
            &state.owner_pubkey,
            iris_drive_core::PRIMARY_DRIVE_ID,
            &authorized_devices,
            timeout,
        )
        .await
        .context("fetching drive roots")?;
        let mut drive_roots_applied = 0usize;
        let mut drive_roots_skipped = 0usize;
        let mut applied_root_cids: Vec<String> = Vec::new();
        let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
        for ev in &drive_root_events {
            let parsed =
                iris_drive_core::nostr_events::parse_drive_root_event_for_device(ev, device.keys())
                    .ok();
            match relay_sync::apply_remote_drive_root_event(&mut config, ev, Some(device.keys()))
                .context("applying drive-root event")?
            {
                relay_sync::DriveRootApply::Applied => {
                    drive_roots_applied += 1;
                    if let Some((_, _, _, root_ref)) = parsed {
                        applied_root_cids.push(root_ref.root_cid);
                    }
                }
                _ => drive_roots_skipped += 1,
            }
        }

        config.save(config_path_in(config_dir))?;

        // 3) Replicate blocks for each newly-applied drive root via
        // Blossom if servers are configured.
        let mut blossom_download_report: Option<DownloadReport> = None;
        if !applied_root_cids.is_empty() && !config.blossom_servers.is_empty() {
            let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
                .context("loading device key")?;
            let daemon = Daemon::open(config_dir).context("opening daemon")?;
            let local = daemon.tree().get_store().clone();
            let bclient = iris_drive_core::blossom_sync_client(
                device.keys().clone(),
                &config.blossom_servers,
            );
            let mut totals = DownloadReport::default();
            for cid_str in &applied_root_cids {
                let cid =
                    Cid::parse(cid_str).with_context(|| format!("parsing root cid {cid_str}"))?;
                let r = iris_drive_core::blossom_sync::download_tree(
                    local.clone(),
                    &cid,
                    bclient.clone(),
                )
                .await
                .with_context(|| format!("downloading tree for {cid_str}"))?;
                totals.total_hashes += r.total_hashes;
                totals.fetched += r.fetched;
                totals.already_local += r.already_local;
            }
            blossom_download_report = Some(totals);
        }

        let _ = client.disconnect().await;

        println!(
            "{}",
            json!({
                "relays": relays,
                "blossom_servers": config.blossom_servers,
                "app_keys_event_applied": app_keys_applied,
                "drive_root_events_seen": drive_root_events.len(),
                "drive_root_events_applied": drive_roots_applied,
                "drive_root_events_skipped": drive_roots_skipped,
                "blossom_download": blossom_download_report.map(|r| json!({
                    "total_hashes": r.total_hashes,
                    "fetched": r.fetched,
                    "already_local": r.already_local,
                })),
            })
        );
        Ok::<_, anyhow::Error>(())
    })
}

#[allow(clippy::too_many_lines)]
fn cmd_daemon(
    config_dir: &std::path::Path,
    relay_override: &[String],
    watch_interval: u64,
    watch_debounce_ms: u64,
) -> Result<()> {
    use iris_drive_core::relay_sync;
    use nostr_sdk::RelayPoolNotification;
    use tokio::sync::broadcast::error::RecvError;
    use tokio::sync::mpsc;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        // Zero-config bootstrap: if the tray app (or a future installer)
        // staged an account + working_dir but never ran an initial
        // import, do that now so the first launch produces a root
        // before we open the relay subscription.
        if key_path_in(config_dir).exists() {
            let mut daemon = Daemon::open(config_dir).context("opening daemon for bootstrap")?;
            if let Some(report) = daemon
                .ensure_initial_import()
                .await
                .context("initial import")?
            {
                println!(
                    "{}",
                    json!({
                        "event": "initial_import",
                        "root_cid": report.root_cid,
                        "files_iris_to_url": files_iris_to_url_for_root(&report.root_cid),
                        "working_dir": report.working_dir.display().to_string(),
                        "entries": report.top_level_entries,
                    })
                );
            }
        }

        let config = AppConfig::load_or_default(config_path_in(config_dir))?;
        let state = config
            .account
            .clone()
            .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
        let relays = pick_relays(&config, relay_override);
        let filters = relay_sync::subscription_filters(
            &state.owner_pubkey,
            iris_drive_core::PRIMARY_DRIVE_ID,
        );
        if filters.is_empty() {
            return Err(anyhow::anyhow!("no filters to subscribe to"));
        }
        let working_dir = config
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .and_then(|d| d.working_dir.clone());

        let client = relay_sync::connect(&relays)
            .await
            .context("connecting to relays")?;
        client
            .subscribe(filters, None)
            .await
            .context("opening subscription")?;
        let mut notifications = client.notifications();

        // Spawn an fs-notify watcher on the working dir. Events get
        // debounced (notify-debouncer-mini) then forwarded over an
        // mpsc; the main select! loop wakes up and calls
        // scan_and_publish near-immediately, instead of waiting on
        // the periodic timer.
        let (fs_tx, mut fs_rx) = mpsc::channel::<()>(8);
        let _watcher_guard = if let Some(dir) = working_dir.as_ref() {
            Some(
                spawn_fs_watcher(dir, watch_debounce_ms, fs_tx)
                    .context("spawning fs watcher")?,
            )
        } else {
            None
        };

        println!(
            "{}",
            json!({
                "event": "subscribed",
                "relays": relays,
                "owner_npub": account_npub(&state.owner_pubkey),
                "watch_interval_secs": watch_interval,
                "watch_debounce_ms": watch_debounce_ms,
                "working_dir": working_dir.as_ref().map(|p| p.display().to_string()),
            })
        );

        // Announce the current account roster and device root once on
        // startup, and upload the initial blocks if this launch just
        // imported them. The fs-notify + periodic paths only publish on
        // change, so without this a freshly-imported device would sit
        // silent until its first edit.
        match publish_current_state(&client, config_dir, &config, &state).await {
            Ok(report) => {
                let files_iris_to_url = report
                    .root_cid
                    .as_deref()
                    .and_then(files_iris_to_url_for_root);
                println!(
                    "{}",
                    json!({
                        "event": "initial_publish",
                        "published_app_keys": report.published_app_keys,
                        "published_drive_root": report.published_drive_root,
                        "root_cid": report.root_cid,
                        "files_iris_to_url": files_iris_to_url,
                        "blossom_upload": report.blossom_upload.map(|r| json!({
                            "total_hashes": r.total_hashes,
                            "uploaded": r.uploaded,
                            "already_present": r.already_present,
                        })),
                    })
                );
            }
            Err(e) => {
                println!(
                    "{}",
                    json!({
                        "event": "initial_publish_error",
                        "error": e.to_string(),
                    })
                );
            }
        }

        println!("(running — Ctrl+C to stop)");

        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);

        // Periodic fallback in addition to fs-notify (some editor
        // patterns produce events fs-notify can miss; this catches
        // drift).
        let mut watch_timer = if watch_interval > 0 && working_dir.is_some() {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(watch_interval));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            Some(interval)
        } else {
            None
        };

        loop {
            tokio::select! {
                _ = &mut ctrl_c => {
                    println!("{}", json!({ "event": "shutdown" }));
                    break;
                }
                recv = notifications.recv() => {
                    match recv {
                        Ok(RelayPoolNotification::Event { event, .. }) => {
                            if let Err(e) = apply_one_event(config_dir, &event).await {
                                println!(
                                    "{}",
                                    json!({"event": "apply_error", "id": event.id.to_hex(), "error": e.to_string()})
                                );
                            }
                        }
                        Ok(RelayPoolNotification::Shutdown)
                        | Err(RecvError::Closed) => break,
                        Ok(_) => {}
                        Err(RecvError::Lagged(n)) => {
                            println!("{}", json!({"event": "lagged", "skipped": n}));
                        }
                    }
                }
                Some(()) = fs_rx.recv() => {
                    if let Err(e) = scan_and_publish(&client, config_dir).await {
                        println!(
                            "{}",
                            json!({"event": "auto_publish_error", "trigger": "fs", "error": e.to_string()})
                        );
                    }
                }
                () = async {
                    if let Some(timer) = watch_timer.as_mut() {
                        timer.tick().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                } => {
                    if let Err(e) = scan_and_publish(&client, config_dir).await {
                        println!(
                            "{}",
                            json!({"event": "auto_publish_error", "trigger": "timer", "error": e.to_string()})
                        );
                    }
                }
            }
        }
        let _ = client.disconnect().await;
        Ok::<_, anyhow::Error>(())
    })
}

/// Spawn an fs-notify watcher on `dir`. The returned debouncer must be
/// kept alive for the watcher to keep firing; drop it to stop.
fn spawn_fs_watcher(
    dir: &std::path::Path,
    debounce_ms: u64,
    tx: tokio::sync::mpsc::Sender<()>,
) -> Result<notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>> {
    use notify::RecursiveMode;
    use notify_debouncer_mini::new_debouncer;
    use std::time::Duration;

    let mut debouncer = new_debouncer(
        Duration::from_millis(debounce_ms),
        move |result: notify_debouncer_mini::DebounceEventResult| {
            if let Ok(events) = result
                && !events.is_empty()
            {
                // Coalesce a batch into a single nudge; the main loop
                // re-reads disk state on each receive anyway.
                let _ = tx.try_send(());
            }
        },
    )
    .context("creating notify debouncer")?;
    debouncer
        .watcher()
        .watch(dir, RecursiveMode::Recursive)
        .context("starting fs watch")?;
    Ok(debouncer)
}

/// Re-import the configured working dir; if the new root CID differs
/// from what's already recorded for this device, publish a new
/// drive-root event. No-op when the root hasn't changed.
async fn scan_and_publish(client: &nostr_sdk::Client, config_dir: &std::path::Path) -> Result<()> {
    use iris_drive_core::relay_sync;
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no account"))?;
    let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID) else {
        return Ok(());
    };
    let Some(working_dir) = drive.working_dir.clone() else {
        return Ok(());
    };
    let previous_root_cid = drive
        .device_roots
        .get(&state.device_pubkey)
        .map(|r| r.root_cid.clone());

    let mut daemon = Daemon::open(config_dir).context("opening daemon")?;
    let report = daemon
        .import_working_dir(&working_dir)
        .await
        .context("re-importing working dir")?;
    if previous_root_cid.as_deref() == Some(report.root_cid.as_str()) {
        // No change — silently skip publish.
        return Ok(());
    }
    let new_root = daemon
        .config()
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .and_then(|d| d.device_roots.get(&state.device_pubkey))
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("device root missing after import"))?;

    let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
        .context("loading device key")?;
    relay_sync::publish_drive_root(
        client,
        device.keys(),
        &state.owner_pubkey,
        iris_drive_core::PRIMARY_DRIVE_ID,
        &new_root,
        &authorized_device_pubkeys(state),
    )
    .await
    .context("publishing drive root")?;

    // Upload blocks to Blossom (best-effort; logged on failure).
    let mut upload_report: Option<UploadReport> = None;
    if !config.blossom_servers.is_empty() {
        let cid = Cid::parse(&new_root.root_cid)
            .with_context(|| format!("parsing root cid {}", new_root.root_cid))?;
        let bclient =
            iris_drive_core::blossom_sync_client(device.keys().clone(), &config.blossom_servers);
        match iris_drive_core::blossom_sync::upload_tree(daemon.tree(), &cid, &bclient).await {
            Ok(r) => upload_report = Some(r),
            Err(e) => println!(
                "{}",
                json!({"event": "blossom_upload_error", "error": e.to_string()})
            ),
        }
    }
    println!(
        "{}",
        json!({
            "event": "auto_published",
            "root_cid": report.root_cid,
            "files_iris_to_url": files_iris_to_url_for_root(&report.root_cid),
            "top_level_entries": report.top_level_entries,
            "blossom_upload": upload_report.map(|r| json!({
                "total_hashes": r.total_hashes,
                "uploaded": r.uploaded,
                "already_present": r.already_present,
            })),
        })
    );
    Ok(())
}

async fn apply_one_event(config_dir: &std::path::Path, event: &nostr_sdk::Event) -> Result<()> {
    use iris_drive_core::relay_sync;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let kind = event.kind.as_u16();
    if kind == iris_drive_core::nostr_events::KIND_APP_KEYS {
        let outcome = relay_sync::apply_remote_app_keys_event(&mut config, event)?;
        println!(
            "{}",
            json!({
                "event": "app_keys",
                "event_id": event.id.to_hex(),
                "outcome": format!("{outcome:?}"),
            })
        );
    } else if kind == iris_drive_core::nostr_events::KIND_DRIVE_ROOT {
        let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
        let parsed =
            iris_drive_core::nostr_events::parse_drive_root_event_for_device(event, device.keys())
                .ok();
        let outcome =
            relay_sync::apply_remote_drive_root_event(&mut config, event, Some(device.keys()))?;
        let was_applied = matches!(outcome, relay_sync::DriveRootApply::Applied);
        println!(
            "{}",
            json!({
                "event": "drive_root",
                "event_id": event.id.to_hex(),
                "author": account_npub(&event.pubkey.to_hex()),
                "outcome": format!("{outcome:?}"),
            })
        );
        config.save(config_path_in(config_dir))?;

        // If we applied a fresh drive root and Blossom is configured,
        // pull the underlying blocks so `idrive list` can walk the
        // remote device's tree. Best-effort; errors are logged.
        if was_applied
            && !config.blossom_servers.is_empty()
            && let Some((_, _, _, root_ref)) = parsed
            && let Err(e) = pull_blocks_for_root(config_dir, &config, &root_ref.root_cid).await
        {
            println!(
                "{}",
                json!({"event": "blossom_download_error", "error": e.to_string()})
            );
        }
        return Ok(());
    } else {
        // Unknown kind; ignore.
        return Ok(());
    }
    config.save(config_path_in(config_dir))?;
    Ok(())
}

async fn pull_blocks_for_root(
    config_dir: &std::path::Path,
    config: &AppConfig,
    root_cid_str: &str,
) -> Result<()> {
    let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
        .context("loading device key")?;
    let daemon = Daemon::open(config_dir).context("opening daemon")?;
    let local = daemon.tree().get_store().clone();
    let bclient =
        iris_drive_core::blossom_sync_client(device.keys().clone(), &config.blossom_servers);
    let cid =
        Cid::parse(root_cid_str).with_context(|| format!("parsing root cid {root_cid_str}"))?;
    let report = iris_drive_core::blossom_sync::download_tree(local, &cid, bclient)
        .await
        .with_context(|| format!("downloading tree {root_cid_str}"))?;
    println!(
        "{}",
        json!({
            "event": "blossom_downloaded",
            "root_cid": root_cid_str,
            "fetched": report.fetched,
            "already_local": report.already_local,
            "total_hashes": report.total_hashes,
        })
    );
    Ok(())
}

fn pick_relays(config: &AppConfig, override_list: &[String]) -> Vec<String> {
    if override_list.is_empty() {
        config.relays.clone()
    } else {
        override_list.to_vec()
    }
}

fn authorized_device_pubkeys(state: &AccountState) -> Vec<String> {
    let mut devices: Vec<String> = state
        .app_keys
        .as_ref()
        .map(|snap| snap.devices.iter().map(|d| d.pubkey.clone()).collect())
        .unwrap_or_default();
    if !devices.contains(&state.device_pubkey) {
        devices.push(state.device_pubkey.clone());
    }
    devices
}

fn cmd_history(config_dir: &std::path::Path, limit: usize) -> Result<()> {
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

fn cmd_event_app_keys(config_dir: &std::path::Path) -> Result<()> {
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

fn cmd_event_drive_root(config_dir: &std::path::Path) -> Result<()> {
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

fn load_account_state(config_dir: &std::path::Path) -> Result<AccountState> {
    AppConfig::load_or_default(config_path_in(config_dir))?
        .account
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))
}

fn cmd_index(dir: &std::path::Path) -> Result<()> {
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
    normalize_pubkey(input)
        .map_or_else(|_| account_npub(&state.device_pubkey), |h| account_npub(&h))
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
