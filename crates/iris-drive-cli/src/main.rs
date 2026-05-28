use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hashtree_core::{
    Cid, HashTree, HashTreeConfig, LinkType, MemoryStore, NHashData, Store, nhash_encode_full,
    to_hex,
};
use hashtree_fs::FsBlobStore;
use hashtree_lmdb::LmdbBlobStore;
use hashtree_provider::{HashTreeProviderFs, ItemKind, ProviderFs};
use iris_drive_core::{
    AccountState, BackupTarget, BackupTargetCheck, BackupTargetKind, BackupTargetSync,
    DeviceRootRef, Drive, DriveRole, FsFipsBlockSync, PRIMARY_DRIVE_ID, UserProfile,
    account::Account,
    blossom_sync::{DownloadReport, UploadReport},
    config::AppConfig,
    daemon::{Daemon, EmbeddedHashtreeHost},
    gateway::{GatewayBind, GatewayServer},
    index_dir,
    merge::{DeviceFileEntry, DeviceSnapshot, DeviceTombstone, merge_drives},
    paths::{config_path_in, default_config_dir, default_mountpoint_in, key_path_in},
};
use nostr_sdk::nips::nip19::FromBech32;
use nostr_sdk::{Event, JsonUtil, PublicKey, RelayStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

mod account;
mod backups;
mod commands;
mod daemon;
mod device_link;
mod drive;
mod mount;
mod publish;
mod stats;
mod status;
mod sync;

#[allow(clippy::wildcard_imports)]
use account::*;
#[allow(clippy::wildcard_imports)]
use backups::*;
#[allow(clippy::wildcard_imports)]
use commands::*;
#[allow(clippy::wildcard_imports)]
use daemon::*;
#[allow(clippy::wildcard_imports)]
use device_link::*;
#[allow(clippy::wildcard_imports)]
use drive::*;
#[allow(clippy::wildcard_imports)]
use publish::*;
#[allow(clippy::wildcard_imports)]
use stats::*;
#[allow(clippy::wildcard_imports)]
use status::*;
#[allow(clippy::wildcard_imports)]
use sync::*;

const DEFAULT_GATEWAY_PORT: u16 = 17_321;
const CONFLICT_STATUS_PATH_CAP: usize = 32;
const FIPS_DOWNLOAD_RETRY_DELAYS: &[u64] = &[1, 2, 4];
const FIPS_DOWNLOAD_BEFORE_BLOSSOM_RETRY_DELAYS: &[u64] = &[];
const FIPS_DOWNLOAD_ATTEMPT_TIMEOUT_SECS: u64 = 8;
const FIPS_DOWNLOAD_BEFORE_BLOSSOM_ATTEMPT_TIMEOUT_SECS: u64 = 3;
const STARTUP_NETWORK_TIMEOUT_SECS: u64 = 20;
const EVENT_BLOCK_PULL_TIMEOUT_SECS: u64 = 3;
const EVENT_BLOCK_PULL_RETRY_DELAYS: &[u64] = &[0, 1, 2];
const EVENT_BLOCK_PULL_WITH_BLOSSOM_HEADROOM_SECS: u64 = 5;
const EVENT_BLOCK_PULL_WITH_BLOSSOM_RETRY_DELAYS: &[u64] = &[0];
const RELAY_PUBLISH_TIMEOUT_SECS: u64 = 10;
const STATUS_PROBE_TIMEOUT_SECS: u64 = 2;
const BLOSSOM_DOWNLOAD_RETRY_DELAYS: &[u64] = &[1, 2, 4];
const BLOSSOM_UPLOAD_TIMEOUT_SECS: u64 = 10;
const ROOT_UPDATE_THROTTLE_MS: u64 = 150;
const DIRECT_ROOT_MESH_STREAM_PREFIX: &str = "iris-drive/root-events/v1";
const DIRECT_ROOT_EVENT_CACHE_CAP: usize = 128;
const DIRECT_ROOT_REPUBLISH_INTERVAL_SECS: u64 = 30;
const LOCAL_ROOT_AVAILABILITY_RETRY_DELAYS_MS: &[u64] = &[250, 500, 1_000, 2_000, 4_000, 8_000];

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
        Command::Version { json } => {
            cmd_version(json);
            Ok(())
        }
        Command::InstallCli { path, force } => cmd_install_cli(path, force),
        Command::UninstallCli { path } => cmd_uninstall_cli(path),
        Command::Init {
            force,
            label,
            username,
            profile_photo,
        } => cmd_init(
            &config_dir,
            force,
            label,
            username.as_deref(),
            profile_photo.as_deref(),
        ),
        Command::Restore { nsec, label } => cmd_restore(&config_dir, &nsec, label),
        Command::Link { owner, label } => cmd_link(&config_dir, &owner, label),
        Command::Approve { device, label } => cmd_approve(&config_dir, &device, label),
        Command::Revoke { device } => cmd_revoke(&config_dir, &device),
        Command::Roster => cmd_roster(&config_dir),
        Command::RotateDck => cmd_rotate_dck(&config_dir),
        Command::Status => cmd_status(&config_dir),
        Command::Stats => cmd_stats(&config_dir),
        Command::Devices(command) => cmd_devices(&config_dir, command),
        Command::NhashResolver { command } => cmd_nhash_resolver(&config_dir, command),
        Command::Conflicts(command) => cmd_conflicts(&config_dir, command),
        Command::Drives => cmd_drives(&config_dir),
        Command::Whoami => cmd_whoami(&config_dir),
        Command::Index { dir } => cmd_index(&dir),
        Command::Import { dir } => cmd_import(&config_dir, &dir),
        Command::Materialize { dir } => cmd_materialize(&config_dir, &dir),
        Command::List { at } => cmd_list(&config_dir, at),
        Command::Provider(command) => cmd_provider(&config_dir, command),
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
            mount,
            mountpoint,
        } => cmd_daemon(
            &config_dir,
            &relay,
            watch_interval,
            watch_debounce_ms,
            gateway_port,
            !no_gateway,
            mount,
            mountpoint,
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

fn cmd_version(json_output: bool) {
    if json_output {
        println!(
            "{}",
            json!({
                "version": env!("CARGO_PKG_VERSION"),
            })
        );
    } else {
        println!("{}", env!("CARGO_PKG_VERSION"));
    }
}

fn cmd_install_cli(path: Option<PathBuf>, force: bool) -> Result<()> {
    let path = path.unwrap_or_else(default_cli_install_path);
    if path.as_os_str().is_empty() {
        return Err(anyhow::anyhow!("install path must not be empty"));
    }
    if path.is_dir() {
        return Err(anyhow::anyhow!(
            "install path points to a directory: {}",
            path.display()
        ));
    }

    let current_exe = std::env::current_exe().context("locating current idrive executable")?;
    let current_exe = std::fs::canonicalize(&current_exe)
        .with_context(|| format!("canonicalizing {}", current_exe.display()))?;
    if let Ok(existing) = std::fs::canonicalize(&path)
        && existing == current_exe
    {
        println!(
            "{}",
            json!({
                "installed": true,
                "path": path.display().to_string(),
                "already_current": true,
            })
        );
        return Ok(());
    }

    if path.exists() && !force {
        return Err(anyhow::anyhow!("{} already exists", path.display()));
    }
    if path.exists() {
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("inspecting {}", path.display()))?;
        if metadata.file_type().is_dir() {
            return Err(anyhow::anyhow!(
                "refusing to overwrite directory {}",
                path.display()
            ));
        }
        std::fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
    }

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating install directory {}", parent.display()))?;
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos());
    let temp_path = parent.join(format!(".idrive-install-{}-{nonce}", std::process::id()));
    if temp_path.exists() {
        let _ = std::fs::remove_file(&temp_path);
    }

    std::fs::copy(&current_exe, &temp_path).with_context(|| {
        format!(
            "copying {} to {}",
            current_exe.display(),
            temp_path.display()
        )
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(&temp_path)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&temp_path, permissions)
            .with_context(|| format!("marking {} executable", temp_path.display()))?;
    }
    std::fs::rename(&temp_path, &path)
        .with_context(|| format!("moving {} into {}", temp_path.display(), path.display()))?;
    println!(
        "{}",
        json!({
            "installed": true,
            "path": path.display().to_string(),
        })
    );
    Ok(())
}

fn cmd_uninstall_cli(path: Option<PathBuf>) -> Result<()> {
    let path = path.unwrap_or_else(default_cli_install_path);
    match std::fs::remove_file(&path) {
        Ok(()) => {
            println!(
                "{}",
                json!({
                    "uninstalled": true,
                    "path": path.display().to_string(),
                })
            );
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            println!(
                "{}",
                json!({
                    "uninstalled": false,
                    "path": path.display().to_string(),
                    "not_found": true,
                })
            );
            Ok(())
        }
        Err(error) => Err(error).with_context(|| format!("removing {}", path.display())),
    }
}

fn default_cli_install_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        if let Some(dir) = default_windows_cli_install_dir() {
            return dir.join("idrive.exe");
        }
        return PathBuf::from("idrive.exe");
    }

    #[cfg(not(target_os = "windows"))]
    {
        PathBuf::from("/usr/local/bin/idrive")
    }
}

#[cfg(target_os = "windows")]
fn default_windows_cli_install_dir() -> Option<PathBuf> {
    let home = dirs::home_dir();
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            if dir.as_os_str().is_empty() {
                continue;
            }
            if home.as_ref().is_some_and(|home| dir.starts_with(home)) {
                return Some(dir);
            }
        }
    }
    home.map(|home| home.join(".cargo").join("bin"))
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
        cmd_init(
            cfg_dir.path(),
            false,
            Some("test-device".into()),
            None,
            None,
        )
        .unwrap();

        std::fs::write(work.path().join("from-peer.txt"), b"materialized copy").unwrap();
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        daemon.import_source_dir(work.path()).await.unwrap();

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
        cmd_init(
            cfg_dir.path(),
            false,
            Some("test-device".into()),
            None,
            None,
        )
        .unwrap();

        std::fs::write(work.path().join("mine.txt"), b"local edit").unwrap();
        let mut daemon = Daemon::open(cfg_dir.path()).unwrap();
        let first = daemon.import_source_dir(work.path()).await.unwrap();
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

    #[tokio::test]
    async fn direct_mesh_sync_events_refuse_unreadable_local_root() {
        let cfg_dir = tempdir().unwrap();
        cmd_init(
            cfg_dir.path(),
            false,
            Some("test-device".into()),
            None,
            None,
        )
        .unwrap();

        let mut config = AppConfig::load_or_default(config_path_in(cfg_dir.path())).unwrap();
        let account = config.account.clone().unwrap();
        let mut drive = config
            .drive(iris_drive_core::PRIMARY_DRIVE_ID)
            .unwrap()
            .clone();
        let missing_root = Cid::encrypted([0x42; 32], [0x24; 32]).to_string();
        drive.device_roots.insert(
            account.device_pubkey.clone(),
            DeviceRootRef::legacy(missing_root.clone(), 1, 1),
        );
        config.upsert_drive(drive);
        config.save(config_path_in(cfg_dir.path())).unwrap();

        let error = build_current_sync_events(cfg_dir.path(), &config, &account)
            .await
            .unwrap_err()
            .to_string();

        assert!(
            error.contains(&missing_root),
            "error should name unreadable root {missing_root}: {error}"
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
