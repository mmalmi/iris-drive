//! Add an original to the private Photos folder, preserving both sides of name collisions.
use crate::{AppConfig, Daemon, mirror, provider::sanitized_provider_file_name};
use anyhow::{Context, Result, ensure};
use hashtree_core::{Cid, LinkType, TreeEntry, sha256, to_hex};
use std::path::Path;

const FOLDER: &str = "Photos";

pub struct PhotoOriginal {
    cid: Cid,
    size: u64,
    digest: [u8; 32],
    modified: Option<i64>,
}

impl PhotoOriginal {
    /// Staging can take minutes for a video. Run outside the config writer lock.
    pub async fn stage(daemon: &Daemon, source: &Path) -> Result<Self> {
        let (cid, size, digest, modified) = mirror::verified_original(daemon, source).await?;
        Ok(Self {
            cid,
            size,
            digest,
            modified,
        })
    }

    /// Return the updated root and its visible path. The caller owns the writer lock.
    pub async fn add_to(
        &self,
        daemon: &Daemon,
        root: &Cid,
        resource: &str,
        filename: &str,
    ) -> Result<(Cid, String)> {
        let tree = daemon.tree();
        let root_entries = tree.list_directory_required(root).await?;
        ensure!(
            !root_entries
                .iter()
                .any(|e| e.name != FOLDER && e.name.eq_ignore_ascii_case(FOLDER)),
            "A folder or file already uses another capitalization of Photos"
        );
        let folder_entry = root_entries.iter().find(|e| e.name == FOLDER);
        let (folder, entries) = if let Some(entry) = folder_entry {
            ensure!(
                entry.link_type.is_directory_like(),
                "A file already uses the name Photos"
            );
            let folder = Cid {
                hash: entry.hash,
                key: entry.key,
            };
            let entries = tree.list_directory_required(&folder).await?;
            (folder, entries)
        } else {
            (tree.put_directory(Vec::new()).await?, Vec::new())
        };
        let filename = sanitized_provider_file_name(filename);
        ensure!(
            !filename.starts_with('.') && !filename.chars().any(char::is_control),
            "Invalid photo filename"
        );
        let identify = to_hex(&sha256(resource.as_bytes()));
        // The root itself records the write. A crash before the phone saves its receipt
        // can retry and find the original even if a similarly named file came first.
        let id = &identify[..12];
        for entry in entries.iter().filter(|e| self.matches(e)) {
            if is_backup_variant(&entry.name, &filename) {
                return Ok((root.clone(), format!("{FOLDER}/{}", entry.name)));
            }
        }
        let mut number = 0;
        let name = loop {
            let candidate = backup_name(&filename, id, number);
            match entries
                .iter()
                .find(|e| e.name.to_lowercase() == candidate.to_lowercase())
            {
                Some(e) if self.matches(e) => {
                    return Ok((root.clone(), format!("{FOLDER}/{}", e.name)));
                }
                Some(_) => number += 1,
                None => break candidate,
            }
        };
        let kind = if self.size > hashtree_core::DEFAULT_CHUNK_SIZE as u64 {
            LinkType::File
        } else {
            LinkType::Blob
        };
        let updated = tree
            .set_entry_with_meta(
                &folder,
                &[],
                &name,
                &self.cid,
                self.size,
                kind,
                Some(crate::indexer::file_entry_meta(&self.digest, self.modified)),
            )
            .await?;
        let root = tree
            .set_entry_with_meta(
                root,
                &[],
                FOLDER,
                &updated,
                0,
                LinkType::Dir,
                folder_entry.and_then(|e| e.meta.clone()),
            )
            .await?;
        Ok((root, format!("{FOLDER}/{name}")))
    }

    fn matches(&self, e: &TreeEntry) -> bool {
        !e.link_type.is_directory_like() && e.hash == self.cid.hash && e.key == self.cid.key
    }
}

// A matching original may have been backed up by a different phone, whose
// local photo-library identifier produced a different collision suffix.
fn is_backup_variant(name: &str, original: &str) -> bool {
    let Some((_, suffix)) = name.rsplit_once(" (") else {
        return false;
    };
    let Some((marker, _)) = suffix.split_once(')') else {
        return false;
    };
    let (id, number) = marker.split_once('-').map_or((marker, Some(1)), |(id, n)| {
        (id, n.parse::<usize>().ok().filter(|n| *n >= 2))
    });
    id.len() == 12
        && id.bytes().all(|b| b.is_ascii_hexdigit())
        && number
            .is_some_and(|n| name.to_lowercase() == backup_name(original, id, n).to_lowercase())
}

fn backup_name(original: &str, id: &str, number: usize) -> String {
    let suffix = match number {
        0 => String::new(),
        1 => format!(" ({id})"),
        n => format!(" ({id}-{n})"),
    };
    // File Provider and desktop filesystems cap a single name at 255 UTF-8 bytes.
    let (stem, ext) = original
        .rfind('.')
        .map(|dot| original.split_at(dot))
        .filter(|(stem, ext)| {
            stem.chars()
                .next()
                .is_some_and(|c| c.len_utf8() + ext.len() + suffix.len() <= 255)
        })
        .unwrap_or((original, ""));
    let mut end = stem
        .len()
        .min(255_usize.saturating_sub(suffix.len() + ext.len()));
    while !stem.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{suffix}{ext}", &stem[..end])
}

pub fn check_profile(config: &AppConfig, expected: &str) -> Result<()> {
    let profile = config
        .profile
        .as_ref()
        .context("Sign in to back up photos")?;
    ensure!(
        profile.is_authorized() && profile.profile_id.to_string() == expected,
        "The signed-in Drive profile changed"
    );
    mirror::ensure_private(config, FOLDER)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn oversized_photo_names_fit_even_with_a_long_extension() {
        let original = format!("雪.{}", "花".repeat(100));
        for number in [0, 1, 20] {
            let name = backup_name(&original, "0123456789ab", number);
            assert!(name.starts_with('雪') && name.len() <= 255);
        }
    }

    #[tokio::test]
    async fn photo_backup_retries_and_collisions_preserve_both_originals() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        crate::identity::AppKey::load_or_generate(crate::paths::key_path_in(tmp.path()))?;
        let daemon = Daemon::open(tmp.path())?;
        let tree = daemon.tree();
        let empty = tree.put_directory(vec![]).await?;
        let staged = tmp.path().join("original");
        let bytes = vec![42; hashtree_core::DEFAULT_CHUNK_SIZE + 4096];
        std::fs::write(&staged, &bytes)?;
        let first = PhotoOriginal::stage(&daemon, &staged).await?;
        let (root, path) = first
            .add_to(&daemon, &empty, "device/1", "IMG_001.MOV")
            .await?;
        assert_eq!(path, "Photos/IMG_001.MOV");
        let retry = PhotoOriginal::stage(&daemon, &staged).await?;
        assert_eq!(
            retry
                .add_to(&daemon, &root, "device/1", "IMG_001.MOV")
                .await?,
            (root.clone(), path.clone())
        );
        std::fs::write(&staged, b"second clip")?;
        let other = PhotoOriginal::stage(&daemon, &staged).await?;
        let (root, second) = other
            .add_to(&daemon, &root, "device/2", "IMG_001.MOV")
            .await?;
        assert_ne!(second, path);
        assert_eq!(Path::new(&second).extension().unwrap(), "MOV");
        assert_eq!(
            other
                .add_to(&daemon, &root, "device/2", "IMG_001.MOV")
                .await?,
            (root.clone(), second.clone())
        );
        assert_eq!(
            other
                .add_to(&daemon, &root, "another-phone/7", "IMG_001.MOV")
                .await?,
            (root.clone(), second.clone()),
            "the other phone has the same original, including its camera filename"
        );
        let second_cid = tree.resolve_path(&root, &second).await?.unwrap();
        assert_eq!(
            tree.read_file_range_cid(&second_cid, 0, None)
                .await?
                .unwrap(),
            b"second clip"
        );
        let cid = tree.resolve_path(&root, &path).await?.unwrap();
        assert_eq!(
            tree.read_file_range_cid(&cid, 0, None).await?.unwrap(),
            bytes
        );
        let without_first = tree.remove_entry(&root, &[FOLDER], "IMG_001.MOV").await?;
        assert_eq!(
            other
                .add_to(&daemon, &without_first, "device/2", "IMG_001.MOV")
                .await?,
            (without_first, second)
        );
        let existing_file = tree
            .set_entry(&empty, &[], FOLDER, &cid, 10, LinkType::File)
            .await?;
        assert!(
            other
                .add_to(&daemon, &existing_file, "device/2", "clip.mov")
                .await
                .is_err()
        );
        Ok(())
    }
}
