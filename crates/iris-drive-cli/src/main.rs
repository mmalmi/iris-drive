use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hashtree_core::{Cid, HashTree, HashTreeConfig, MemoryStore, NHashData, nhash_encode_full};
use iris_drive_core::{
    AccountState, BackupTarget, BackupTargetKind, BackupTargetSync, DeviceRootRef, Drive,
    DriveRole, FsFipsBlockSync, PRIMARY_DRIVE_ID,
    account::Account,
    blossom_sync::{DownloadReport, UploadReport},
    config::AppConfig,
    daemon::{Daemon, EmbeddedHashtreeHost, MaterializeWorkingDirReport},
    gateway::{GatewayBind, GatewayServer},
    index_dir,
    merge::{DeviceFileEntry, DeviceSnapshot, DeviceTombstone, merge_drives},
    paths::{config_path_in, default_config_dir, key_path_in},
};
use nostr_sdk::nips::nip19::FromBech32;
use nostr_sdk::{Event, PublicKey, RelayStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const DEFAULT_GATEWAY_PORT: u16 = 17_321;
const CONFLICT_STATUS_PATH_CAP: usize = 32;
const FIPS_DOWNLOAD_RETRY_DELAYS: &[u64] = &[1, 2, 5, 10, 20];
const FIPS_DOWNLOAD_BEFORE_BLOSSOM_RETRY_DELAYS: &[u64] = &[];
const FIPS_DOWNLOAD_ATTEMPT_TIMEOUT_SECS: u64 = 30;
const FIPS_DOWNLOAD_BEFORE_BLOSSOM_ATTEMPT_TIMEOUT_SECS: u64 = 30;
const STARTUP_NETWORK_TIMEOUT_SECS: u64 = 20;
const EVENT_BLOCK_PULL_TIMEOUT_SECS: u64 = 10;
const EVENT_MATERIALIZE_TIMEOUT_SECS: u64 = 30;
const RELAY_PUBLISH_TIMEOUT_SECS: u64 = 10;
const STATUS_PROBE_TIMEOUT_SECS: u64 = 2;
const BLOSSOM_DOWNLOAD_RETRY_DELAYS: &[u64] = &[2, 5, 10, 20, 30, 45, 60];
const BLOSSOM_UPLOAD_TIMEOUT_SECS: u64 = 10;
const DIRECT_ROOT_MESH_STREAM_PREFIX: &str = "iris-drive/root-events/v1";
const DIRECT_ROOT_EVENT_CACHE_CAP: usize = 128;

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
    /// Revoke an authorized device and rotate the drive content key.
    Revoke {
        /// Device pubkey to revoke (npub1... or 64-char hex).
        device: String,
    },
    /// Print the current `AppKeys` roster as JSON.
    Roster,
    /// Rotate the drive content key (DCK) without changing the roster.
    /// Useful for periodic key freshness. Owner-only.
    RotateDck,
    /// Print daemon and sync status as JSON.
    Status,
    /// Inspect or resolve durable conflict ledger records.
    #[command(subcommand)]
    Conflicts(ConflictsCmd),
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
    /// List or modify configured Nostr relays.
    Relays {
        #[command(subcommand)]
        command: Option<RelaysCmd>,
    },
    /// List or modify configured Blossom HTTP blob servers used for
    /// block replication.
    #[command(subcommand)]
    BlossomServers(BlossomServersCmd),
    /// List, add, remove, or sync encrypted backup targets.
    #[command(subcommand)]
    Backups(BackupsCmd),
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
        /// Start the loopback browser gateway on this port.
        #[arg(long, default_value_t = DEFAULT_GATEWAY_PORT)]
        gateway_port: u16,
        /// Disable the loopback browser gateway.
        #[arg(long)]
        no_gateway: bool,
    },
}

#[derive(Debug, Subcommand)]
enum ConflictsCmd {
    /// Mark a conflict record resolved after the files have been handled.
    Resolve {
        /// Conflict id from `idrive status`.
        conflict_id: String,
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
enum BackupsCmd {
    /// Print configured encrypted backup targets as JSON.
    List,
    /// Add or update a Blossom URL or FIPS npub backup target.
    Add {
        target: String,
        #[arg(long)]
        label: Option<String>,
    },
    /// Remove a backup target.
    Remove { target: String },
    /// Push the current encrypted root to usable backup targets.
    Sync {
        /// Restrict sync to one configured target.
        #[arg(long)]
        target: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
enum RelaysCmd {
    /// Print current relay URLs as JSON.
    List,
    /// Append a relay URL to the configured list.
    Add { url: String },
    /// Replace an existing relay URL.
    Update { old_url: String, new_url: String },
    /// Remove a relay URL from the configured list.
    Remove { url: String },
    /// Restore the default relay list.
    Reset,
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

#[cfg(windows)]
fn main() -> ExitCode {
    std::thread::Builder::new()
        .name("idrive-main".to_string())
        .stack_size(8 * 1024 * 1024)
        .spawn(run_cli)
        .and_then(|handle| {
            handle
                .join()
                .map_err(|_| std::io::Error::other("idrive main thread panicked"))
        })
        .unwrap_or(ExitCode::FAILURE)
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    run_cli()
}

fn run_cli() -> ExitCode {
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
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
        Command::Revoke { device } => cmd_revoke(&config_dir, &device),
        Command::Roster => cmd_roster(&config_dir),
        Command::RotateDck => cmd_rotate_dck(&config_dir),
        Command::Status => cmd_status(&config_dir),
        Command::Conflicts(command) => cmd_conflicts(&config_dir, command),
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
        Command::Relays { command } => cmd_relays(&config_dir, command),
        Command::BlossomServers(sub) => cmd_blossom_servers(&config_dir, sub),
        Command::Backups(sub) => cmd_backups(&config_dir, sub),
        Command::Publish { relay, timeout } => cmd_publish(&config_dir, &relay, timeout),
        Command::Sync { relay, timeout } => cmd_sync(&config_dir, &relay, timeout),
        Command::Daemon {
            relay,
            watch_interval,
            watch_debounce_ms,
            gateway_port,
            no_gateway,
        } => cmd_daemon(
            &config_dir,
            &relay,
            watch_interval,
            watch_debounce_ms,
            gateway_port,
            !no_gateway,
        ),
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
    if config.drive(PRIMARY_DRIVE_ID).is_none() {
        let mut drive = Drive::primary(&account.state.owner_pubkey);
        drive.working_dir = Some(iris_drive_core::paths::default_working_dir_in(config_dir));
        config.upsert_drive(drive);
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
            "device_link_request": device_link_request_json(&account.state),
            "drives": config.drives.iter().map(|d| &d.drive_id).collect::<Vec<_>>(),
        })
    );
    Ok(())
}

fn cmd_approve(config_dir: &std::path::Path, device: &str, label: Option<String>) -> Result<()> {
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let (device_hex, label) = resolve_device_approval_input(device, &state.owner_pubkey, label)
        .context("parsing device approval request")?;
    let approved_device_npub = account_npub(&device_hex);
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
            "approved_device_npub": approved_device_npub,
            "roster_size": device_count,
        })
    );
    Ok(())
}

fn cmd_revoke(config_dir: &std::path::Path, device: &str) -> Result<()> {
    let device_hex = normalize_pubkey(device).context("parsing device pubkey")?;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    if state.device_pubkey == device_hex {
        return Err(anyhow::anyhow!("cannot revoke this device from itself"));
    }
    let mut account = Account::load(state, config_dir).context("loading account")?;
    let snap = account
        .revoke_device(&device_hex)
        .context("revoking device")?;
    let device_count = snap.devices.len();
    let dck_generation = snap.dck_generation;
    config.account = Some(account.state.clone());
    config.save(config_path_in(config_dir))?;
    println!(
        "{}",
        json!({
            "revoked_device_npub": account_npub(&device_hex),
            "roster_size": device_count,
            "dck_generation": dck_generation,
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
    let current_root_cid = current_primary_root_cid(&config);
    let current_root_private = current_root_cid.as_deref().and_then(root_is_private);
    let drive_iris_to_url = current_root_cid
        .as_ref()
        .and_then(|_| drive_iris_to_url_for_primary_drive(&config));
    let snapshot_url = current_root_cid
        .as_deref()
        .and_then(drive_iris_to_snapshot_url_for_root);
    let browser_gateway_urls =
        local_gateway_urls_for_root(current_root_cid.as_deref(), DEFAULT_GATEWAY_PORT);
    let merged_counts = primary_drive_counts(config_dir, &config);
    let top_level_entries = merged_counts.map(|(_, top_level)| top_level).or_else(|| {
        current_root_cid
            .as_deref()
            .and_then(|root| root_top_level_entries(config_dir, root))
    });
    let file_count = merged_counts.map(|(files, _)| files).or_else(|| {
        current_root_cid
            .as_deref()
            .and_then(|root| root_file_count(config_dir, root))
    });
    let conflict_status = current_root_cid
        .as_deref()
        .and_then(|root| root_conflict_status(config_dir, root))
        .unwrap_or_else(|| conflict_status_payload(&[]));
    let daemon_status = load_daemon_status(config_dir);
    let peers = peer_statuses(config_dir, &config, daemon_status.as_ref());
    let default_working_dir = iris_drive_core::paths::default_working_dir_in(config_dir);
    let authorized_device_count = peers.len();
    let published_device_roots = config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .map_or(0, |drive| drive.device_roots.len());
    let fips_diagnostics = fips_network_diagnostics(&config, daemon_status.as_ref());
    let backup_target_count = config.backup_targets.len();
    let backup_targets = backup_targets_status(&config);
    let account_block = status_account_block(&config);
    println!(
        "{}",
        json!({
            "initialized": initialized,
            "config_dir": config_dir.display().to_string(),
            "default_working_dir": default_working_dir.display().to_string(),
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
                "drive_iris_to_url": drive_iris_to_url,
                "files_iris_to_url": drive_iris_to_url,
                "snapshot_url": snapshot_url,
                "permalink_url": snapshot_url,
                "local_gateway": browser_gateway_urls,
                "file_count": file_count,
                "top_level_entries": top_level_entries,
            },
            "network": {
                "relays": config.relays,
                "blossom_servers": config.blossom_servers,
                "backup_target_count": backup_target_count,
                "backup_targets": backup_targets,
                "authorized_device_count": authorized_device_count,
                "published_device_roots": published_device_roots,
                "relay_statuses": daemon_status
                    .as_ref()
                    .and_then(|status| status.get("relay_statuses"))
                    .cloned()
                    .unwrap_or_else(|| json!([])),
                "fips": fips_diagnostics,
            },
            "daemon": daemon_status,
            "conflicts": conflict_status,
            "peers": peers,
        })
    );
    Ok(())
}

fn status_account_block(config: &AppConfig) -> Option<Value> {
    config.account.as_ref().map(|state| {
        json!({
            "owner_npub": account_npub(&state.owner_pubkey),
            "device_npub": account_npub(&state.device_pubkey),
            "has_owner_signing_authority": state.has_owner_signing_authority,
            "authorization_state": authorization_state_label(state),
            "roster_size": state.app_keys.as_ref().map_or(0, |s| s.devices.len()),
            "device_link_request": device_link_request_json(state),
        })
    })
}

fn current_primary_root_cid(config: &AppConfig) -> Option<String> {
    config
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
        })
}

fn backup_targets_status(config: &AppConfig) -> Vec<Value> {
    config
        .backup_targets
        .iter()
        .map(backup_target_status)
        .collect()
}

fn backup_target_status(target: &BackupTarget) -> Value {
    json!({
        "id": target.id.as_str(),
        "kind": backup_target_kind_label(target.kind),
        "target": target.target.as_str(),
        "label": target.label.as_deref(),
        "enabled": target.enabled,
        "last_sync": target.last_sync.as_ref().map(backup_target_sync_status),
    })
}

fn backup_target_sync_status(sync: &BackupTargetSync) -> Value {
    json!({
        "state": sync.state.as_str(),
        "root_cid": sync.root_cid.as_str(),
        "synced_at": sync.synced_at,
        "total_hashes": sync.total_hashes,
        "uploaded": sync.uploaded,
        "already_present": sync.already_present,
    })
}

fn backup_target_kind_label(kind: BackupTargetKind) -> &'static str {
    match kind {
        BackupTargetKind::Blossom => "blossom",
        BackupTargetKind::Fips => "fips",
    }
}

const DAEMON_STATUS_SCHEMA: u32 = 1;
const DAEMON_STATUS_FRESH_SECS: i64 = 15;

fn daemon_status_path(config_dir: &Path) -> PathBuf {
    config_dir.join("daemon-status.json")
}

fn load_daemon_status(config_dir: &Path) -> Option<Value> {
    let pid = daemon_lock_pid(config_dir);
    let running = pid.is_some_and(process_is_running);
    let now = unix_now();
    let raw = std::fs::read_to_string(daemon_status_path(config_dir)).ok();
    let mut value = raw
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .unwrap_or_else(|| json!({}));
    let object = value.as_object_mut()?;
    let updated_at = object
        .get("updated_at")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let fresh = running && now.saturating_sub(updated_at) <= DAEMON_STATUS_FRESH_SECS;
    object.insert("schema".to_string(), json!(DAEMON_STATUS_SCHEMA));
    object.insert("running".to_string(), json!(running));
    object.insert("pid".to_string(), json!(pid));
    object.insert("fresh".to_string(), json!(fresh));
    if !fresh
        && let Some(fips) = object
            .get_mut("fips_block_sync")
            .and_then(Value::as_object_mut)
    {
        fips.insert("connected_peers".to_string(), json!([]));
    }
    Some(value)
}

fn write_daemon_status(config_dir: &Path, mut payload: Value) {
    let now = unix_now();
    if let Some(payload_object) = payload.as_object_mut()
        && let Ok(raw) = std::fs::read_to_string(daemon_status_path(config_dir))
        && let Ok(existing) = serde_json::from_str::<Value>(&raw)
        && let Some(existing_object) = existing.as_object()
    {
        for key in [
            "last_block_sync",
            "block_sync_by_root",
            "relays",
            "owner_npub",
            "watch_interval_secs",
            "watch_debounce_ms",
            "working_dir",
            "relay_statuses",
            "embedded_hashtree",
            "browser_gateway",
            "fips_block_sync",
            "fips_block_sync_error",
        ] {
            if !payload_object.contains_key(key)
                && let Some(value) = existing_object.get(key)
            {
                payload_object.insert(key.to_string(), value.clone());
            }
        }
    }
    if let Some(object) = payload.as_object_mut() {
        object.insert("schema".to_string(), json!(DAEMON_STATUS_SCHEMA));
        object.insert("pid".to_string(), json!(std::process::id()));
        object.insert("running".to_string(), json!(true));
        object.insert("fresh".to_string(), json!(true));
        object.insert("updated_at".to_string(), json!(now));
    }
    if let Some(parent) = daemon_status_path(config_dir).parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(bytes) = serde_json::to_vec_pretty(&payload) {
        let _ = std::fs::write(daemon_status_path(config_dir), bytes);
    }
}

fn merge_daemon_status(
    config_dir: &Path,
    update: impl FnOnce(&mut serde_json::Map<String, Value>),
) {
    let mut value = std::fs::read_to_string(daemon_status_path(config_dir))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .unwrap_or_else(|| json!({}));
    if !value.is_object() {
        value = json!({});
    }
    if let Some(object) = value.as_object_mut() {
        update(object);
    }
    write_daemon_status(config_dir, value);
}

fn daemon_lock_pid(config_dir: &Path) -> Option<u32> {
    std::fs::read_to_string(config_dir.join("daemon.lock"))
        .ok()
        .and_then(|contents| contents.trim().parse::<u32>().ok())
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

fn cmd_conflicts(config_dir: &std::path::Path, command: ConflictsCmd) -> Result<()> {
    match command {
        ConflictsCmd::Resolve { conflict_id } => cmd_conflict_resolve(config_dir, &conflict_id),
    }
}

fn cmd_conflict_resolve(config_dir: &std::path::Path, conflict_id: &str) -> Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let mut daemon = Daemon::open(config_dir)
            .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
        let report = daemon
            .resolve_conflict_record(conflict_id)
            .await
            .with_context(|| format!("resolving conflict {conflict_id}"))?;
        println!(
            "{}",
            json!({
                "conflict_id": report.conflict_id,
                "previous_root_cid": report.previous_root_cid,
                "root_cid": report.root_cid,
                "changed": report.changed,
            })
        );
        Ok::<_, anyhow::Error>(())
    })
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

const DRIVE_IRIS_TO_ORIGIN: &str = "https://drive.iris.to";

fn drive_iris_to_url_for_primary_drive(config: &AppConfig) -> Option<String> {
    let drive = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)?;
    Some(drive_iris_to_url_for_drive(
        &drive.owner_pubkey,
        &drive.drive_id,
    ))
}

fn drive_iris_to_url_for_drive(owner_pubkey_hex: &str, drive_id: &str) -> String {
    format!(
        "{DRIVE_IRIS_TO_ORIGIN}/#/{}/{}",
        account_npub(owner_pubkey_hex),
        percent_encode_path_segment(drive_id)
    )
}

fn drive_iris_to_snapshot_url_for_root(root_cid: &str) -> Option<String> {
    let cid = Cid::parse(root_cid).ok()?;
    let nhash = nhash_encode_full(&NHashData {
        hash: cid.hash,
        decrypt_key: cid.key,
    })
    .ok()?;
    Some(format!("{DRIVE_IRIS_TO_ORIGIN}/#/{nhash}"))
}

fn local_gateway_urls_for_root(root_cid: Option<&str>, port: u16) -> serde_json::Value {
    let immutable_url = root_cid
        .and_then(|root| Cid::parse(root).ok())
        .map(|cid| iris_drive_core::gateway::local_immutable_url(port, &cid));
    json!({
        "portal_url": format!("http://sites.iris.localhost:{port}/"),
        "primary_drive_url": iris_drive_core::gateway::local_drive_url(
            port,
            iris_drive_core::PRIMARY_DRIVE_ID,
        ),
        "immutable_url": immutable_url,
    })
}

fn percent_encode_path_segment(segment: &str) -> String {
    let mut encoded = String::new();
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(&mut encoded, "%{byte:02X}");
            }
        }
    }
    encoded
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

fn root_file_count(config_dir: &Path, root_cid: &str) -> Option<usize> {
    let cid = Cid::parse(root_cid).ok()?;
    let daemon = Daemon::open(config_dir).ok()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    runtime
        .block_on(async { walk_device_tree(daemon.tree(), &cid).await.ok() })
        .map(|(files, _tombstones)| files.len())
}

fn primary_drive_counts(config_dir: &Path, config: &AppConfig) -> Option<(usize, usize)> {
    let daemon = Daemon::open(config_dir).ok()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    runtime
        .block_on(async {
            iris_drive_core::primary_merged_view(daemon.tree(), config)
                .await
                .ok()
        })
        .map(|merged| (merged.file_count(), merged.top_level_entries()))
}

fn root_conflict_status(config_dir: &Path, root_cid: &str) -> Option<serde_json::Value> {
    let cid = Cid::parse(root_cid).ok()?;
    let daemon = Daemon::open(config_dir).ok()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let records = runtime.block_on(async {
        iris_drive_core::read_conflict_records(daemon.tree(), &cid)
            .await
            .ok()
    })?;
    Some(conflict_status_payload(&records))
}

fn conflict_status_payload(records: &[iris_drive_core::ConflictRecord]) -> serde_json::Value {
    let unresolved_records: Vec<_> = records
        .iter()
        .filter(|record| record.state == iris_drive_core::ConflictState::Unresolved)
        .collect();
    let unresolved: Vec<_> = unresolved_records
        .iter()
        .map(|record| conflict_record_status_payload(record))
        .collect();
    let overflow_paths = conflict_overflow_payload(&unresolved_records);
    let resolved_count = records.len().saturating_sub(unresolved.len());

    json!({
        "total_count": records.len(),
        "unresolved_count": unresolved.len(),
        "resolved_count": resolved_count,
        "per_path_cap": CONFLICT_STATUS_PATH_CAP,
        "overflow_count": overflow_paths.len(),
        "overflow_paths": overflow_paths,
        "unresolved": unresolved,
    })
}

fn conflict_overflow_payload(
    records: &[&iris_drive_core::ConflictRecord],
) -> Vec<serde_json::Value> {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for record in records {
        *counts.entry(record.path.as_str()).or_default() += 1;
    }
    counts
        .into_iter()
        .filter(|(_, count)| *count > CONFLICT_STATUS_PATH_CAP)
        .map(|(path, count)| {
            json!({
                "path": path,
                "unresolved_count": count,
                "cap": CONFLICT_STATUS_PATH_CAP,
            })
        })
        .collect()
}

fn conflict_record_status_payload(record: &iris_drive_core::ConflictRecord) -> serde_json::Value {
    json!({
        "conflict_id": record.conflict_id.as_str(),
        "path": record.path.as_str(),
        "visible_conflict_path": record.visible_conflict_path.as_str(),
        "created_at": record.created_at,
        "state": conflict_state_label(&record.state),
    })
}

fn conflict_state_label(state: &iris_drive_core::ConflictState) -> &'static str {
    match state {
        iris_drive_core::ConflictState::Unresolved => "unresolved",
        iris_drive_core::ConflictState::Resolved => "resolved",
    }
}

fn peer_statuses(
    config_dir: &Path,
    config: &AppConfig,
    daemon_status: Option<&Value>,
) -> Vec<serde_json::Value> {
    let Some(account) = config.account.as_ref() else {
        return Vec::new();
    };
    let Some(snapshot) = account.app_keys.as_ref() else {
        return Vec::new();
    };
    let primary_drive = config.drive(iris_drive_core::PRIMARY_DRIVE_ID);

    let daemon_running = daemon_status
        .and_then(|status| status.get("running"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fips_status = daemon_status
        .and_then(|status| status.get("fips_block_sync"))
        .filter(|value| value.is_object());
    let connected_fips =
        string_set_from_json_array(fips_status.and_then(|status| status.get("connected_peers")));
    let authorized_fips =
        string_set_from_json_array(fips_status.and_then(|status| status.get("authorized_peers")));
    let block_sync_by_root = daemon_status
        .and_then(|status| status.get("block_sync_by_root"))
        .filter(|value| value.is_object());

    snapshot
        .devices
        .iter()
        .map(|device| {
            let root = primary_drive.and_then(|drive| drive.device_roots.get(&device.pubkey));
            let root_cid = root.map(|root| root.root_cid.clone());
            let root_private = root_cid.as_deref().and_then(root_is_private);
            let root_available = root_cid
                .as_deref()
                .map(|root| root_file_count(config_dir, root).is_some());
            let device_npub = account_npub(&device.pubkey);
            let is_current_device = device.pubkey == account.device_pubkey;
            let fips_online = if is_current_device {
                daemon_running && fips_status.is_some()
            } else {
                connected_fips.contains(&device_npub)
            };
            let sync_state = device_sync_state(is_current_device, root.is_some(), root_available);
            let last_block_sync = root_cid
                .as_ref()
                .and_then(|root| block_sync_by_root.and_then(|map| map.get(root)).cloned());
            json!({
                "device_pubkey": device.pubkey,
                "device_npub": device_npub,
                "label": device.label,
                "authorized": true,
                "is_current_device": is_current_device,
                "added_at": device.added_at,
                "fips_authorized": authorized_fips.contains(&device_npub),
                "fips_online": fips_online,
                "has_root": root.is_some(),
                "root_cid": root_cid,
                "root_private": root_private,
                "root_available": root_available,
                "sync_state": sync_state,
                "last_block_sync": last_block_sync,
                "published_at": root.map(|root| root.published_at),
                "dck_generation": root.map(|root| root.dck_generation),
                "device_seq": root.map(|root| root.device_seq),
            })
        })
        .collect()
}

fn fips_network_diagnostics(config: &AppConfig, daemon_status: Option<&Value>) -> Value {
    let running = daemon_status
        .and_then(|status| status.get("running"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fresh = daemon_status
        .and_then(|status| status.get("fresh"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let fips_status = daemon_status
        .and_then(|status| status.get("fips_block_sync"))
        .filter(|value| value.is_object());
    let mut authorized_peers =
        string_vec_from_json_array(fips_status.and_then(|status| status.get("authorized_peers")));
    if authorized_peers.is_empty() {
        authorized_peers = configured_fips_authorized_peer_npubs(config);
    }
    let connected_peers =
        string_vec_from_json_array(fips_status.and_then(|status| status.get("connected_peers")));
    let authorized_set = authorized_peers.iter().cloned().collect::<BTreeSet<_>>();
    let connected_set = connected_peers.iter().cloned().collect::<BTreeSet<_>>();
    let roster_connected_peer_count = connected_set.intersection(&authorized_set).count();
    let other_peer_count = connected_set.difference(&authorized_set).count();
    let error = daemon_status
        .and_then(|status| status.get("fips_block_sync_error"))
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or(Value::Null);

    json!({
        "enabled": fips_status.is_some(),
        "running": running,
        "fresh": fresh,
        "endpoint_npub": fips_status
            .and_then(|status| status.get("endpoint_npub"))
            .and_then(Value::as_str),
        "discovery_scope": fips_status
            .and_then(|status| status.get("discovery_scope"))
            .and_then(Value::as_str),
        "roster_peer_count": authorized_peers.len(),
        "roster_connected_peer_count": roster_connected_peer_count,
        "authorized_peer_count": authorized_peers.len(),
        "connected_peer_count": connected_peers.len(),
        "other_peer_count": other_peer_count,
        "authorized_peers": authorized_peers,
        "connected_peers": connected_peers,
        "error": error,
    })
}

fn configured_fips_authorized_peer_npubs(config: &AppConfig) -> Vec<String> {
    let Some(account) = config.account.as_ref() else {
        return Vec::new();
    };
    let Some(snapshot) = account.app_keys.as_ref() else {
        return Vec::new();
    };

    snapshot
        .devices
        .iter()
        .filter(|device| device.pubkey != account.device_pubkey)
        .map(|device| account_npub(&device.pubkey))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn string_vec_from_json_array(value: Option<&Value>) -> Vec<String> {
    string_set_from_json_array(value).into_iter().collect()
}

fn string_set_from_json_array(value: Option<&Value>) -> BTreeSet<String> {
    value
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn device_sync_state(
    is_current_device: bool,
    has_root: bool,
    root_available: Option<bool>,
) -> &'static str {
    if is_current_device {
        return if has_root { "local" } else { "not imported" };
    }
    match (has_root, root_available) {
        (false, _) => "waiting for root",
        (true, Some(true)) => "synced",
        (true, Some(false)) => "blocks pending",
        (true, None) => "metadata only",
    }
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
            "device_link_request": device_link_request_json(&state),
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

fn cmd_relays(config_dir: &std::path::Path, sub: Option<RelaysCmd>) -> Result<()> {
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

fn normalize_relay_url(value: &str) -> String {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.starts_with("ws://") || trimmed.starts_with("wss://") {
        trimmed.to_string()
    } else {
        format!("wss://{trimmed}")
    }
}

fn dedupe_relays(relays: &mut Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    relays.retain(|relay| seen.insert(relay.clone()));
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

fn cmd_backups(config_dir: &std::path::Path, sub: BackupsCmd) -> Result<()> {
    if let BackupsCmd::Sync { target } = sub {
        return cmd_backups_sync(config_dir, target.as_deref());
    }

    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    match sub {
        BackupsCmd::List | BackupsCmd::Sync { .. } => {}
        BackupsCmd::Add { target, label } => {
            let target = parse_backup_target(&target, label).context("parsing backup target")?;
            config.upsert_backup_target(target);
            config.save(config_path_in(config_dir))?;
        }
        BackupsCmd::Remove { target } => {
            let target_id = parse_backup_target(&target, None)
                .context("parsing backup target")?
                .id;
            if config.remove_backup_target(&target_id).is_some() {
                config.save(config_path_in(config_dir))?;
            }
        }
    }
    println!(
        "{}",
        json!({
            "backup_targets": backup_targets_status(&config),
        })
    );
    Ok(())
}

fn cmd_backups_sync(config_dir: &std::path::Path, target: Option<&str>) -> Result<()> {
    let target_id = target
        .map(|target| parse_backup_target(target, None).map(|target| target.id))
        .transpose()
        .context("parsing backup target")?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(async {
        let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
        let root_cid_str = current_primary_root_cid(&config)
            .ok_or_else(|| anyhow::anyhow!("no current drive root; import files first"))?;
        let root_cid = Cid::parse(&root_cid_str).context("parsing current root cid")?;
        let device = iris_drive_core::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
        let daemon = Daemon::open(config_dir).context("opening daemon for backup upload")?;
        let mut reports = Vec::new();

        for index in 0..config.backup_targets.len() {
            let target = config.backup_targets[index].clone();
            if !target.enabled {
                continue;
            }
            if let Some(target_id) = target_id.as_deref()
                && target.id != target_id
            {
                continue;
            }

            match target.kind {
                BackupTargetKind::Blossom => {
                    let servers = vec![target.target.clone()];
                    let client =
                        iris_drive_core::blossom_sync_client(device.keys().clone(), &servers);
                    match iris_drive_core::blossom_sync::upload_tree(
                        daemon.tree(),
                        &root_cid,
                        &client,
                    )
                    .await
                    {
                        Ok(upload) => {
                            let sync = BackupTargetSync {
                                state: "synced".to_string(),
                                root_cid: root_cid_str.clone(),
                                synced_at: unix_now(),
                                total_hashes: upload.total_hashes,
                                uploaded: upload.uploaded,
                                already_present: upload.already_present,
                            };
                            config.backup_targets[index].last_sync = Some(sync);
                            reports.push(json!({
                                "id": target.id,
                                "kind": "blossom",
                                "target": target.target,
                                "label": target.label,
                                "state": "synced",
                                "root_cid": root_cid_str.as_str(),
                                "upload": upload_report_json(&upload),
                            }));
                        }
                        Err(error) => {
                            reports.push(json!({
                                "id": target.id,
                                "kind": "blossom",
                                "target": target.target,
                                "label": target.label,
                                "state": "error",
                                "root_cid": root_cid_str.as_str(),
                                "error": error.to_string(),
                            }));
                        }
                    }
                }
                BackupTargetKind::Fips => {
                    reports.push(json!({
                        "id": target.id,
                        "kind": "fips",
                        "target": target.target,
                        "label": target.label,
                        "state": "pending",
                        "root_cid": root_cid_str.as_str(),
                        "error": "direct FIPS backup transport pending",
                    }));
                }
            }
        }

        config.save(config_path_in(config_dir))?;
        println!("{}", json!({ "reports": reports }));
        Ok::<_, anyhow::Error>(())
    })
}

fn parse_backup_target(input: &str, label: Option<String>) -> Result<BackupTarget> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(anyhow::anyhow!("backup target is required"));
    }

    let (kind_hint, value) = if let Some(rest) = trimmed.strip_prefix("blossom:") {
        (Some(BackupTargetKind::Blossom), rest)
    } else if let Some(rest) = trimmed.strip_prefix("fips://") {
        (Some(BackupTargetKind::Fips), rest)
    } else if let Some(rest) = trimmed.strip_prefix("fips:") {
        (Some(BackupTargetKind::Fips), rest)
    } else {
        (None, trimmed)
    };

    let target_label = label
        .map(|label| label.trim().to_string())
        .filter(|label| !label.is_empty());
    match kind_hint {
        Some(BackupTargetKind::Blossom) | None
            if value.starts_with("http://") || value.starts_with("https://") =>
        {
            let target = normalize_blossom_url(value)?;
            Ok(BackupTarget {
                id: format!("blossom:{target}"),
                kind: BackupTargetKind::Blossom,
                target,
                label: target_label,
                enabled: true,
                last_sync: None,
            })
        }
        Some(BackupTargetKind::Fips) | None => {
            let hex = normalize_pubkey(value)?;
            let target = account_npub(&hex);
            Ok(BackupTarget {
                id: format!("fips:{target}"),
                kind: BackupTargetKind::Fips,
                target,
                label: target_label,
                enabled: true,
                last_sync: None,
            })
        }
        Some(BackupTargetKind::Blossom) => Err(anyhow::anyhow!(
            "expected Blossom target URL starting with http:// or https://"
        )),
    }
}

fn normalize_blossom_url(value: &str) -> Result<String> {
    let trimmed = value.trim().trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        Ok(trimmed.to_string())
    } else {
        Err(anyhow::anyhow!(
            "expected Blossom target URL starting with http:// or https://"
        ))
    }
}

fn upload_report_json(report: &UploadReport) -> Value {
    json!({
        "total_hashes": report.total_hashes,
        "uploaded": report.uploaded,
        "already_present": report.already_present,
    })
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

        let report = publish_current_state(&client, config_dir, &config, &state, true).await?;

        let _ = client.disconnect().await;
        let drive_iris_to_url = report
            .root_cid
            .as_ref()
            .and_then(|_| drive_iris_to_url_for_primary_drive(&config));
        let snapshot_url = report
            .root_cid
            .as_deref()
            .and_then(drive_iris_to_snapshot_url_for_root);
        println!(
            "{}",
            json!({
                "relays": relays,
                "blossom_servers": config.blossom_servers,
                "published_app_keys": report.published_app_keys,
                "app_keys_publish_error": report.app_keys_publish_error,
                "published_drive_root": report.published_drive_root,
                "drive_root_publish_error": report.drive_root_publish_error,
                "published_files_root": report.published_files_root,
                "files_root_publish_error": report.files_root_publish_error,
                "root_cid": report.root_cid,
                "drive_iris_to_url": drive_iris_to_url,
                "files_iris_to_url": drive_iris_to_url,
                "snapshot_url": snapshot_url,
                "permalink_url": snapshot_url,
                "blossom_upload_error": report.blossom_upload_error,
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
    app_keys_publish_error: Option<String>,
    published_drive_root: bool,
    drive_root_publish_error: Option<String>,
    published_files_root: bool,
    files_root_publish_error: Option<String>,
    root_cid: Option<String>,
    blossom_upload: Option<UploadReport>,
    blossom_upload_error: Option<String>,
}

#[derive(Debug, Clone)]
struct DirectRootEvent {
    key: String,
    event_id: String,
    kind: u16,
    json: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct DirectRootFrame {
    key: String,
    event_id: String,
    event_json: String,
}

#[derive(Default)]
struct DirectRootExchange {
    cached_events: BTreeMap<String, DirectRootEvent>,
    seen_keys: BTreeSet<String>,
    subscribed_streams: BTreeSet<String>,
}

impl DirectRootExchange {
    async fn subscribe_owner_stream(&mut self, owner_pubkey: &str, sync: Option<&FsFipsBlockSync>) {
        let Some(sync) = sync else {
            self.subscribed_streams.clear();
            return;
        };
        let stream = direct_root_mesh_stream(owner_pubkey);
        if self.subscribed_streams.insert(stream.clone()) {
            let subscribe_stats = sync.subscribe_mesh_pubsub(stream.clone()).await;
            println!(
                "{}",
                json!({
                    "event": "direct_root_mesh_subscribe",
                    "stream": stream,
                    "selected_peers": subscribe_stats.selected_peers,
                    "sent_peers": subscribe_stats.sent_peers,
                })
            );
        }
    }

    async fn announce_current_state(
        &mut self,
        config_dir: &Path,
        config: &AppConfig,
        state: &AccountState,
        fips_blocks: Option<&FsFipsBlockSync>,
    ) -> Result<()> {
        let Some(sync) = fips_blocks else {
            return Ok(());
        };
        self.subscribe_owner_stream(&state.owner_pubkey, Some(sync))
            .await;
        let stream = direct_root_mesh_stream(&state.owner_pubkey);
        let events = build_current_sync_events(config_dir, config, state).await?;
        for event in events {
            self.cache_event(event.clone());
            let frame = DirectRootFrame {
                key: event.key.clone(),
                event_id: event.event_id.clone(),
                event_json: event.json.clone(),
            };
            let bytes = serde_json::to_vec(&frame)?;
            let publish_stats = sync
                .publish_mesh_pubsub(stream.clone(), direct_root_seq(&event.key), bytes)
                .await;
            println!(
                "{}",
                json!({
                    "event": "direct_root_mesh_publish",
                    "stream": stream,
                    "root_key": event.key,
                    "root_event_id": event.event_id,
                    "kind": event.kind,
                    "selected_peers": publish_stats.selected_peers,
                    "sent_peers": publish_stats.sent_peers,
                    "sent_bytes": publish_stats.sent_bytes,
                })
            );
        }
        Ok(())
    }

    async fn request_roots_from_new_peers(
        &mut self,
        config_dir: &Path,
        sync: Option<&FsFipsBlockSync>,
    ) {
        let Some(sync) = sync else {
            self.subscribed_streams.clear();
            return;
        };
        let Ok(config) = AppConfig::load_or_default(config_path_in(config_dir)) else {
            return;
        };
        if let Some(state) = config.account.as_ref() {
            self.subscribe_owner_stream(&state.owner_pubkey, Some(sync))
                .await;
        }
    }

    async fn drain_mesh_events(
        &mut self,
        client: &nostr_sdk::Client,
        config_dir: &Path,
        sync: Arc<FsFipsBlockSync>,
    ) -> Result<()> {
        for message in sync.drain_mesh_pubsub_events().await {
            if !message
                .stream_id
                .starts_with(DIRECT_ROOT_MESH_STREAM_PREFIX)
            {
                continue;
            }
            let frame: DirectRootFrame =
                serde_json::from_slice(&message.payload).context("parsing mesh root frame")?;
            if self.seen_keys.contains(&frame.key) {
                continue;
            }
            let event: Event =
                serde_json::from_str(&frame.event_json).context("parsing mesh root event")?;
            if event.id.to_hex() != frame.event_id {
                return Err(anyhow::anyhow!("direct mesh root event id mismatch"));
            }
            self.seen_keys.insert(frame.key.clone());
            if let Err(error) =
                apply_one_event(client, config_dir, &event, Some(sync.clone())).await
            {
                self.seen_keys.remove(&frame.key);
                return Err(error);
            }
            println!(
                "{}",
                json!({
                    "event": "direct_root_mesh_event",
                    "stream": message.stream_id,
                    "peer": message.from_peer_id,
                    "origin": message.origin_peer_id,
                    "seq": message.seq,
                    "root_key": frame.key,
                    "root_event_id": frame.event_id,
                })
            );
            let config = AppConfig::load_or_default(config_path_in(config_dir))?;
            if let Some(state) = config.account.as_ref() {
                self.announce_current_state(config_dir, &config, state, Some(sync.as_ref()))
                    .await?;
            }
        }
        Ok(())
    }

    fn cache_event(&mut self, event: DirectRootEvent) {
        self.seen_keys.insert(event.key.clone());
        self.cached_events.insert(event.key.clone(), event);
        while self.cached_events.len() > DIRECT_ROOT_EVENT_CACHE_CAP {
            let Some(key) = self.cached_events.keys().next().cloned() else {
                break;
            };
            self.cached_events.remove(&key);
        }
    }
}

async fn build_current_sync_events(
    config_dir: &Path,
    config: &AppConfig,
    state: &AccountState,
) -> Result<Vec<DirectRootEvent>> {
    let mut events = Vec::new();

    if state.has_owner_signing_authority
        && let Some(snap) = state.app_keys.as_ref()
    {
        let account = Account::load(state.clone(), config_dir).context("loading account")?;
        let owner_keys = account
            .owner_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("owner key missing on disk"))?
            .keys();
        let event = iris_drive_core::nostr_events::build_app_keys_event(owner_keys, snap)
            .context("building AppKeys event")?;
        events.push(direct_root_event(
            format!(
                "appkeys:{}:{}:{}:{}",
                snap.owner_pubkey,
                snap.created_at,
                snap.dck_generation,
                snap.devices
                    .iter()
                    .map(|device| device.pubkey.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            &event,
        )?);
    }

    if let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)
        && let Some(root) = publishable_device_root(config_dir, drive, state).await?
    {
        let authorized_devices = authorized_device_pubkeys(state);
        let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
        let event = iris_drive_core::nostr_events::build_drive_root_event(
            device.keys(),
            &state.owner_pubkey,
            &drive.drive_id,
            &root,
            &authorized_devices,
        )
        .context("building drive-root event")?;
        events.push(direct_root_event(
            format!(
                "drive-root:{}:{}:{}:{}:{}",
                state.device_pubkey,
                drive.drive_id,
                root.device_seq,
                root.root_cid,
                authorized_devices.join(",")
            ),
            &event,
        )?);

        if state.has_owner_signing_authority {
            let account = Account::load(state.clone(), config_dir).context("loading account")?;
            let owner_keys = account
                .owner_key
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("owner key missing on disk"))?
                .keys();
            let event = iris_drive_core::nostr_events::build_private_hashtree_root_event(
                owner_keys,
                &drive.drive_id,
                &root,
            )
            .context("building files-root event")?;
            events.push(direct_root_event(
                format!(
                    "files-root:{}:{}:{}",
                    state.owner_pubkey, drive.drive_id, root.root_cid
                ),
                &event,
            )?);
        }
    }

    Ok(events)
}

async fn publishable_device_root(
    config_dir: &Path,
    drive: &Drive,
    state: &AccountState,
) -> Result<Option<DeviceRootRef>> {
    let Some(root) = drive.device_roots.get(&state.device_pubkey).cloned() else {
        return Ok(None);
    };
    if !root.materialized_only {
        return Ok(Some(root));
    }
    publishable_parent_root(config_dir, state, root).await
}

async fn publishable_parent_root(
    config_dir: &Path,
    state: &AccountState,
    mut root: DeviceRootRef,
) -> Result<Option<DeviceRootRef>> {
    let daemon = Daemon::open(config_dir).context("opening daemon for publishable root lookup")?;
    let mut seen = BTreeSet::new();
    for _ in 0..32 {
        if !seen.insert(root.root_cid.clone()) {
            return Ok(None);
        }
        let cid = Cid::parse(&root.root_cid)
            .with_context(|| format!("parsing root cid {}", root.root_cid))?;
        let Some(meta) = iris_drive_core::indexer::read_root_meta(daemon.tree(), &cid)
            .await
            .with_context(|| format!("reading root metadata for {}", root.root_cid))?
        else {
            return Ok(None);
        };
        let Some(parent) = meta
            .parents
            .iter()
            .find(|parent| parent.device_id == state.device_pubkey)
        else {
            return Ok(None);
        };
        let parent_cid = Cid::parse(&parent.root_cid)
            .with_context(|| format!("parsing parent root cid {}", parent.root_cid))?;
        let parent_root = match iris_drive_core::indexer::read_root_meta(daemon.tree(), &parent_cid)
            .await
            .with_context(|| format!("reading parent root metadata for {}", parent.root_cid))?
        {
            Some(parent_meta) => DeviceRootRef::from_meta(
                parent.root_cid.clone(),
                parent_meta.created_at,
                &parent_meta,
            ),
            None => DeviceRootRef::legacy(
                parent.root_cid.clone(),
                root.published_at,
                root.dck_generation,
            ),
        };
        if !parent_root.materialized_only {
            return Ok(Some(parent_root));
        }
        root = parent_root;
    }
    Ok(None)
}

fn direct_root_event(key: String, event: &Event) -> Result<DirectRootEvent> {
    Ok(DirectRootEvent {
        key,
        event_id: event.id.to_hex(),
        kind: event.kind.as_u16(),
        json: serde_json::to_string(&event)?,
    })
}

fn direct_root_mesh_stream(owner_pubkey: &str) -> String {
    format!("{DIRECT_ROOT_MESH_STREAM_PREFIX}/{owner_pubkey}")
}

fn direct_root_seq(key: &str) -> u64 {
    let hash = hashtree_core::sha256(key.as_bytes());
    let mut bytes = [0u8; 8];
    bytes.copy_from_slice(&hash[..8]);
    u64::from_be_bytes(bytes).max(1)
}

async fn announce_current_state_direct(
    direct_roots: &mut DirectRootExchange,
    config_dir: &Path,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(state) = config.account.as_ref() else {
        return Ok(());
    };
    direct_roots
        .announce_current_state(config_dir, &config, state, fips_blocks)
        .await
}

async fn upload_tree_to_blossom_with_hashtree(
    config_dir: &std::path::Path,
    config: &AppConfig,
    device: &iris_drive_core::DeviceIdentity,
    root_cid: Cid,
    _previous_root_cid: Option<Cid>,
) -> Result<UploadReport> {
    if config.blossom_servers.is_empty() {
        return Err(anyhow::anyhow!("no blossom servers configured"));
    }

    let bclient =
        iris_drive_core::blossom_sync_client(device.keys().clone(), &config.blossom_servers);
    let daemon = Daemon::open(config_dir).context("opening daemon for blossom upload")?;
    iris_drive_core::blossom_sync::upload_tree(daemon.tree(), &root_cid, &bclient)
        .await
        .context("uploading tree to blossom")
}

async fn maybe_upload_root_to_blossom(
    config_dir: &std::path::Path,
    config: &AppConfig,
    device: &iris_drive_core::DeviceIdentity,
    root_cid_str: &str,
    previous_root_cid: Option<&str>,
) -> Result<(Option<UploadReport>, Option<String>)> {
    if config.blossom_servers.is_empty() {
        return Ok((None, None));
    }

    let root_cid =
        Cid::parse(root_cid_str).with_context(|| format!("parsing root cid {root_cid_str}"))?;
    let previous_root_cid = previous_root_cid
        .map(Cid::parse)
        .transpose()
        .context("parsing previous root cid")?;
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(BLOSSOM_UPLOAD_TIMEOUT_SECS),
        upload_tree_to_blossom_with_hashtree(
            config_dir,
            config,
            device,
            root_cid,
            previous_root_cid,
        ),
    )
    .await;
    Ok(match result {
        Ok(Ok(upload)) => (Some(upload), None),
        Ok(Err(error)) => (None, Some(format!("{error:#}"))),
        Err(_) => (
            None,
            Some(format!("timed out after {BLOSSOM_UPLOAD_TIMEOUT_SECS}s")),
        ),
    })
}

async fn start_fips_block_sync(
    config_dir: &std::path::Path,
    config: &AppConfig,
) -> Result<FsFipsBlockSync> {
    let device = iris_drive_core::DeviceIdentity::load(key_path_in(config_dir))
        .context("loading device key")?;
    let daemon = Daemon::open(config_dir).context("opening daemon for direct FIPS sync")?;
    let local = daemon.tree().get_store().clone();
    iris_drive_core::FipsBlockSync::start(&device, local, config)
        .await
        .context("starting direct FIPS block sync")
}

async fn download_roots_over_fips(
    fips: &FsFipsBlockSync,
    root_cid_strs: &[String],
    policy: FipsDownloadPolicy,
) -> Result<DownloadReport> {
    let mut totals = DownloadReport::default();
    for cid_str in root_cid_strs {
        let cid = Cid::parse(cid_str).with_context(|| format!("parsing root cid {cid_str}"))?;
        let report = download_tree_over_fips_with_retry(fips, &cid, policy)
            .await
            .with_context(|| format!("downloading tree over FIPS for {cid_str}"))?;
        add_download_report(&mut totals, report);
    }
    Ok(totals)
}

async fn download_tree_over_fips_with_retry(
    fips: &FsFipsBlockSync,
    root: &Cid,
    policy: FipsDownloadPolicy,
) -> Result<DownloadReport> {
    let mut last_error: Option<anyhow::Error> = None;
    for delay in std::iter::once(0).chain(policy.retry_delays.iter().copied()) {
        if delay > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        }
        match tokio::time::timeout(policy.attempt_timeout, fips.download_tree(root)).await {
            Ok(Ok(report)) => return Ok(report),
            Ok(Err(error)) => last_error = Some(anyhow::Error::from(error)),
            Err(_) => {
                last_error = Some(anyhow::anyhow!(
                    "FIPS download timed out after {}s",
                    policy.attempt_timeout.as_secs()
                ));
            }
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("FIPS download failed")))
}

#[derive(Clone, Copy)]
struct FipsDownloadPolicy {
    retry_delays: &'static [u64],
    attempt_timeout: std::time::Duration,
}

fn fips_download_policy(config: &AppConfig) -> FipsDownloadPolicy {
    if config.blossom_servers.is_empty() {
        FipsDownloadPolicy {
            retry_delays: FIPS_DOWNLOAD_RETRY_DELAYS,
            attempt_timeout: std::time::Duration::from_secs(FIPS_DOWNLOAD_ATTEMPT_TIMEOUT_SECS),
        }
    } else {
        FipsDownloadPolicy {
            retry_delays: FIPS_DOWNLOAD_BEFORE_BLOSSOM_RETRY_DELAYS,
            attempt_timeout: std::time::Duration::from_secs(
                FIPS_DOWNLOAD_BEFORE_BLOSSOM_ATTEMPT_TIMEOUT_SECS,
            ),
        }
    }
}

async fn download_roots_over_blossom(
    config_dir: &std::path::Path,
    config: &AppConfig,
    root_cid_strs: &[String],
) -> Result<DownloadReport> {
    if config.blossom_servers.is_empty() {
        return Err(anyhow::anyhow!("no blossom servers configured"));
    }

    let device = iris_drive_core::DeviceIdentity::load(key_path_in(config_dir))
        .context("loading device key")?;
    let daemon = Daemon::open(config_dir).context("opening daemon for Blossom sync")?;
    let local = daemon.tree().get_store().clone();
    let bclient =
        iris_drive_core::blossom_sync_client(device.keys().clone(), &config.blossom_servers);
    let mut totals = DownloadReport::default();
    for cid_str in root_cid_strs {
        let cid = Cid::parse(cid_str).with_context(|| format!("parsing root cid {cid_str}"))?;
        let report = iris_drive_core::blossom_sync::download_tree_with_retry(
            local.clone(),
            &cid,
            bclient.clone(),
            BLOSSOM_DOWNLOAD_RETRY_DELAYS,
        )
        .await
        .with_context(|| format!("downloading tree from Blossom for {cid_str}"))?;
        add_download_report(&mut totals, report);
    }
    Ok(totals)
}

fn add_download_report(total: &mut DownloadReport, report: DownloadReport) {
    total.total_hashes += report.total_hashes;
    total.fetched += report.fetched;
    total.already_local += report.already_local;
}

fn download_report_json(report: &DownloadReport) -> serde_json::Value {
    json!({
        "total_hashes": report.total_hashes,
        "fetched": report.fetched,
        "already_local": report.already_local,
    })
}

fn materialize_report_json(report: &MaterializeWorkingDirReport) -> serde_json::Value {
    json!({
        "written": report.materialize.written,
        "updated": report.materialize.updated,
        "deleted": report.materialize.deleted,
        "unchanged": report.materialize.unchanged,
        "skipped": report.materialize.skipped,
        "changed": report.materialize.changed(),
        "import": report.import.as_ref().map(|import| json!({
            "working_dir": import.working_dir.display().to_string(),
            "root_cid": import.root_cid,
            "file_count": import.file_count,
            "top_level_entries": import.top_level_entries,
        })),
    })
}

fn publish_state_report_json(report: &PublishStateReport) -> serde_json::Value {
    json!({
        "published_app_keys": report.published_app_keys,
        "app_keys_publish_error": report.app_keys_publish_error,
        "published_drive_root": report.published_drive_root,
        "drive_root_publish_error": report.drive_root_publish_error,
        "published_files_root": report.published_files_root,
        "files_root_publish_error": report.files_root_publish_error,
        "root_cid": report.root_cid,
        "blossom_upload_error": report.blossom_upload_error,
        "blossom_upload": report.blossom_upload.as_ref().map(|r| json!({
            "total_hashes": r.total_hashes,
            "uploaded": r.uploaded,
            "already_present": r.already_present,
        })),
    })
}

async fn materialize_working_dir(
    config_dir: &std::path::Path,
) -> Result<Option<MaterializeWorkingDirReport>> {
    let mut daemon = Daemon::open(config_dir)
        .with_context(|| format!("opening daemon at {}", config_dir.display()))?;
    daemon
        .materialize_primary_drive()
        .await
        .context("materializing merged drive")
}

async fn materialize_and_publish(
    client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    event_name: &str,
) -> Result<()> {
    let Some(report) = materialize_working_dir(config_dir).await? else {
        return Ok(());
    };
    if !report.materialize.changed() {
        return Ok(());
    }
    let updated_config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let Some(updated_state) = updated_config.account.clone() else {
        return Err(anyhow::anyhow!("missing account after materialize"));
    };
    let publish =
        publish_current_state(client, config_dir, &updated_config, &updated_state, true).await?;
    println!(
        "{}",
        json!({
            "event": event_name,
            "materialize": materialize_report_json(&report),
            "publish": publish_state_report_json(&publish),
        })
    );
    Ok(())
}

async fn publish_current_state(
    client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    config: &AppConfig,
    state: &AccountState,
    upload_blossom: bool,
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
        match relay_publish_with_timeout(relay_sync::publish_app_keys(client, owner_keys, snap))
            .await
        {
            Ok(_) => report.published_app_keys = true,
            Err(error) => report.app_keys_publish_error = Some(error),
        }
    }

    if let Some(drive) = config.drive(iris_drive_core::PRIMARY_DRIVE_ID)
        && let Some(root) = publishable_device_root(config_dir, drive, state).await?
    {
        let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
        report.root_cid = Some(root.root_cid.clone());

        if upload_blossom {
            let (blossom_upload, blossom_upload_error) =
                maybe_upload_root_to_blossom(config_dir, config, &device, &root.root_cid, None)
                    .await?;
            report.blossom_upload = blossom_upload;
            report.blossom_upload_error = blossom_upload_error;
        }

        match relay_publish_with_timeout(relay_sync::publish_drive_root(
            client,
            device.keys(),
            &state.owner_pubkey,
            &drive.drive_id,
            &root,
            &authorized_device_pubkeys(state),
        ))
        .await
        {
            Ok(_) => report.published_drive_root = true,
            Err(error) => report.drive_root_publish_error = Some(error),
        }

        if state.has_owner_signing_authority {
            let account = Account::load(state.clone(), config_dir).context("loading account")?;
            let owner_keys = account
                .owner_key
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("owner key missing on disk"))?
                .keys();
            match relay_publish_with_timeout(relay_sync::publish_files_root(
                client,
                owner_keys,
                &drive.drive_id,
                &root,
            ))
            .await
            {
                Ok(_) => report.published_files_root = true,
                Err(error) => {
                    report.files_root_publish_error = Some(error);
                }
            }
        }
    }

    Ok(report)
}

fn spawn_initial_publish(
    client: nostr_sdk::Client,
    config_dir: PathBuf,
    startup_config: AppConfig,
    startup_state: AccountState,
) {
    tokio::spawn(async move {
        match tokio::time::timeout(
            std::time::Duration::from_secs(STARTUP_NETWORK_TIMEOUT_SECS),
            publish_current_state(&client, &config_dir, &startup_config, &startup_state, true),
        )
        .await
        {
            Ok(Ok(report)) => {
                let drive_iris_to_url = report
                    .root_cid
                    .as_ref()
                    .and_then(|_| drive_iris_to_url_for_primary_drive(&startup_config));
                let snapshot_url = report
                    .root_cid
                    .as_deref()
                    .and_then(drive_iris_to_snapshot_url_for_root);
                println!(
                    "{}",
                    json!({
                        "event": "initial_publish",
                        "published_app_keys": report.published_app_keys,
                        "app_keys_publish_error": report.app_keys_publish_error,
                        "published_drive_root": report.published_drive_root,
                        "drive_root_publish_error": report.drive_root_publish_error,
                        "published_files_root": report.published_files_root,
                        "files_root_publish_error": report.files_root_publish_error,
                        "root_cid": report.root_cid,
                        "drive_iris_to_url": drive_iris_to_url,
                        "files_iris_to_url": drive_iris_to_url,
                        "snapshot_url": snapshot_url,
                        "permalink_url": snapshot_url,
                        "blossom_upload_error": report.blossom_upload_error,
                        "blossom_upload": report.blossom_upload.map(|r| json!({
                            "total_hashes": r.total_hashes,
                            "uploaded": r.uploaded,
                            "already_present": r.already_present,
                        })),
                    })
                );
            }
            Ok(Err(error)) => {
                println!(
                    "{}",
                    json!({"event": "initial_publish_error", "error": error.to_string()})
                );
            }
            Err(_) => {
                println!(
                    "{}",
                    json!({
                        "event": "initial_publish_error",
                        "error": format!("timed out after {STARTUP_NETWORK_TIMEOUT_SECS}s"),
                    })
                );
            }
        }
    });
}

#[derive(Clone, Copy)]
struct ScanPublishRequest {
    trigger: &'static str,
    retry_current_root: bool,
    upload_current_to_blossom: bool,
}

impl ScanPublishRequest {
    fn merge(self, next: Self) -> Self {
        Self {
            trigger: if self.trigger == next.trigger {
                self.trigger
            } else {
                "coalesced"
            },
            retry_current_root: self.retry_current_root || next.retry_current_root,
            upload_current_to_blossom: self.upload_current_to_blossom
                || next.upload_current_to_blossom,
        }
    }
}

struct ScanPublishResult {
    trigger: &'static str,
    error: Option<String>,
}

fn queue_scan_publish(
    tx: &tokio::sync::mpsc::Sender<ScanPublishRequest>,
    request: ScanPublishRequest,
) {
    use tokio::sync::mpsc::error::TrySendError;

    if let Err(error) = tx.try_send(request) {
        let reason = match error {
            TrySendError::Full(_) => "full",
            TrySendError::Closed(_) => "closed",
        };
        println!(
            "{}",
            json!({
                "event": "scan_publish_queue_error",
                "trigger": request.trigger,
                "error": reason,
            })
        );
    }
}

fn spawn_scan_publish_worker(
    client: nostr_sdk::Client,
    config_dir: PathBuf,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    mut rx: tokio::sync::mpsc::Receiver<ScanPublishRequest>,
    done_tx: tokio::sync::mpsc::Sender<ScanPublishResult>,
) {
    tokio::spawn(async move {
        while let Some(mut request) = rx.recv().await {
            while let Ok(next) = rx.try_recv() {
                request = request.merge(next);
            }
            let error = scan_and_publish(
                &client,
                &config_dir,
                fips_blocks.as_deref(),
                request.retry_current_root,
                request.upload_current_to_blossom,
            )
            .await
            .err()
            .map(|error| error.to_string());
            if done_tx
                .send(ScanPublishResult {
                    trigger: request.trigger,
                    error,
                })
                .await
                .is_err()
            {
                break;
            }
        }
    });
}

async fn relay_publish_with_timeout<T, F>(future: F) -> std::result::Result<T, String>
where
    F: std::future::Future<
            Output = std::result::Result<T, iris_drive_core::relay_sync::RelayError>,
        >,
{
    relay_publish_with_timeout_duration(
        std::time::Duration::from_secs(RELAY_PUBLISH_TIMEOUT_SECS),
        future,
    )
    .await
}

async fn relay_publish_with_timeout_duration<T, F>(
    timeout: std::time::Duration,
    future: F,
) -> std::result::Result<T, String>
where
    F: std::future::Future<
            Output = std::result::Result<T, iris_drive_core::relay_sync::RelayError>,
        >,
{
    match tokio::time::timeout(timeout, future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err(format!("timed out after {}s", timeout.as_secs())),
    }
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
        let mut root_cids_to_download: Vec<String> = Vec::new();
        let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
        for ev in &drive_root_events {
            let parsed =
                iris_drive_core::nostr_events::parse_drive_root_event_for_device(ev, device.keys())
                    .ok();
            if let Some((_, _, _, root_ref)) = parsed.as_ref()
                && !root_cids_to_download
                    .iter()
                    .any(|root_cid| root_cid == &root_ref.root_cid)
            {
                root_cids_to_download.push(root_ref.root_cid.clone());
            }
            match relay_sync::apply_remote_drive_root_event(&mut config, ev, Some(device.keys()))
                .context("applying drive-root event")?
            {
                relay_sync::DriveRootApply::Applied => {
                    drive_roots_applied += 1;
                }
                _ => drive_roots_skipped += 1,
            }
        }

        // 3) Pull the owner-signed web root for drive.iris.to. This is the
        // standard hashtree mutable-root event used by all web Iris apps, so a
        // native restore of a web account can import browser-origin changes.
        let mut files_root_event_seen = false;
        let mut files_root_event_outcome = "none".to_string();
        let mut files_root_fetch_error: Option<String> = None;
        match relay_sync::fetch_latest_files_root(
            &client,
            &state.owner_pubkey,
            iris_drive_core::PRIMARY_DRIVE_ID,
            timeout,
        )
        .await
        {
            Ok(Some(ev)) => {
                files_root_event_seen = true;
                if state.has_owner_signing_authority {
                    let account_state = config.account.clone().ok_or_else(|| {
                        anyhow::anyhow!("not initialized; run `idrive init` first")
                    })?;
                    let account = Account::load(account_state, config_dir)
                        .context("loading owner account")?;
                    let owner_keys = account
                        .owner_key
                        .as_ref()
                        .map(iris_drive_core::OwnerKey::keys);
                    let outcome =
                        relay_sync::apply_remote_files_root_event(&mut config, &ev, owner_keys)
                            .context("applying files-root event")?;
                    files_root_event_outcome = files_root_apply_label(&outcome).to_string();
                    if matches!(outcome, relay_sync::FilesRootApply::Applied)
                        && let Some(root_ref) = config
                            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
                            .and_then(|drive| drive.device_roots.get(&account.state.device_pubkey))
                        && !root_cids_to_download
                            .iter()
                            .any(|root_cid| root_cid == &root_ref.root_cid)
                    {
                        root_cids_to_download.push(root_ref.root_cid.clone());
                    }
                } else {
                    files_root_event_outcome = "owner_key_unavailable".to_string();
                }
            }
            Ok(None) => {}
            Err(error) => {
                files_root_event_outcome = "fetch_error".to_string();
                files_root_fetch_error = Some(format!("{error:#}"));
            }
        }

        config.save(config_path_in(config_dir))?;

        // 4) Replicate blocks for each seen drive root. Prefer direct
        // FIPS peer transfer between authorized Iris Drive instances;
        // Blossom stays as the public fallback/cache path.
        let mut fips_download_report: Option<DownloadReport> = None;
        let mut fips_download_error: Option<String> = None;
        let mut blossom_download_report: Option<DownloadReport> = None;
        let mut blossom_download_error: Option<String> = None;
        let mut materialize_report: Option<MaterializeWorkingDirReport> = None;
        let mut materialize_error: Option<String> = None;
        let mut materialize_publish_report: Option<PublishStateReport> = None;
        let mut materialize_publish_error: Option<String> = None;
        if !root_cids_to_download.is_empty() {
            let fips_policy = fips_download_policy(&config);
            let mut block_config = config.clone();
            block_config.relays = relays.clone();
            match start_fips_block_sync(config_dir, &block_config).await {
                Ok(fips) => {
                    match download_roots_over_fips(&fips, &root_cids_to_download, fips_policy).await
                    {
                        Ok(report) => fips_download_report = Some(report),
                        Err(error) => fips_download_error = Some(format!("{error:#}")),
                    }
                }
                Err(error) => fips_download_error = Some(format!("{error:#}")),
            }

            if fips_download_report.is_none() && !config.blossom_servers.is_empty() {
                match download_roots_over_blossom(config_dir, &config, &root_cids_to_download).await
                {
                    Ok(report) => blossom_download_report = Some(report),
                    Err(error) => blossom_download_error = Some(error.to_string()),
                }
            }
        }

        match materialize_working_dir(config_dir).await {
            Ok(report) => {
                let changed = report
                    .as_ref()
                    .is_some_and(|report| report.materialize.changed());
                materialize_report = report;
                if changed {
                    match AppConfig::load_or_default(config_path_in(config_dir)) {
                        Ok(updated_config) => {
                            if let Some(updated_state) = updated_config.account.clone() {
                                match publish_current_state(
                                    &client,
                                    config_dir,
                                    &updated_config,
                                    &updated_state,
                                    true,
                                )
                                .await
                                {
                                    Ok(report) => materialize_publish_report = Some(report),
                                    Err(error) => {
                                        materialize_publish_error = Some(format!("{error:#}"));
                                    }
                                }
                            } else {
                                materialize_publish_error =
                                    Some("missing account after materialize".to_string());
                            }
                        }
                        Err(error) => materialize_publish_error = Some(error.to_string()),
                    }
                }
            }
            Err(error) => materialize_error = Some(format!("{error:#}")),
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
                "files_root_event_seen": files_root_event_seen,
                "files_root_event_outcome": files_root_event_outcome,
                "files_root_fetch_error": files_root_fetch_error,
                "fips_download": fips_download_report.as_ref().map(download_report_json),
                "fips_download_error": fips_download_error,
                "blossom_download": blossom_download_report.as_ref().map(download_report_json),
                "blossom_download_error": blossom_download_error,
                "materialize": materialize_report.as_ref().map(materialize_report_json),
                "materialize_error": materialize_error,
                "materialize_publish": materialize_publish_report.as_ref().map(publish_state_report_json),
                "materialize_publish_error": materialize_publish_error,
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
    gateway_port: u16,
    enable_gateway: bool,
) -> Result<()> {
    use iris_drive_core::relay_sync;
    use nostr_sdk::RelayPoolNotification;
    use tokio::sync::broadcast::error::RecvError;
    use tokio::sync::mpsc;

    let _daemon_lock = DaemonProcessLock::acquire(config_dir)?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;

    // Zero-config bootstrap: if the tray app (or a future installer)
    // staged an account + working_dir but never ran an initial import,
    // do that now before we start the embedded hashtree host.
    runtime.block_on(async {
        if !key_path_in(config_dir).exists() {
            return Ok::<_, anyhow::Error>(());
        }
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
                    "drive_iris_to_url": drive_iris_to_url_for_primary_drive(daemon.config()),
                    "files_iris_to_url": drive_iris_to_url_for_primary_drive(daemon.config()),
                    "snapshot_url": drive_iris_to_snapshot_url_for_root(&report.root_cid),
                    "permalink_url": drive_iris_to_snapshot_url_for_root(&report.root_cid),
                    "working_dir": report.working_dir.display().to_string(),
                    "file_count": report.file_count,
                    "entries": report.top_level_entries,
                })
            );
        }
        Ok::<_, anyhow::Error>(())
    })?;

    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let state = config
        .account
        .clone()
        .ok_or_else(|| anyhow::anyhow!("not initialized; run `idrive init` first"))?;
    let relays = pick_relays(&config, relay_override);
    let filters =
        relay_sync::subscription_filters(&state.owner_pubkey, iris_drive_core::PRIMARY_DRIVE_ID);
    if filters.is_empty() {
        return Err(anyhow::anyhow!("no filters to subscribe to"));
    }
    let working_dir = config
        .drive(iris_drive_core::PRIMARY_DRIVE_ID)
        .and_then(|d| d.working_dir.clone());
    let embedded_hashtree =
        EmbeddedHashtreeHost::start(config_dir, &config).context("starting embedded hashtree")?;
    let embedded_hashtree_status = embedded_hashtree.status_payload();

    runtime.block_on(async {
        let mut block_config = config.clone();
        block_config.relays = relays.clone();
        let (fips_blocks, fips_block_sync_error) =
            match start_fips_block_sync(config_dir, &block_config).await {
                Ok(sync) => (Some(Arc::new(sync)), None),
                Err(error) => (None, Some(error.to_string())),
            };
        let gateway = if enable_gateway {
            let daemon = Daemon::open(config_dir).context("opening daemon for browser gateway")?;
            Some(
                GatewayServer::bind_with_tree_and_htree_daemon(
                    config_dir,
                    daemon.tree_handle(),
                    embedded_hashtree.status().base_url.clone(),
                    GatewayBind::loopback_v4(gateway_port),
                )
                    .await
                    .context("starting browser gateway")?,
            )
        } else {
            None
        };
        let gateway_status = gateway.as_ref().map(|server| {
            let port = server.local_addr().port();
            json!({
                "bind": server.local_addr().to_string(),
                "portal_url": format!("http://sites.iris.localhost:{port}/"),
                "primary_drive_url": iris_drive_core::gateway::local_drive_url(
                    port,
                    iris_drive_core::PRIMARY_DRIVE_ID,
                ),
                "hashtree_base_url": embedded_hashtree.status().base_url.clone(),
            })
        });
        let client = relay_sync::connect(&relays)
            .await
            .context("connecting to relays")?;
        let relay_statuses = relay_status_payload(&client).await;
        client
            .subscribe(filters, None)
            .await
            .context("opening subscription")?;
        let mut notifications = client.notifications();
        let mut direct_roots = DirectRootExchange::default();
        let startup_fips_block_sync_status = fips_block_sync_status(fips_blocks.as_deref()).await;

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

        let subscribed_status = json!({
                "event": "subscribed",
                "relays": relays,
                "owner_npub": account_npub(&state.owner_pubkey),
                "watch_interval_secs": watch_interval,
                "watch_debounce_ms": watch_debounce_ms,
                "working_dir": working_dir.as_ref().map(|p| p.display().to_string()),
                "relay_statuses": relay_statuses,
                "embedded_hashtree": embedded_hashtree_status,
                "browser_gateway": gateway_status,
                "fips_block_sync": startup_fips_block_sync_status,
                "fips_block_sync_error": fips_block_sync_error,
        });
        write_daemon_status(config_dir, subscribed_status.clone());
        println!("{subscribed_status}");

        let startup_config = config.clone();
        let startup_state = state.clone();
        spawn_root_apply_followup(
            client.clone(),
            config_dir.to_path_buf(),
            config.clone(),
            None,
            fips_blocks.clone(),
            true,
            "startup_materialized",
        );
        let (scan_tx, scan_rx) = mpsc::channel::<ScanPublishRequest>(8);
        let (scan_done_tx, mut scan_done_rx) = mpsc::channel::<ScanPublishResult>(8);
        spawn_scan_publish_worker(
            client.clone(),
            config_dir.to_path_buf(),
            fips_blocks.clone(),
            scan_rx,
            scan_done_tx,
        );

        // Announce the current account roster and device root once on
        // startup, and upload the initial blocks if this launch just
        // imported them. The fs-notify + periodic paths only publish on
        // change, so without this a freshly-imported device would sit
        // silent until its first edit.
        if working_dir.is_some() {
            queue_scan_publish(
                &scan_tx,
                ScanPublishRequest {
                    trigger: "startup",
                    retry_current_root: true,
                    upload_current_to_blossom: true,
                },
            );
        } else {
            spawn_initial_publish(
                client.clone(),
                config_dir.to_path_buf(),
                startup_config,
                startup_state,
            );
        }
        println!("(running — Ctrl+C to stop)");

        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        let parent_exit = parent_exit_signal();
        tokio::pin!(parent_exit);

        // Periodic fallback in addition to fs-notify (some editor
        // patterns produce events fs-notify can miss; this catches
        // drift).
        let mut watch_timer = if watch_interval > 0 && working_dir.is_some() {
            let period = std::time::Duration::from_secs(watch_interval);
            let mut interval = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            Some(interval)
        } else {
            None
        };
        let mut relay_status_timer = tokio::time::interval(std::time::Duration::from_secs(2));
        relay_status_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut direct_mesh_timer = tokio::time::interval(std::time::Duration::from_millis(100));
        direct_mesh_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        loop {
            tokio::select! {
                _ = &mut ctrl_c => {
                    println!("{}", json!({ "event": "shutdown" }));
                    break;
                }
                () = &mut parent_exit => {
                    println!("{}", json!({ "event": "shutdown", "reason": "parent_exit" }));
                    break;
                }
                recv = notifications.recv() => {
                    match recv {
                        Ok(RelayPoolNotification::Event { event, .. }) => {
                            if let Err(e) =
                                apply_one_event(&client, config_dir, &event, fips_blocks.clone()).await
                            {
                                println!(
                                    "{}",
                                    json!({"event": "apply_error", "id": event.id.to_hex(), "error": e.to_string()})
                                );
                            } else if let Err(error) =
                                announce_current_state_direct(
                                    &mut direct_roots,
                                    config_dir,
                                    fips_blocks.as_deref(),
                                )
                                .await
                            {
                                println!(
                                    "{}",
                                    json!({"event": "direct_root_mesh_error", "error": format!("{error:#}")})
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
                    queue_scan_publish(
                        &scan_tx,
                        ScanPublishRequest {
                            trigger: "fs",
                            retry_current_root: false,
                            upload_current_to_blossom: false,
                        },
                    );
                }
                Some(result) = scan_done_rx.recv() => {
                    if let Some(error) = result.error {
                        println!(
                            "{}",
                            json!({"event": "auto_publish_error", "trigger": result.trigger, "error": error})
                        );
                    } else if let Err(error) =
                        announce_current_state_direct(
                            &mut direct_roots,
                            config_dir,
                            fips_blocks.as_deref(),
                        )
                        .await
                    {
                        println!(
                            "{}",
                            json!({"event": "direct_root_mesh_error", "error": format!("{error:#}")})
                        );
                    }
                }
                _ = relay_status_timer.tick() => {
                    write_daemon_status(config_dir, json!({"event": "heartbeat"}));
                    let relay_statuses = match tokio::time::timeout(
                        std::time::Duration::from_secs(STATUS_PROBE_TIMEOUT_SECS),
                        relay_status_payload(&client),
                    )
                    .await
                    {
                        Ok(statuses) => statuses,
                        Err(_) => vec![json!({"url": "*", "status": "timeout"})],
                    };
                    let fips_status = match tokio::time::timeout(
                        std::time::Duration::from_secs(STATUS_PROBE_TIMEOUT_SECS),
                        fips_block_sync_status(fips_blocks.as_deref()),
                    )
                    .await
                    {
                        Ok(status) => status,
                        Err(_) => Some(json!({"status": "timeout"})),
                    };
                    let status = json!({
                            "event": "relay_statuses",
                            "relay_statuses": relay_statuses,
                            "fips_block_sync": fips_status,
                    });
                    write_daemon_status(config_dir, status.clone());
                    println!("{status}");
                    direct_roots
                        .request_roots_from_new_peers(config_dir, fips_blocks.as_deref())
                        .await;
                    if let Err(error) =
                        announce_current_state_direct(
                            &mut direct_roots,
                            config_dir,
                            fips_blocks.as_deref(),
                        )
                        .await
                    {
                        println!(
                            "{}",
                            json!({"event": "direct_root_mesh_error", "error": format!("{error:#}")})
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
                    queue_scan_publish(
                        &scan_tx,
                        ScanPublishRequest {
                            trigger: "timer",
                            retry_current_root: false,
                            upload_current_to_blossom: false,
                        },
                    );
                }
                _ = direct_mesh_timer.tick() => {
                    if let Some(sync) = fips_blocks.as_ref()
                        && let Err(error) = direct_roots
                            .drain_mesh_events(&client, config_dir, sync.clone())
                            .await
                    {
                        println!(
                            "{}",
                            json!({"event": "direct_root_mesh_error", "error": format!("{error:#}")})
                        );
                    }
                }
            }
        }
        let _ = client.disconnect().await;
        Ok::<_, anyhow::Error>(())
    })
}

async fn relay_status_payload(client: &nostr_sdk::Client) -> Vec<serde_json::Value> {
    let relays = client.relays().await;
    let mut payload = Vec::with_capacity(relays.len());
    for (url, relay) in relays {
        let url = normalize_relay_url(url.as_ref());
        payload.push(json!({
            "url": url,
            "status": relay_status_label(relay.status().await),
        }));
    }
    payload
}

async fn fips_block_sync_status(sync: Option<&FsFipsBlockSync>) -> Option<Value> {
    let sync = sync?;
    Some(json!({
        "endpoint_npub": sync.endpoint_npub(),
        "discovery_scope": sync.discovery_scope(),
        "authorized_peers": sync.authorized_peer_ids().await,
        "connected_peers": sync.connected_peer_ids().await,
    }))
}

fn relay_status_label(status: RelayStatus) -> &'static str {
    match status {
        RelayStatus::Initialized | RelayStatus::Pending | RelayStatus::Connecting => "connecting",
        RelayStatus::Connected => "connected",
        RelayStatus::Disconnected => "offline",
        RelayStatus::Terminated => "terminated",
    }
}

struct DaemonProcessLock {
    path: PathBuf,
}

impl DaemonProcessLock {
    fn acquire(config_dir: &std::path::Path) -> Result<Self> {
        std::fs::create_dir_all(config_dir)
            .with_context(|| format!("creating config dir {}", config_dir.display()))?;
        let path = config_dir.join("daemon.lock");
        match Self::try_create(&path) {
            Ok(lock) => return Ok(lock),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("creating daemon lock {}", path.display()));
            }
        }

        if let Ok(contents) = std::fs::read_to_string(&path)
            && let Ok(pid) = contents.trim().parse::<u32>()
            && !process_is_running(pid)
        {
            let _ = std::fs::remove_file(&path);
            return Self::try_create(&path)
                .with_context(|| format!("replacing stale daemon lock {}", path.display()));
        }

        Err(anyhow::anyhow!(
            "iris-drive daemon already appears to be running for {}",
            config_dir.display()
        ))
    }

    fn try_create(path: &Path) -> std::io::Result<Self> {
        use std::io::Write;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        writeln!(file, "{}", std::process::id())?;
        Ok(Self {
            path: path.to_path_buf(),
        })
    }
}

impl Drop for DaemonProcessLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
fn process_is_running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    std::process::Command::new("/bin/kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(windows)]
fn process_is_running(pid: u32) -> bool {
    if pid == std::process::id() {
        return true;
    }
    let filter = format!("PID eq {pid}");
    let output = std::process::Command::new("tasklist")
        .args(["/FI", &filter, "/FO", "CSV", "/NH"])
        .output();
    let Ok(output) = output else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().any(|line| {
        let mut fields = line.split(',');
        let _image = fields.next();
        fields
            .next()
            .map(|value| value.trim_matches('"').trim() == pid.to_string())
            .unwrap_or(false)
    })
}

#[cfg(not(any(unix, windows)))]
fn process_is_running(pid: u32) -> bool {
    pid == std::process::id()
}

async fn parent_exit_signal() {
    let Some(parent_pid) = std::env::var("IRIS_DRIVE_PARENT_PID")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
    else {
        std::future::pending::<()>().await;
        return;
    };

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        if !process_is_running(parent_pid) {
            return;
        }
    }
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
#[allow(clippy::too_many_lines)]
async fn scan_and_publish(
    client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    fips_blocks: Option<&FsFipsBlockSync>,
    retry_current_root: bool,
    upload_current_to_blossom: bool,
) -> Result<()> {
    use iris_drive_core::relay_sync;
    let config = AppConfig::load_or_default(config_path_in(config_dir))?;
    if let Some(sync) = fips_blocks {
        sync.refresh_authorized_peers(&config).await;
    }
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
    if working_dir_has_same_visible_files(&daemon, &working_dir, previous_root_cid.as_deref())
        .await?
    {
        if retry_current_root {
            let report = publish_current_state(
                client,
                config_dir,
                &config,
                state,
                upload_current_to_blossom,
            )
            .await?;
            println!(
                "{}",
                json!({
                    "event": "republished_current_state",
                    "published_app_keys": report.published_app_keys,
                    "app_keys_publish_error": report.app_keys_publish_error,
                    "published_drive_root": report.published_drive_root,
                    "drive_root_publish_error": report.drive_root_publish_error,
                    "published_files_root": report.published_files_root,
                    "files_root_publish_error": report.files_root_publish_error,
                    "root_cid": report.root_cid,
                    "blossom_upload_error": report.blossom_upload_error,
                    "blossom_upload": report.blossom_upload.as_ref().map(upload_report_json),
                })
            );
        }
        return Ok(());
    }

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
    let (upload_report, upload_error) = maybe_upload_root_to_blossom(
        config_dir,
        &config,
        &device,
        &new_root.root_cid,
        previous_root_cid.as_deref(),
    )
    .await?;
    if let Some(error) = &upload_error {
        println!(
            "{}",
            json!({"event": "blossom_upload_error", "error": error})
        );
    }

    let mut published_drive_root = false;
    match relay_publish_with_timeout(relay_sync::publish_drive_root(
        client,
        device.keys(),
        &state.owner_pubkey,
        iris_drive_core::PRIMARY_DRIVE_ID,
        &new_root,
        &authorized_device_pubkeys(state),
    ))
    .await
    {
        Ok(_) => published_drive_root = true,
        Err(e) => println!(
            "{}",
            json!({"event": "drive_root_publish_error", "error": e})
        ),
    }

    let mut published_files_root = false;
    if state.has_owner_signing_authority {
        let account = Account::load(state.clone(), config_dir).context("loading account")?;
        let owner_keys = account
            .owner_key
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("owner key missing on disk"))?
            .keys();
        match relay_publish_with_timeout(relay_sync::publish_files_root(
            client,
            owner_keys,
            iris_drive_core::PRIMARY_DRIVE_ID,
            &new_root,
        ))
        .await
        {
            Ok(_) => published_files_root = true,
            Err(e) => println!(
                "{}",
                json!({"event": "files_root_publish_error", "error": e})
            ),
        }
    }

    println!(
        "{}",
        json!({
            "event": "auto_published",
            "root_cid": report.root_cid,
            "drive_iris_to_url": drive_iris_to_url_for_primary_drive(daemon.config()),
            "files_iris_to_url": drive_iris_to_url_for_primary_drive(daemon.config()),
            "snapshot_url": drive_iris_to_snapshot_url_for_root(&report.root_cid),
            "permalink_url": drive_iris_to_snapshot_url_for_root(&report.root_cid),
            "published_drive_root": published_drive_root,
            "published_files_root": published_files_root,
            "file_count": report.file_count,
            "top_level_entries": report.top_level_entries,
            "blossom_upload_error": upload_error,
            "blossom_upload": upload_report.map(|r| json!({
                "total_hashes": r.total_hashes,
                "uploaded": r.uploaded,
                "already_present": r.already_present,
            })),
        })
    );
    Ok(())
}

async fn working_dir_has_same_visible_files(
    daemon: &Daemon,
    working_dir: &std::path::Path,
    previous_root_cid: Option<&str>,
) -> Result<bool> {
    let Some(previous_root_cid) = previous_root_cid else {
        return Ok(false);
    };
    let previous_root = Cid::parse(previous_root_cid)
        .with_context(|| format!("parsing previous root cid {previous_root_cid}"))?;
    let current_root = index_dir(daemon.tree(), working_dir)
        .await
        .context("indexing working dir for change detection")?;
    let (mut previous_files, _) = walk_device_tree(daemon.tree(), &previous_root)
        .await
        .with_context(|| format!("walking previous root {previous_root_cid}"))?;
    let (mut current_files, _) = walk_device_tree(daemon.tree(), &current_root)
        .await
        .context("walking current working dir root")?;
    sort_device_files(&mut previous_files);
    sort_device_files(&mut current_files);
    Ok(previous_files == current_files)
}

fn sort_device_files(files: &mut [DeviceFileEntry]) {
    files.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.hash.cmp(&b.hash))
            .then_with(|| a.size.cmp(&b.size))
            .then_with(|| a.whole_file_hash.cmp(&b.whole_file_hash))
    });
}

async fn apply_one_event(
    client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    event: &nostr_sdk::Event,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
) -> Result<()> {
    use iris_drive_core::relay_sync;
    let mut config = AppConfig::load_or_default(config_path_in(config_dir))?;
    let kind = event.kind.as_u16();
    if kind == iris_drive_core::nostr_events::KIND_APP_KEYS
        && event.identifier() == Some(iris_drive_core::nostr_events::D_TAG_APP_KEYS)
    {
        let outcome = relay_sync::apply_remote_app_keys_event(&mut config, event)?;
        println!(
            "{}",
            json!({
                "event": "app_keys",
                "event_id": event.id.to_hex(),
                "outcome": format!("{outcome:?}"),
            })
        );
        if let Some(sync) = fips_blocks.as_deref() {
            sync.refresh_authorized_peers(&config).await;
        }
    } else if kind == iris_drive_core::nostr_events::KIND_HASHTREE_ROOT {
        let Some(account_state) = config.account.clone() else {
            return Ok(());
        };
        return apply_files_root_event(
            client,
            config_dir,
            event,
            fips_blocks,
            &mut config,
            account_state,
        )
        .await;
    } else if kind == iris_drive_core::nostr_events::KIND_DRIVE_ROOT {
        let device = iris_drive_core::identity::DeviceIdentity::load(key_path_in(config_dir))
            .context("loading device key")?;
        let parsed =
            iris_drive_core::nostr_events::parse_drive_root_event_for_device(event, device.keys())
                .ok();
        let outcome =
            relay_sync::apply_remote_drive_root_event(&mut config, event, Some(device.keys()))?;
        let was_applied = matches!(outcome, relay_sync::DriveRootApply::Applied);
        let stale_current_root = matches!(outcome, relay_sync::DriveRootApply::StaleTimestamp)
            && parsed
                .as_ref()
                .is_some_and(|(device_pubkey, _, drive_id, root_ref)| {
                    config
                        .drive(drive_id)
                        .and_then(|drive| drive.device_roots.get(device_pubkey))
                        .is_some_and(|stored| stored.root_cid == root_ref.root_cid)
                });
        let root_cid_to_pull = parsed
            .as_ref()
            .filter(|_| was_applied || stale_current_root)
            .map(|(_, _, _, root_ref)| root_ref.root_cid.clone());
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
        if let Some(sync) = fips_blocks.as_deref() {
            sync.refresh_authorized_peers(&config).await;
        }

        spawn_root_apply_followup(
            client.clone(),
            config_dir.to_path_buf(),
            config.clone(),
            root_cid_to_pull,
            fips_blocks,
            was_applied || stale_current_root,
            "materialized_drive_root",
        );
        return Ok(());
    } else {
        // Unknown kind; ignore.
        return Ok(());
    }
    config.save(config_path_in(config_dir))?;
    Ok(())
}

async fn apply_files_root_event(
    client: &nostr_sdk::Client,
    config_dir: &std::path::Path,
    event: &nostr_sdk::Event,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    config: &mut AppConfig,
    account_state: AccountState,
) -> Result<()> {
    use iris_drive_core::relay_sync;
    if !account_state.has_owner_signing_authority {
        println!(
            "{}",
            json!({
                "event": "files_root",
                "event_id": event.id.to_hex(),
                "author": account_npub(&event.pubkey.to_hex()),
                "outcome": "owner_key_unavailable",
            })
        );
        return Ok(());
    }
    let account = Account::load(account_state, config_dir).context("loading owner account")?;
    let owner_keys = account
        .owner_key
        .as_ref()
        .map(iris_drive_core::OwnerKey::keys);
    let outcome = relay_sync::apply_remote_files_root_event(config, event, owner_keys)?;
    let was_applied = matches!(outcome, relay_sync::FilesRootApply::Applied);
    let root_cid_to_pull = if was_applied {
        config
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .and_then(|drive| drive.device_roots.get(&account.state.device_pubkey))
            .map(|root| root.root_cid.clone())
    } else {
        None
    };
    println!(
        "{}",
        json!({
            "event": "files_root",
            "event_id": event.id.to_hex(),
            "author": account_npub(&event.pubkey.to_hex()),
            "outcome": files_root_apply_label(&outcome),
        })
    );
    config.save(config_path_in(config_dir))?;
    spawn_root_apply_followup(
        client.clone(),
        config_dir.to_path_buf(),
        config.clone(),
        root_cid_to_pull,
        fips_blocks,
        was_applied,
        "materialized_files_root",
    );
    Ok(())
}

fn spawn_root_apply_followup(
    client: nostr_sdk::Client,
    config_dir: PathBuf,
    config: AppConfig,
    root_cid_to_pull: Option<String>,
    fips_blocks: Option<Arc<FsFipsBlockSync>>,
    should_materialize: bool,
    materialize_event: &'static str,
) {
    if root_cid_to_pull.is_none() && !should_materialize {
        return;
    }

    tokio::spawn(async move {
        if let Some(root_cid) = root_cid_to_pull
            && let Err(error) = pull_blocks_for_root_bounded(
                &config_dir,
                &config,
                &root_cid,
                fips_blocks.as_deref(),
            )
            .await
        {
            println!(
                "{}",
                json!({"event": "block_download_error", "error": error})
            );
        }

        if should_materialize {
            match tokio::time::timeout(
                std::time::Duration::from_secs(EVENT_MATERIALIZE_TIMEOUT_SECS),
                materialize_and_publish(&client, &config_dir, materialize_event),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(error)) => println!(
                    "{}",
                    json!({"event": "materialize_error", "error": format!("{error:#}")})
                ),
                Err(_) => println!(
                    "{}",
                    json!({
                        "event": "materialize_error",
                        "error": format!("timed out after {EVENT_MATERIALIZE_TIMEOUT_SECS}s"),
                    })
                ),
            }
        }
    });
}

async fn pull_blocks_for_root(
    config_dir: &std::path::Path,
    config: &AppConfig,
    root_cid_str: &str,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> Result<()> {
    let cid =
        Cid::parse(root_cid_str).with_context(|| format!("parsing root cid {root_cid_str}"))?;
    let mut attempted = false;
    let mut errors = Vec::new();
    if let Some(sync) = fips_blocks {
        let connected_peers = sync.connected_peer_ids().await;
        if connected_peers.is_empty() {
            println!(
                "{}",
                json!({
                    "event": "fips_download_skipped",
                    "root_cid": root_cid_str,
                    "reason": "no_connected_peers",
                })
            );
        } else {
            attempted = true;
            match download_tree_over_fips_with_retry(sync, &cid, fips_download_policy(config)).await
            {
                Ok(report) => {
                    record_block_sync(config_dir, root_cid_str, "fips", &report);
                    println!(
                        "{}",
                        json!({
                            "event": "fips_downloaded",
                            "root_cid": root_cid_str,
                            "report": download_report_json(&report),
                        })
                    );
                    return Ok(());
                }
                Err(error) => {
                    let error = format!("{error:#}");
                    errors.push(format!("fips: {error}"));
                    println!(
                        "{}",
                        json!({
                            "event": "fips_download_error",
                            "root_cid": root_cid_str,
                            "error": error,
                        })
                    );
                }
            }
        }
    }

    if !config.blossom_servers.is_empty() {
        attempted = true;
        match download_roots_over_blossom(config_dir, config, &[root_cid_str.to_string()]).await {
            Ok(report) => {
                record_block_sync(config_dir, root_cid_str, "blossom", &report);
                println!(
                    "{}",
                    json!({
                        "event": "blossom_downloaded",
                        "root_cid": root_cid_str,
                        "report": download_report_json(&report),
                    })
                );
                return Ok(());
            }
            Err(error) => {
                let error = error.to_string();
                errors.push(format!("blossom: {error}"));
                println!(
                    "{}",
                    json!({
                        "event": "blossom_download_error",
                        "root_cid": root_cid_str,
                        "error": error,
                    })
                );
            }
        }
    }

    if attempted {
        Err(anyhow::anyhow!(
            "all block download transports failed for {root_cid_str}: {}",
            errors.join("; ")
        ))
    } else {
        Err(anyhow::anyhow!(
            "no block download transport available for {root_cid_str}"
        ))
    }
}

async fn pull_blocks_for_root_bounded(
    config_dir: &std::path::Path,
    config: &AppConfig,
    root_cid_str: &str,
    fips_blocks: Option<&FsFipsBlockSync>,
) -> std::result::Result<(), String> {
    match tokio::time::timeout(
        std::time::Duration::from_secs(EVENT_BLOCK_PULL_TIMEOUT_SECS),
        pull_blocks_for_root(config_dir, config, root_cid_str, fips_blocks),
    )
    .await
    {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err(format!("timed out after {EVENT_BLOCK_PULL_TIMEOUT_SECS}s")),
    }
}

fn record_block_sync(config_dir: &Path, root_cid: &str, transport: &str, report: &DownloadReport) {
    let value = json!({
        "root_cid": root_cid,
        "transport": transport,
        "updated_at": unix_now(),
        "total_hashes": report.total_hashes,
        "fetched": report.fetched,
        "already_local": report.already_local,
    });
    merge_daemon_status(config_dir, |status| {
        status.insert("last_block_sync".to_string(), value.clone());
        let entry = status
            .entry("block_sync_by_root".to_string())
            .or_insert_with(|| json!({}));
        if !entry.is_object() {
            *entry = json!({});
        }
        if let Some(map) = entry.as_object_mut() {
            map.insert(root_cid.to_string(), value);
        }
    });
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

fn files_root_apply_label(outcome: &iris_drive_core::relay_sync::FilesRootApply) -> &'static str {
    match outcome {
        iris_drive_core::relay_sync::FilesRootApply::NotOurOwner => "not_our_owner",
        iris_drive_core::relay_sync::FilesRootApply::UnknownDrive => "unknown_drive",
        iris_drive_core::relay_sync::FilesRootApply::StaleTimestamp => "stale_timestamp",
        iris_drive_core::relay_sync::FilesRootApply::Applied => "applied",
    }
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct DeviceApprovalRequest {
    owner_hex: String,
    device_hex: String,
    label: Option<String>,
}

fn resolve_device_approval_input(
    input: &str,
    expected_owner_hex: &str,
    explicit_label: Option<String>,
) -> Result<(String, Option<String>)> {
    if let Some(request) = decode_device_approval_request(input)? {
        if request.owner_hex != expected_owner_hex {
            return Err(anyhow::anyhow!(
                "device request belongs to a different owner"
            ));
        }
        let label = explicit_label.or(request.label);
        return Ok((request.device_hex, label));
    }

    Ok((
        normalize_pubkey(input).context("parsing device pubkey")?,
        explicit_label,
    ))
}

fn device_link_request_json(state: &AccountState) -> Value {
    if state.has_owner_signing_authority
        || state.authorization_state != iris_drive_core::DeviceAuthorizationState::AwaitingApproval
    {
        return Value::Null;
    }

    let url = encode_device_approval_request(
        &state.owner_pubkey,
        &state.device_pubkey,
        state.device_label.as_deref(),
    );

    json!({
        "url": url,
        "owner_npub": account_npub(&state.owner_pubkey),
        "device_npub": account_npub(&state.device_pubkey),
        "label": state.device_label.as_deref(),
    })
}

fn encode_device_approval_request(
    owner_hex: &str,
    device_hex: &str,
    label: Option<&str>,
) -> String {
    let mut url = format!(
        "iris-drive://device-link?owner={}&device={}",
        account_npub(owner_hex),
        account_npub(device_hex)
    );
    if let Some(label) = label.map(str::trim).filter(|label| !label.is_empty()) {
        url.push_str("&label=");
        url.push_str(&percent_encode_component(label));
    }
    url
}

fn decode_device_approval_request(input: &str) -> Result<Option<DeviceApprovalRequest>> {
    let trimmed = input.trim();
    let Some(query) = device_approval_query(trimmed) else {
        return Ok(None);
    };

    let mut owner = None;
    let mut device = None;
    let mut label = None;
    for pair in query.split('&') {
        if pair.is_empty() {
            continue;
        }
        let (raw_key, raw_value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode_component(raw_key)?;
        let value = percent_decode_component(raw_value)?;
        match key.as_str() {
            "owner" if !value.trim().is_empty() => owner = Some(value),
            "device" if !value.trim().is_empty() => device = Some(value),
            "label" if !value.trim().is_empty() => label = Some(value),
            _ => {}
        }
    }

    let owner = owner.ok_or_else(|| anyhow::anyhow!("device request is missing owner"))?;
    let device = device.ok_or_else(|| anyhow::anyhow!("device request is missing device"))?;

    Ok(Some(DeviceApprovalRequest {
        owner_hex: normalize_pubkey(&owner).context("parsing request owner")?,
        device_hex: normalize_pubkey(&device).context("parsing request device")?,
        label,
    }))
}

fn device_approval_query(input: &str) -> Option<&str> {
    if let Some(rest) = input.strip_prefix("iris-drive://device-link") {
        return rest.strip_prefix('?');
    }
    if let Some(rest) = input.strip_prefix("https://drive.iris.to/device-link") {
        return rest.strip_prefix('?');
    }
    None
}

fn percent_encode_component(input: &str) -> String {
    let mut encoded = String::with_capacity(input.len());
    for byte in input.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(hex_digit(byte >> 4));
            encoded.push(hex_digit(byte & 0x0f));
        }
    }
    encoded
}

fn percent_decode_component(input: &str) -> Result<String> {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hi = bytes
                .get(index + 1)
                .copied()
                .and_then(hex_value)
                .ok_or_else(|| anyhow::anyhow!("invalid percent encoding"))?;
            let lo = bytes
                .get(index + 2)
                .copied()
                .and_then(hex_value)
                .ok_or_else(|| anyhow::anyhow!("invalid percent encoding"))?;
            output.push((hi << 4) | lo);
            index += 3;
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }

    String::from_utf8(output).context("request contains invalid UTF-8")
}

fn hex_digit(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'A' + value - 10) as char,
        _ => '0',
    }
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn account_npub(hex: &str) -> String {
    use nostr_sdk::nips::nip19::ToBech32;
    PublicKey::from_hex(hex)
        .ok()
        .and_then(|pk| pk.to_bech32().ok())
        .unwrap_or_else(|| hex.to_string())
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

#[cfg(test)]
mod daemon_lock_tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn daemon_lock_allows_only_one_process_per_config_dir() {
        let dir = tempdir().unwrap();
        let first = DaemonProcessLock::acquire(dir.path()).unwrap();
        assert!(DaemonProcessLock::acquire(dir.path()).is_err());
        drop(first);
        assert!(DaemonProcessLock::acquire(dir.path()).is_ok());
    }

    #[test]
    fn daemon_lock_replaces_stale_pid_file() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("daemon.lock"), "99999999\n").unwrap();
        assert!(DaemonProcessLock::acquire(dir.path()).is_ok());
    }

    #[test]
    fn conflict_status_counts_unresolved_records() {
        let records = vec![
            iris_drive_core::ConflictRecord {
                schema: iris_drive_core::ConflictRecord::SCHEMA,
                conflict_id: "unresolved-a".into(),
                path: "report.pdf".into(),
                visible_conflict_path: "report (conflict from phone).pdf".into(),
                local: iris_drive_core::ConflictSide {
                    device_id: "laptop".into(),
                    device_seq: 4,
                    root_cid: "cid-local".into(),
                    whole_file_hash: "hash-local".into(),
                },
                remote: Some(iris_drive_core::ConflictSide {
                    device_id: "phone".into(),
                    device_seq: 9,
                    root_cid: "cid-remote".into(),
                    whole_file_hash: "hash-remote".into(),
                }),
                deleted: None,
                state: iris_drive_core::ConflictState::Unresolved,
                created_at: 1234,
            },
            iris_drive_core::ConflictRecord {
                schema: iris_drive_core::ConflictRecord::SCHEMA,
                conflict_id: "resolved-b".into(),
                path: "notes.txt".into(),
                visible_conflict_path: "notes (conflict from tablet).txt".into(),
                local: iris_drive_core::ConflictSide {
                    device_id: "laptop".into(),
                    device_seq: 5,
                    root_cid: "cid-local-2".into(),
                    whole_file_hash: "hash-local-2".into(),
                },
                remote: None,
                deleted: Some(iris_drive_core::ConflictDeletedSide {
                    device_id: "tablet".into(),
                    device_seq: 2,
                    root_cid: "cid-delete".into(),
                    tombstoned_at: 1200,
                }),
                state: iris_drive_core::ConflictState::Resolved,
                created_at: 1201,
            },
        ];

        let status = conflict_status_payload(&records);

        assert_eq!(status["total_count"], 2);
        assert_eq!(status["unresolved_count"], 1);
        assert_eq!(status["resolved_count"], 1);
        assert_eq!(status["overflow_count"], 0);
        assert_eq!(status["unresolved"][0]["conflict_id"], "unresolved-a");
        assert_eq!(status["unresolved"][0]["path"], "report.pdf");
        assert_eq!(
            status["unresolved"][0]["visible_conflict_path"],
            "report (conflict from phone).pdf"
        );
    }

    #[tokio::test]
    async fn working_dir_change_detection_ignores_snapshot_metadata() {
        let cfg_dir = tempdir().unwrap();
        let work = tempdir().unwrap();
        cmd_init(cfg_dir.path(), false, Some("test-device".into())).unwrap();

        std::fs::write(work.path().join("note.txt"), b"one").unwrap();
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        let first = daemon.import_working_dir(work.path()).await.unwrap();
        assert!(
            working_dir_has_same_visible_files(&daemon, work.path(), Some(&first.root_cid))
                .await
                .unwrap()
        );

        let second = daemon.import_working_dir(work.path()).await.unwrap();
        assert_ne!(
            first.root_cid, second.root_cid,
            "history metadata still creates a new root when explicitly imported"
        );
        assert!(
            working_dir_has_same_visible_files(&daemon, work.path(), Some(&second.root_cid))
                .await
                .unwrap()
        );

        std::fs::write(work.path().join("note.txt"), b"two").unwrap();
        assert!(
            !working_dir_has_same_visible_files(&daemon, work.path(), Some(&second.root_cid))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn working_dir_change_detection_detects_deletions() {
        let cfg_dir = tempdir().unwrap();
        let work = tempdir().unwrap();
        cmd_init(cfg_dir.path(), false, Some("test-device".into())).unwrap();

        std::fs::write(work.path().join("keep.txt"), b"keep").unwrap();
        std::fs::write(work.path().join("delete-me.txt"), b"delete").unwrap();
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        let first = daemon.import_working_dir(work.path()).await.unwrap();
        std::fs::remove_file(work.path().join("delete-me.txt")).unwrap();

        assert!(
            !working_dir_has_same_visible_files(&daemon, work.path(), Some(&first.root_cid))
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn relay_publish_timeout_returns_control_to_daemon_loop() {
        let error =
            relay_publish_with_timeout_duration(
                std::time::Duration::from_millis(1),
                std::future::pending::<
                    std::result::Result<(), iris_drive_core::relay_sync::RelayError>,
                >(),
            )
            .await
            .unwrap_err();

        assert!(error.starts_with("timed out after"));
    }

    #[tokio::test]
    async fn direct_mesh_sync_events_skip_materialized_only_roots() {
        let cfg_dir = tempdir().unwrap();
        let work = tempdir().unwrap();
        cmd_init(cfg_dir.path(), false, Some("test-device".into())).unwrap();

        std::fs::write(work.path().join("from-peer.txt"), b"materialized copy").unwrap();
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        daemon.import_working_dir(work.path()).await.unwrap();

        let mut config = AppConfig::load_or_default(config_path_in(cfg_dir.path())).unwrap();
        let state = config.account.clone().unwrap();
        let mut drive = config
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .unwrap()
            .clone();
        drive
            .device_roots
            .get_mut(&state.device_pubkey)
            .unwrap()
            .materialized_only = true;
        config.upsert_drive(drive);
        config.save(config_path_in(cfg_dir.path())).unwrap();

        let events = build_current_sync_events(cfg_dir.path(), &config, &state)
            .await
            .unwrap();

        assert!(
            events
                .iter()
                .all(|event| !event.key.starts_with("drive-root:")
                    && !event.key.starts_with("files-root:")),
            "materialized-only roots must not be announced as local edits: {events:#?}"
        );
    }

    #[tokio::test]
    async fn direct_mesh_sync_events_reannounce_publishable_parent_root() {
        let cfg_dir = tempdir().unwrap();
        let work = tempdir().unwrap();
        cmd_init(cfg_dir.path(), false, Some("test-device".into())).unwrap();

        std::fs::write(work.path().join("mine.txt"), b"local edit").unwrap();
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        let first = daemon.import_working_dir(work.path()).await.unwrap();
        let first_root = daemon
            .config()
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .unwrap()
            .device_roots
            .get(&daemon.config().account.as_ref().unwrap().device_pubkey)
            .unwrap()
            .clone();

        std::fs::write(work.path().join("peer.txt"), b"materialized peer").unwrap();
        let account = daemon.config().account.as_ref().unwrap().clone();
        let first_cid = Cid::parse(&first.root_cid).unwrap();
        let second_time = first_root.published_at + 1;
        let meta = iris_drive_core::DriveRootMeta {
            schema: iris_drive_core::DriveRootMeta::SCHEMA,
            drive_id: iris_drive_core::PRIMARY_DRIVE_ID.to_string(),
            device_id: account.device_pubkey.clone(),
            device_seq: first_root.device_seq + 1,
            dck_generation: first_root.dck_generation,
            materialized_only: true,
            parents: vec![iris_drive_core::RootParent {
                device_id: account.device_pubkey.clone(),
                device_seq: first_root.device_seq,
                root_cid: first.root_cid.clone(),
            }],
            observed: BTreeMap::new(),
            created_at: second_time,
        };
        let second = iris_drive_core::indexer::index_dir_with_history_and_meta(
            daemon.tree(),
            work.path(),
            Some(&first_cid),
            second_time,
            Some(&meta),
        )
        .await
        .unwrap();
        let mut config = AppConfig::load_or_default(config_path_in(cfg_dir.path())).unwrap();
        let mut drive = config
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .unwrap()
            .clone();
        drive.device_roots.insert(
            account.device_pubkey.clone(),
            DeviceRootRef::from_meta(second.to_string(), second_time, &meta),
        );
        config.upsert_drive(drive);
        config.save(config_path_in(cfg_dir.path())).unwrap();

        let events = build_current_sync_events(cfg_dir.path(), &config, &account)
            .await
            .unwrap();

        assert!(
            events
                .iter()
                .any(|event| event.key.starts_with("drive-root:")
                    && event.key.contains(&first.root_cid)),
            "last real local root should still be re-announced: {events:#?}"
        );
        assert!(
            events
                .iter()
                .all(|event| !event.key.contains(&second.to_string())),
            "materialized parent root must stay local-only: {events:#?}"
        );
    }

    #[test]
    fn conflict_status_reports_per_path_overflow() {
        let records: Vec<_> = (0..=CONFLICT_STATUS_PATH_CAP)
            .map(|index| iris_drive_core::ConflictRecord {
                schema: iris_drive_core::ConflictRecord::SCHEMA,
                conflict_id: format!("conflict-{index}"),
                path: "report.pdf".into(),
                visible_conflict_path: format!("report (conflict from phone {index}).pdf"),
                local: iris_drive_core::ConflictSide {
                    device_id: "laptop".into(),
                    device_seq: 4,
                    root_cid: "cid-local".into(),
                    whole_file_hash: format!("hash-local-{index}"),
                },
                remote: Some(iris_drive_core::ConflictSide {
                    device_id: "phone".into(),
                    device_seq: index as u64,
                    root_cid: "cid-remote".into(),
                    whole_file_hash: format!("hash-remote-{index}"),
                }),
                deleted: None,
                state: iris_drive_core::ConflictState::Unresolved,
                created_at: 1234,
            })
            .collect();

        let status = conflict_status_payload(&records);

        assert_eq!(status["unresolved_count"], CONFLICT_STATUS_PATH_CAP + 1);
        assert_eq!(status["overflow_count"], 1);
        assert_eq!(status["overflow_paths"][0]["path"], "report.pdf");
        assert_eq!(
            status["overflow_paths"][0]["unresolved_count"],
            CONFLICT_STATUS_PATH_CAP + 1
        );
        assert_eq!(status["overflow_paths"][0]["cap"], CONFLICT_STATUS_PATH_CAP);
    }
}
