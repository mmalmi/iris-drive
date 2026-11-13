use crate::commands::MirrorCmd;
use anyhow::{Context, Result};
use iris_drive_core::mirror;
use serde_json::json;
use std::path::{Path, PathBuf};

pub(crate) fn command(config: &Path, cmd: Option<MirrorCmd>, json: bool) -> Result<()> {
    match cmd.unwrap_or(MirrorCmd::List) {
        MirrorCmd::Add { source, to } => {
            let mirror = mirror::add(config, &source, to.as_deref(), None)?;
            if json {
                println!("{}", serde_json::to_string(&mirror)?);
            } else {
                println!(
                    "{} → My Drive/{}\nChecks every 5 minutes while Iris Drive is running.",
                    mirror.source.display(),
                    mirror.destination
                );
            }
        }
        MirrorCmd::List => {
            let mirrors = mirror::list(config)?;
            if json {
                // The bookmark is a local Mac access credential.
                let rows = mirrors
                    .iter()
                    .map(|s| {
                        json!({"source": s.mirror.source, "destination": s.mirror.destination,
                    "last_run": s.last_run, "running": s.running})
                    })
                    .collect::<Vec<_>>();
                println!("{}", serde_json::to_string(&rows)?);
            } else {
                if mirrors.is_empty() {
                    println!("No mirrored folders. Use idrive mirror add <folder> to start.");
                }
                for s in mirrors {
                    println!(
                        "{} → My Drive/{}",
                        s.mirror.source.display(),
                        s.mirror.destination
                    );
                    if s.running {
                        println!("  Checking now");
                    }
                    if let Some(report) = s.last_run {
                        print_report(&report);
                    } else {
                        println!("  Waiting for first check");
                    }
                }
            }
        }
        MirrorCmd::Run { destination } => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            let report = rt
                .block_on(mirror::run(config, &destination, false))?
                .context("Mirror did not run")?;
            if json {
                println!("{}", serde_json::to_string(&report)?);
            } else {
                print_report(&report);
            }
            anyhow::ensure!(
                report.pending == 0,
                "{} item(s) need attention",
                report.pending
            );
        }
        MirrorCmd::Remove { destination } => {
            mirror::remove(config, &destination)?;
            if json {
                println!("{}", json!({"removed":destination}));
            } else {
                println!("Stopped mirroring. Files remain in My Drive/{destination}.");
            }
        }
    }
    Ok(())
}

fn print_report(r: &mirror::Report) {
    println!(
        "  {} copied · {} unchanged · {} need attention",
        r.copied, r.unchanged, r.pending
    );
    for issue in &r.issues {
        println!("  {issue}");
    }
}

pub(crate) async fn schedule(config: PathBuf) {
    loop {
        if let Err(error) = check_due(&config).await {
            tracing::warn!(%error, "folder mirror check");
        }
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
    }
}

async fn check_due(config: &Path) -> Result<()> {
    for s in mirror::list(config)? {
        if s.mirror.bookmark.is_some() || s.running || !s.due {
            continue;
        }
        if let Err(error) =
            mirror::run_with_access(config, &s.mirror.destination, true, Some(s.mirror.id), None)
                .await
        {
            tracing::warn!(%error, "could not run scheduled folder mirror");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn scheduled_checks_honor_interval_and_mac_folder_access() {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("config");
        let source = tmp.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("example"), b"hello").unwrap();
        crate::cmd_init(&config, false, Some("Mirror test".to_owned()), None, None).unwrap();
        mirror::add(&config, &source, Some("CLI"), None).unwrap();
        mirror::add(&config, &source, Some("Mac"), Some("bookmark".to_owned())).unwrap();
        check_due(&config).await.unwrap();
        let rows = mirror::list(&config).unwrap();
        assert_eq!(rows[0].last_run.as_ref().unwrap().copied, 1);
        assert!(!rows[0].due);
        assert!(rows[1].last_run.is_none());
        assert!(mirror::run(&config, "CLI", true).await.unwrap().is_none());
    }
}
