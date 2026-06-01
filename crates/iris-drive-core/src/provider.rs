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

pub fn split_provider_path(path: &str) -> anyhow::Result<(String, String)> {
    let path = normalize_provider_path(path)?;
    let Some((parent, name)) = path.rsplit_once('/') else {
        return Ok((String::new(), path));
    };
    Ok((parent.to_owned(), name.to_owned()))
}

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
}
