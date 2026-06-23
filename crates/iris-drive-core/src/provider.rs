use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProviderListEntry {
    pub path: String,
    pub parent_path: String,
    pub display_name: String,
    pub kind: &'static str,
    pub size: u64,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modified_at: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderListSummary {
    pub file_count: u64,
    pub visible_file_bytes: u64,
    pub directory_paths: Vec<String>,
    pub change_key: String,
}

#[must_use]
pub fn provider_list_summary(anchor: &str, entries: &[ProviderListEntry]) -> ProviderListSummary {
    let mut file_count = 0_u64;
    let mut visible_file_bytes = 0_u64;
    let mut directory_paths = Vec::new();
    let mut entry_keys = Vec::new();
    for entry in entries {
        if entry.kind == "directory" {
            directory_paths.push(entry.path.clone());
        } else {
            file_count += 1;
            visible_file_bytes = visible_file_bytes.saturating_add(entry.size);
        }
        entry_keys.push(format!(
            "{}:{}:{}:{}:{}",
            entry.kind,
            entry.path,
            entry.size,
            entry.version,
            entry.modified_at.unwrap_or_default()
        ));
    }
    directory_paths.sort();
    entry_keys.sort();
    ProviderListSummary {
        file_count,
        visible_file_bytes,
        directory_paths,
        change_key: format!("{anchor}|{}", entry_keys.join("|")),
    }
}

#[must_use]
pub fn provider_refresh_key(current_root_cid: Option<&str>, peers: &[serde_json::Value]) -> String {
    let mut parts = Vec::new();
    if let Some(current) = current_root_cid
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        parts.push(format!("current:{current}"));
    }

    for peer in peers {
        let label = non_empty_json_str(peer, "label")
            .or_else(|| non_empty_json_str(peer, "app_key_npub"))
            .or_else(|| non_empty_json_str(peer, "app_key_pubkey"))
            .unwrap_or("peer");
        let root_cid = non_empty_json_str(peer, "root_cid").unwrap_or("no-root");
        let sync_state = peer
            .get("sync_state")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let root_available = peer
            .get("root_available")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        parts.push(format!("{label}:{root_cid}:{sync_state}:{root_available}"));

        if let Some(block_sync) = peer
            .get("last_block_sync")
            .and_then(serde_json::Value::as_object)
        {
            let block_root = block_sync
                .get("root_cid")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.trim().is_empty())
                .unwrap_or(root_cid);
            let transport = block_sync
                .get("transport")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let fetched = json_i64(block_sync.get("fetched"));
            let already_local = json_i64(block_sync.get("already_local"));
            let total_hashes = json_i64(block_sync.get("total_hashes"));
            parts.push(format!(
                "{label}:blocks:{block_root}:{transport}:{fetched}:{already_local}:{total_hashes}"
            ));
        }
    }

    parts.sort();
    parts.join("|")
}

fn non_empty_json_str<'a>(value: &'a serde_json::Value, name: &str) -> Option<&'a str> {
    value
        .get(name)
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.trim().is_empty())
}

fn json_i64(value: Option<&serde_json::Value>) -> i64 {
    value
        .and_then(|value| {
            value.as_i64().or_else(|| {
                value
                    .as_u64()
                    .and_then(|unsigned| i64::try_from(unsigned).ok())
            })
        })
        .unwrap_or_default()
}

pub fn normalize_provider_path(path: &str) -> anyhow::Result<String> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        anyhow::bail!("provider path is required");
    }
    let mut segments = Vec::new();
    for segment in trimmed.split('/') {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.contains('\\')
            || segment.contains(':')
        {
            anyhow::bail!("unsafe provider path: {path}");
        }
        segments.push(segment);
    }
    Ok(segments.join("/"))
}

pub fn normalize_provider_document_path(path: &str) -> anyhow::Result<String> {
    let normalized = normalize_provider_path(path)?;
    if normalized != path {
        anyhow::bail!("provider document id is not a canonical provider path: {path}");
    }
    Ok(normalized)
}

pub fn normalize_provider_parent_path(path: &str) -> anyhow::Result<String> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        return Ok(String::new());
    }
    normalize_provider_path(trimmed)
}

pub fn optional_normalized_provider_path(path: &str) -> anyhow::Result<Option<String>> {
    let trimmed = path.trim_matches('/');
    if trimmed.is_empty() {
        Ok(None)
    } else {
        normalize_provider_path(trimmed).map(Some)
    }
}

pub fn provider_path_is_child_document(
    parent_path: &str,
    document_path: &str,
) -> anyhow::Result<bool> {
    let parent_path = normalize_provider_parent_path(parent_path)?;
    let document_path = normalize_provider_parent_path(document_path)?;
    Ok(parent_path.is_empty()
        || document_path == parent_path
        || document_path.starts_with(&format!("{parent_path}/")))
}

#[must_use]
pub fn sanitized_provider_file_name(display_name: &str) -> String {
    let mut name = display_name
        .split(['/', ':', '\\'])
        .map(str::trim)
        .filter(|part| !part.is_empty() && *part != "." && *part != "..")
        .collect::<Vec<_>>()
        .join("_");
    if name.is_empty() {
        "Shared file".clone_into(&mut name);
    }
    name
}

#[must_use]
pub fn unique_provider_path(
    entries: &[ProviderListEntry],
    parent: &str,
    name: &str,
    excluding: Option<&str>,
) -> String {
    let prefix = if parent.is_empty() {
        String::new()
    } else {
        format!("{parent}/")
    };
    let existing = entries
        .iter()
        .map(|entry| entry.path.as_str())
        .filter(|path| Some(*path) != excluding)
        .collect::<BTreeSet<_>>();
    let mut candidate = format!("{prefix}{name}");
    if !existing.contains(candidate.as_str()) {
        return candidate;
    }

    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .unwrap_or("Shared file");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    let mut index = 2;
    while existing.contains(candidate.as_str()) {
        candidate = format!("{prefix}{stem} ({index}){extension}");
        index += 1;
    }
    candidate
}

#[must_use]
pub fn provider_collision_family_path(path: &str) -> (String, usize) {
    let (parent, name) = path
        .rsplit_once('/')
        .map_or(("", path), |(parent, name)| (parent, name));
    let (mut stem, extension) = split_extension(name);
    let mut depth = 0usize;
    loop {
        if let Some(base) = strip_numeric_collision_suffix(&stem) {
            stem = base.to_string();
            depth += 1;
            continue;
        }
        if let Some(base) = strip_copy_collision_suffix(&stem) {
            stem = base.to_string();
            depth += 1;
            continue;
        }
        break;
    }
    let family_name = format!("{stem}{extension}");
    if parent.is_empty() {
        (family_name, depth)
    } else {
        (format!("{parent}/{family_name}"), depth)
    }
}

fn strip_numeric_collision_suffix(stem: &str) -> Option<&str> {
    let inner = stem.strip_suffix(')')?;
    let (base, number) = inner.rsplit_once(" (")?;
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(base)
}

fn strip_copy_collision_suffix(stem: &str) -> Option<&str> {
    if let Some(base) = stem.strip_suffix(" copy") {
        return Some(base);
    }
    let (base, number) = stem.rsplit_once(" copy ")?;
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(base)
}

fn split_extension(name: &str) -> (String, &str) {
    if let Some((stem, _extension)) = name.rsplit_once('.')
        && !stem.is_empty()
    {
        return (stem.to_string(), &name[stem.len()..]);
    }
    (name.to_string(), "")
}

pub fn split_provider_path(path: &str) -> anyhow::Result<(String, String)> {
    let path = normalize_provider_path(path)?;
    let Some((parent, name)) = path.rsplit_once('/') else {
        return Ok((String::new(), path));
    };
    Ok((parent.to_owned(), name.to_owned()))
}

#[must_use]
pub fn provider_cache_destination(target_dir: &Path, provider_path: &str) -> Option<PathBuf> {
    let path = normalize_provider_path(provider_path).ok()?;
    let mut destination = target_dir.to_path_buf();
    for segment in path.split('/') {
        destination.push(segment);
    }
    Some(destination)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_path_normalization_rejects_native_separator_aliases() {
        assert!(normalize_provider_path("Reports\\note.txt").is_err());
        assert!(normalize_provider_path("Reports:note.txt").is_err());
    }

    #[test]
    fn provider_document_path_requires_canonical_relative_path() {
        assert_eq!(
            normalize_provider_document_path("Reports/note.txt").unwrap(),
            "Reports/note.txt"
        );
        assert!(normalize_provider_document_path("/Reports/note.txt").is_err());
        assert!(normalize_provider_document_path("Reports/note.txt/").is_err());
        assert!(normalize_provider_document_path("Reports/../note.txt").is_err());
        assert!(normalize_provider_document_path("Reports\\note.txt").is_err());
    }

    #[test]
    fn provider_child_relation_uses_canonical_path_boundaries() {
        assert!(provider_path_is_child_document("", "").unwrap());
        assert!(provider_path_is_child_document("", "Projects/plan.md").unwrap());
        assert!(provider_path_is_child_document("Projects", "Projects").unwrap());
        assert!(provider_path_is_child_document("/Projects/", "/Projects/plan.md").unwrap());
        assert!(!provider_path_is_child_document("Projects", "Projects-old/plan.md").unwrap());
        assert!(!provider_path_is_child_document("Projects", "Notes/plan.md").unwrap());
        assert!(provider_path_is_child_document("Projects", "Projects\\plan.md").is_err());
    }

    #[test]
    fn provider_resolve_path_reports_collision_display_name() {
        let entries = vec![ProviderListEntry {
            path: "Reports/Shared_file.txt".to_string(),
            parent_path: "Reports".to_string(),
            display_name: "Shared_file.txt".to_string(),
            kind: "file",
            size: 5,
            version: "root".to_string(),
            modified_at: None,
        }];

        let path = unique_provider_path(&entries, "Reports", "Shared_file.txt", None);
        let (parent_path, display_name) = split_provider_path(&path).unwrap();

        assert_eq!(parent_path, "Reports");
        assert_eq!(display_name, "Shared_file (2).txt");
    }

    #[test]
    fn provider_collision_family_strips_repeated_numeric_suffixes() {
        assert_eq!(
            provider_collision_family_path("Reports/photo (2) (3).png"),
            ("Reports/photo.png".to_string(), 2)
        );
        assert_eq!(
            provider_collision_family_path("Reports/photo copy (2).png"),
            ("Reports/photo.png".to_string(), 2)
        );
        assert_eq!(
            provider_collision_family_path("Reports/photo copy 2.png"),
            ("Reports/photo.png".to_string(), 1)
        );
        assert_eq!(
            provider_collision_family_path("photo.png"),
            ("photo.png".to_string(), 0)
        );
        assert_eq!(
            provider_collision_family_path("photo (copy).png"),
            ("photo (copy).png".to_string(), 0)
        );
    }

    #[test]
    fn provider_list_summary_includes_counts_directories_and_change_key() {
        let entries = vec![
            ProviderListEntry {
                path: "Reports".to_string(),
                parent_path: String::new(),
                display_name: "Reports".to_string(),
                kind: "directory",
                size: 0,
                version: "dir-root".to_string(),
                modified_at: Some(1_700_000_000),
            },
            ProviderListEntry {
                path: "Reports/nested.txt".to_string(),
                parent_path: "Reports".to_string(),
                display_name: "nested.txt".to_string(),
                kind: "file",
                size: 12,
                version: "file-root".to_string(),
                modified_at: Some(1_700_000_001),
            },
        ];

        let summary = provider_list_summary("anchor", &entries);

        assert_eq!(summary.file_count, 1);
        assert_eq!(summary.visible_file_bytes, 12);
        assert_eq!(summary.directory_paths, vec!["Reports"]);
        assert!(summary.change_key.contains("Reports/nested.txt"));
        assert!(summary.change_key.contains("file"));
    }

    #[test]
    fn provider_refresh_key_includes_root_and_peer_block_sync_state() {
        let peers = vec![
            serde_json::json!({
                "label": "Laptop",
                "root_cid": "peer-root-b",
                "sync_state": "synced",
                "root_available": true,
                "last_block_sync": {
                    "root_cid": "peer-root-b",
                    "transport": "fips",
                    "fetched": 2,
                    "already_local": 3,
                    "total_hashes": 5
                }
            }),
            serde_json::json!({
                "app_key_npub": "npub1appkey",
                "root_cid": "peer-root-a",
                "sync_state": "pending"
            }),
        ];

        assert_eq!(
            provider_refresh_key(Some("current-root"), &peers),
            concat!(
                "Laptop:blocks:peer-root-b:fips:2:3:5|",
                "Laptop:peer-root-b:synced:true|",
                "current:current-root|",
                "npub1appkey:peer-root-a:pending:false"
            )
        );
    }
}
