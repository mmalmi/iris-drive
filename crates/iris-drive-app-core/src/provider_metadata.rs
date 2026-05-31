use std::collections::BTreeMap;

const MIN_PROVIDER_DISPLAY_UNIX_SECS: i64 = 946_684_800;

pub(crate) fn provider_modified_at_index(
    view: &iris_drive_core::projection::PrimaryMergedView,
) -> BTreeMap<String, i64> {
    let mut index = BTreeMap::new();
    for entry in &view.view.files {
        let Some(modified_at) = entry.modified_at else {
            continue;
        };
        remember_provider_modified_at(&mut index, &entry.path, modified_at);
        let mut path = entry.path.as_str();
        while let Some((parent, _name)) = path.rsplit_once('/') {
            remember_provider_modified_at(&mut index, parent, modified_at);
            path = parent;
        }
    }
    index
}

pub(crate) fn remember_provider_modified_at(
    index: &mut BTreeMap<String, i64>,
    path: &str,
    modified_at: i64,
) {
    if path.is_empty() || modified_at < MIN_PROVIDER_DISPLAY_UNIX_SECS {
        return;
    }
    index
        .entry(path.to_owned())
        .and_modify(|existing| *existing = (*existing).max(modified_at))
        .or_insert(modified_at);
}

#[cfg(test)]
mod tests {
    use iris_drive_core::merge::{MergedEntry, MergedView};
    use iris_drive_core::projection::PrimaryMergedView;

    use super::*;

    #[test]
    fn provider_modified_at_index_does_not_use_root_published_at() {
        let view = PrimaryMergedView {
            view: MergedView {
                files: vec![
                    MergedEntry {
                        path: "legacy.txt".to_string(),
                        source_path: None,
                        hash: [1; 32],
                        size: 1,
                        whole_file_hash: None,
                        modified_at: None,
                        source_device: "device".to_string(),
                        published_at: 1_800_000_000,
                    },
                    MergedEntry {
                        path: "docs/current.txt".to_string(),
                        source_path: None,
                        hash: [2; 32],
                        size: 2,
                        whole_file_hash: None,
                        modified_at: Some(1_700_000_000),
                        source_device: "device".to_string(),
                        published_at: 1_800_000_000,
                    },
                ],
                ..MergedView::default()
            },
            authorized_devices: 1,
            device_roots_present: 1,
        };

        let index = provider_modified_at_index(&view);

        assert!(!index.contains_key("legacy.txt"));
        assert_eq!(index.get("docs"), Some(&1_700_000_000));
        assert_eq!(index.get("docs/current.txt"), Some(&1_700_000_000));
    }
}
