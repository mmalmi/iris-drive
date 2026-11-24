//! Moves a previous install's Drive data into this app's data folder.
//!
//! The macOS app keeps its data in a group container named after the signing
//! team. When releases move to another team, the new app starts with an empty
//! container and copies the previous one once. The previous folder is kept.

use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use serde_json::json;

use crate::commands::AppDataCmd;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Adopted {
    /// This folder already holds a profile.
    Current,
    /// There is no previous profile to copy.
    NothingToAdopt,
    /// The previous folder was copied; `files` counts regular files.
    Copied { files: u64 },
}

pub(crate) fn cmd_app_data(command: AppDataCmd) -> Result<()> {
    match command {
        AppDataCmd::Adopt { from, to, json } => {
            let outcome = adopt(&from, &to, crate::service::retire_service)?;
            let (status, files) = match outcome {
                Adopted::Current => ("current", 0),
                Adopted::NothingToAdopt => ("nothing_to_adopt", 0),
                Adopted::Copied { files } => ("adopted", files),
            };
            if json {
                println!("{}", json!({ "status": status, "files": files }));
            } else {
                println!("{status}");
            }
            Ok(())
        }
    }
}

pub(crate) fn adopt(
    from: &Path,
    to: &Path,
    retire_service: impl FnOnce(&Path) -> Result<()>,
) -> Result<Adopted> {
    if to.join("Config").join("config.toml").exists() {
        return Ok(Adopted::Current);
    }
    let previous = from.join("Config").join("config.toml");
    match fs::metadata(&previous) {
        Ok(_) => {}
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Adopted::NothingToAdopt),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("reading previous Drive data {}", previous.display()));
        }
    }
    retire_service(&from.join("Config"))?;

    let parent = to.parent().context("Drive data folder has no parent")?;
    fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let staging = parent.join(format!(".iris-drive-adopt-{}", std::process::id()));
    remove_if_present(&staging)?;
    let files = match copy_tree(from, &staging) {
        Ok(files) => files,
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            return Err(error)
                .with_context(|| format!("copying previous Drive data {}", from.display()));
        }
    };
    point_settings_at(&staging, from, to)?;
    let record = json!({ "from": from.display().to_string(), "adopted_at": stamp });
    fs::write(staging.join("adopted-from.json"), format!("{record}\n"))?;
    if to.exists() {
        let name = to
            .file_name()
            .context("Drive data folder has no name")?
            .to_string_lossy();
        let aside = parent.join(format!("{name}.before-adopt-{stamp}"));
        fs::rename(to, &aside).with_context(|| format!("moving {} aside", to.display()))?;
    }
    fs::rename(&staging, to).with_context(|| format!("moving adopted data to {}", to.display()))?;
    Ok(Adopted::Copied { files })
}

fn remove_if_present(path: &Path) -> Result<()> {
    match fs::remove_dir_all(path) {
        Err(error) if error.kind() != ErrorKind::NotFound => {
            Err(error).with_context(|| format!("removing {}", path.display()))
        }
        _ => Ok(()),
    }
}

/// Settings files may hold absolute paths into the previous folder, including
/// JSON written with escaped slashes. Point them at the new folder. Only whole
/// path components match, so a sibling such as `<folder> Dev` stays as it is.
fn point_settings_at(root: &Path, from: &Path, to: &Path) -> Result<()> {
    let (old, new) = (from.display().to_string(), to.display().to_string());
    let (old_escaped, new_escaped) = (old.replace('/', "\\/"), new.replace('/', "\\/"));
    let replacements = [
        (format!("{old}/"), format!("{new}/")),
        (format!("{old}\""), format!("{new}\"")),
        (format!("{old_escaped}\\/"), format!("{new_escaped}\\/")),
        (format!("{old_escaped}\""), format!("{new_escaped}\"")),
    ];
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            point_settings_at(&path, from, to)?;
            continue;
        }
        let settings = path
            .extension()
            .is_some_and(|ext| ext == "json" || ext == "toml");
        if !kind.is_file() || !settings || entry.metadata()?.len() > 1 << 20 {
            continue;
        }
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        let updated = replacements
            .iter()
            .fold(text.clone(), |text, (old, new)| text.replace(old, new));
        if updated != text {
            fs::write(&path, updated).with_context(|| format!("updating {}", path.display()))?;
        }
    }
    Ok(())
}

/// Copies directories, regular files (APFS clones them) and symlinks.
/// Sockets and other special files belong to the stopped service and are skipped.
fn copy_tree(from: &Path, to: &Path) -> Result<u64> {
    fs::create_dir(to).with_context(|| format!("creating {}", to.display()))?;
    fs::set_permissions(to, fs::metadata(from)?.permissions())?;
    let mut files = 0;
    for entry in fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let entry = entry?;
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type()?;
        if kind.is_dir() {
            files += copy_tree(&source, &target)?;
        } else if kind.is_file() {
            fs::copy(&source, &target).with_context(|| format!("copying {}", source.display()))?;
            files += 1;
        } else if kind.is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(&source)?, &target)?;
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests;
