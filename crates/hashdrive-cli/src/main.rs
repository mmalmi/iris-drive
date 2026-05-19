use std::path::PathBuf;
use std::process::ExitCode;

use std::sync::Arc;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use hashdrive_core::{
    config::AppConfig,
    identity::Identity,
    index_dir,
    paths::{config_path_in, default_config_dir, key_path_in},
    Drive,
};
use hashtree_core::{HashTree, HashTreeConfig, MemoryStore};
use serde_json::json;

#[derive(Debug, Parser)]
#[command(name = "hdrive", version, about = "Hashdrive CLI / daemon")]
struct Cli {
    /// Override the config dir (default: OS config dir / hashdrive).
    #[arg(long, env = "HASHDRIVE_CONFIG_DIR", global = true)]
    config_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize a hashdrive config: generate an identity and a primary drive.
    Init {
        /// Don't error if config already exists; print the existing state.
        #[arg(long)]
        force: bool,
    },
    /// Print daemon and sync status as JSON.
    Status,
    /// List configured drives.
    Drives,
    /// Show the local identity (npub).
    Whoami,
    /// Index a local directory into an in-memory hashtree and print the
    /// root CID + summary. Useful for hands-on sanity checks against the
    /// indexer.
    Index {
        /// Directory to index.
        dir: PathBuf,
    },
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
        eprintln!("error: could not determine a config dir; set --config-dir or HASHDRIVE_CONFIG_DIR");
        return ExitCode::from(2);
    };

    let result = match cli.command {
        Command::Init { force } => cmd_init(&config_dir, force),
        Command::Status => cmd_status(&config_dir),
        Command::Drives => cmd_drives(&config_dir),
        Command::Whoami => cmd_whoami(&config_dir),
        Command::Index { dir } => cmd_index(&dir),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_init(config_dir: &std::path::Path, force: bool) -> Result<()> {
    let key_path = key_path_in(config_dir);
    let config_path = config_path_in(config_dir);

    let already = key_path.exists() && config_path.exists();
    if already && !force {
        eprintln!("hashdrive already initialized at {}", config_dir.display());
        eprintln!("use --force to print the existing state instead of erroring");
        return Err(anyhow::anyhow!("already initialized"));
    }

    let identity = Identity::load_or_generate(&key_path)
        .with_context(|| format!("loading or generating identity at {}", key_path.display()))?;

    let mut config = AppConfig::load_or_default(&config_path)?;
    if config.drive("main").is_none() {
        config.upsert_drive(Drive::primary(identity.pubkey_hex()));
    }
    config.save(&config_path)?;

    println!(
        "{}",
        json!({
            "config_dir": config_dir.display().to_string(),
            "key_path": key_path.display().to_string(),
            "config_path": config_path.display().to_string(),
            "pubkey_npub": identity.pubkey_bech32(),
            "drives": config.drives.iter().map(|d| &d.drive_id).collect::<Vec<_>>(),
        })
    );
    Ok(())
}

fn cmd_status(config_dir: &std::path::Path) -> Result<()> {
    let key_path = key_path_in(config_dir);
    let config_path = config_path_in(config_dir);
    let initialized = key_path.exists() && config_path.exists();
    let config = AppConfig::load_or_default(&config_path)
        .with_context(|| format!("reading config at {}", config_path.display()))?;
    let identity = if initialized {
        Identity::load(&key_path).ok()
    } else {
        None
    };
    println!(
        "{}",
        json!({
            "initialized": initialized,
            "config_dir": config_dir.display().to_string(),
            "pubkey_npub": identity.as_ref().map(hashdrive_core::Identity::pubkey_bech32),
            "drives": config.drives.iter().map(|d| json!({
                "drive_id": d.drive_id,
                "display_name": d.display_name,
                "owner_pubkey": d.owner_pubkey,
                "role": format!("{:?}", d.role).to_lowercase(),
                "last_root_cid": d.last_root_cid,
            })).collect::<Vec<_>>(),
        })
    );
    Ok(())
}

fn cmd_drives(config_dir: &std::path::Path) -> Result<()> {
    let config_path = config_path_in(config_dir);
    let config = AppConfig::load_or_default(&config_path)?;
    if config.drives.is_empty() {
        println!("(no drives — run `hdrive init`)");
        return Ok(());
    }
    for d in &config.drives {
        println!(
            "{:<24}  {:<7}  {:<32}  {}",
            d.drive_id,
            format!("{:?}", d.role).to_lowercase(),
            short_pubkey(&d.owner_pubkey),
            d.display_name,
        );
    }
    Ok(())
}

fn cmd_whoami(config_dir: &std::path::Path) -> Result<()> {
    let key_path = key_path_in(config_dir);
    let identity = Identity::load(&key_path)
        .with_context(|| format!("loading identity from {}", key_path.display()))?;
    println!("{}", identity.pubkey_bech32());
    Ok(())
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

fn short_pubkey(pk: &str) -> String {
    if pk.len() > 14 {
        format!("{}…{}", &pk[..6], &pk[pk.len() - 6..])
    } else {
        pk.to_string()
    }
}
