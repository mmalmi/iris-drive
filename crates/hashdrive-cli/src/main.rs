use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(name = "hdrive", version, about = "Hashdrive CLI / daemon")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Initialize a new hashdrive config in the OS config dir.
    Init,
    /// Print daemon and sync status as JSON.
    Status,
    /// Run the hashdrive daemon in the foreground.
    Start,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Command::Init => {
            println!("hdrive init: not implemented yet");
        }
        Command::Status => {
            println!("{{}}");
        }
        Command::Start => {
            println!("hdrive start: not implemented yet");
        }
    }
    Ok(())
}
