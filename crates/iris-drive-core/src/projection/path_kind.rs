use std::collections::{BTreeMap, BTreeSet};

use hashtree_core::HashTreeError;

use super::{DirectoryMeta, ProjectionError, merged_entry_source_path, next_visible_conflict_path};
use crate::config::AppKeyRootRef;
use crate::conflict::conflict_filename;
use crate::merge::{
    MergedEntry, MergedPathKindReplacement, MergedPathKindRoleRoot, MergedTombstone, RootRelation,
    SuppressedMergedEntry, root_relation,
};
use crate::root_meta::root_cid_identity_matches;

#[derive(Debug, Clone)]
pub(super) struct MergedDirectory {
    pub(super) path: String,
    pub(super) meta: DirectoryMeta,
    pub(super) source_app_key_pubkey: String,
    pub(super) source_root: AppKeyRootRef,
}

#[derive(Debug, Clone)]
struct ReplacementRoot {
    app_key_pubkey: String,
    root: AppKeyRootRef,
    recovery_tombstoned_at: Option<i64>,
}

/// A logical path cannot be both a file and a directory. Causal replacement
/// owns the canonical path. Concurrent conflicts keep the directory canonical
/// so its complete subtree remains visible and surface the file with the same
/// conflict naming used for write/write conflicts.
pub(super) fn resolve_path_kind_conflicts(
    mut files: Vec<MergedEntry>,
    suppressed_files: &[SuppressedMergedEntry],
    mut directories: BTreeMap<String, MergedDirectory>,
    tombstones: &[MergedTombstone],
    path_kind_replacements: &[MergedPathKindReplacement],
    path_kind_role_roots: &[MergedPathKindRoleRoot],
    source_roots: &BTreeMap<String, AppKeyRootRef>,
) -> Result<(Vec<MergedEntry>, BTreeMap<String, MergedDirectory>), ProjectionError> {
    files.retain(|file| {
        !tombstones.iter().any(|tombstone| {
            inactive_role_barrier_suppresses_file(
                tombstone,
                file,
                tombstones,
                path_kind_replacements,
                path_kind_role_roots,
                source_roots,
            )
        })
    });
    let mut replacement_roots = BTreeMap::<String, Vec<ReplacementRoot>>::new();
    for file in &files {
        let collision_path = merged_entry_source_path(file);
        if !directories.contains_key(collision_path) {
            continue;
        }
        let source_root = source_roots
            .get(&file.source_app_key_pubkey)
            .ok_or_else(|| HashTreeError::PathNotFound(file.source_app_key_pubkey.clone()))?;
        insert_replacement_root(
            &mut replacement_roots,
            collision_path,
            ReplacementRoot {
                app_key_pubkey: file.source_app_key_pubkey.clone(),
                root: source_root.clone(),
                recovery_tombstoned_at: None,
            },
        );
    }
    for suppressed in suppressed_files {
        let Some(directory) = directories.get(&suppressed.entry.path) else {
            continue;
        };
        if root_source_matches(
            &directory.source_app_key_pubkey,
            &directory.source_root,
            &suppressed.tombstone_app_key_pubkey,
            &suppressed.tombstone_root,
        ) {
            insert_replacement_root(
                &mut replacement_roots,
                &suppressed.entry.path,
                ReplacementRoot {
                    app_key_pubkey: directory.source_app_key_pubkey.clone(),
                    root: directory.source_root.clone(),
                    recovery_tombstoned_at: None,
                },
            );
        }
    }
    for (replacement_path, replacements) in &mut replacement_roots {
        for replacement in replacements {
            let matching_tombstones = tombstones
                .iter()
                .filter(|tombstone| {
                    path_is_at_or_below(&tombstone.path, replacement_path)
                        && root_source_matches(
                            &replacement.app_key_pubkey,
                            &replacement.root,
                            &tombstone.source_app_key_pubkey,
                            &tombstone.source_root,
                        )
                })
                .collect::<Vec<_>>();
            let exact_barrier = matching_tombstones
                .iter()
                .find(|tombstone| tombstone.path == *replacement_path)
                .map(|tombstone| tombstone.tombstoned_at);
            let newest_matching_tombstone = matching_tombstones
                .iter()
                .map(|tombstone| tombstone.tombstoned_at)
                .max();
            let role_aware = path_kind_role_roots.iter().any(|role_root| {
                root_source_matches(
                    &replacement.app_key_pubkey,
                    &replacement.root,
                    &role_root.source_app_key_pubkey,
                    &role_root.source_root,
                )
            });
            if role_aware {
                replacement.recovery_tombstoned_at = path_kind_replacements
                    .iter()
                    .find(|active| {
                        active.path == *replacement_path
                            && root_source_matches(
                                &replacement.app_key_pubkey,
                                &replacement.root,
                                &active.source_app_key_pubkey,
                                &active.source_root,
                            )
                            && exact_barrier == Some(active.generation)
                    })
                    .map(|active| active.generation);
            } else {
                // Legacy publishers had no explicit per-path role. Preserve
                // their old lossless behavior: prefer an exact barrier and
                // otherwise recover only the newest descendant marker batch.
                replacement.recovery_tombstoned_at = exact_barrier.or(newest_matching_tombstone);
            }
        }
    }
    directories.retain(|path, directory| {
        applicable_tombstone(path, tombstones).is_none_or(|tombstone| {
            root_source_matches(
                &directory.source_app_key_pubkey,
                &directory.source_root,
                &tombstone.source_app_key_pubkey,
                &tombstone.source_root,
            ) || replacement_roots
                .iter()
                .any(|(replacement_path, replacements)| {
                    path_is_at_or_below(path, replacement_path)
                        && replacements.iter().any(|replacement| {
                            replacement.recovery_tombstoned_at == Some(tombstone.tombstoned_at)
                                && root_source_matches(
                                    &replacement.app_key_pubkey,
                                    &replacement.root,
                                    &tombstone.source_app_key_pubkey,
                                    &tombstone.source_root,
                                )
                        })
                })
        })
    });

    let mut occupied_file_paths = files
        .iter()
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    for suppressed in suppressed_files {
        let belongs_to_replacement =
            replacement_roots
                .iter()
                .any(|(replacement_path, replacements)| {
                    path_is_at_or_below(&suppressed.entry.path, replacement_path)
                        && replacements.iter().any(|replacement| {
                            replacement.recovery_tombstoned_at == Some(suppressed.tombstoned_at)
                                && root_source_matches(
                                    &replacement.app_key_pubkey,
                                    &replacement.root,
                                    &suppressed.tombstone_app_key_pubkey,
                                    &suppressed.tombstone_root,
                                )
                        })
                });
        if belongs_to_replacement
            && !suppressed_entry_already_visible(&files, &suppressed.entry)
            && occupied_file_paths.insert(suppressed.entry.path.clone())
        {
            files.push(suppressed.entry.clone());
        }
    }

    while let Some(collision_index) = files
        .iter()
        .enumerate()
        .filter(|(_, file)| directories.contains_key(&file.path))
        .min_by(|(_, left), (_, right)| compare_paths_by_depth(&left.path, &right.path))
        .map(|(index, _)| index)
    {
        let collision_path = files[collision_index].path.clone();
        let directory = directories
            .get(&collision_path)
            .cloned()
            .ok_or_else(|| HashTreeError::PathNotFound(collision_path.clone()))?;
        let file_root = source_roots
            .get(&files[collision_index].source_app_key_pubkey)
            .ok_or_else(|| {
                HashTreeError::PathNotFound(files[collision_index].source_app_key_pubkey.clone())
            })?;
        let relation = root_relation(
            &directory.source_app_key_pubkey,
            &directory.source_root,
            &files[collision_index].source_app_key_pubkey,
            file_root,
        );

        // A path-kind role is durable across unrelated publications. Root-level
        // causality alone cannot distinguish a descendant snapshot carrying its
        // prior opposite-kind contribution from a deliberate new replacement.
        // When both roots understand the role document, keep the active role's
        // kind canonical until the descendant authors its own exact barrier or
        // active role. Legacy roots retain the historical relation-only policy.
        let directory_role_persists = relation == RootRelation::RightDescends
            && active_role_persists_against_descendant(
                &collision_path,
                &directory.source_app_key_pubkey,
                &directory.source_root,
                &files[collision_index].source_app_key_pubkey,
                file_root,
                tombstones,
                path_kind_replacements,
                path_kind_role_roots,
            );
        let file_role_persists = relation == RootRelation::LeftDescends
            && active_role_persists_against_descendant(
                &collision_path,
                &files[collision_index].source_app_key_pubkey,
                file_root,
                &directory.source_app_key_pubkey,
                &directory.source_root,
                tombstones,
                path_kind_replacements,
                path_kind_role_roots,
            );
        let file_is_canonical = (relation == RootRelation::RightDescends
            && !directory_role_persists)
            || file_role_persists;

        if !file_is_canonical {
            let mut occupied = collect_occupied_paths(
                files
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| *index != collision_index)
                    .map(|(_, file)| file.path.clone())
                    .chain(directories.keys().cloned()),
            );
            let conflict_path = next_visible_conflict_path(
                &collision_path,
                &files[collision_index].source_app_key_pubkey,
                &mut occupied,
            );
            let source_path = merged_entry_source_path(&files[collision_index]).to_string();
            files[collision_index].path = conflict_path;
            files[collision_index].source_path = Some(source_path);
            continue;
        }

        let moving_directories = directories
            .values()
            .filter(|entry| path_is_at_or_below(&entry.path, &collision_path))
            .cloned()
            .collect::<Vec<_>>();
        let moving_file_indices = files
            .iter()
            .enumerate()
            .filter(|(index, file)| {
                *index != collision_index && path_is_at_or_below(&file.path, &collision_path)
            })
            .map(|(index, _)| index)
            .collect::<BTreeSet<_>>();
        let stationary_directories = directories
            .values()
            .filter(|entry| !path_is_at_or_below(&entry.path, &collision_path))
            .cloned()
            .map(|entry| (entry.path.clone(), entry))
            .collect::<BTreeMap<_, _>>();
        let occupied = collect_occupied_paths(
            files
                .iter()
                .enumerate()
                .filter(|(index, _)| !moving_file_indices.contains(index))
                .map(|(_, file)| file.path.clone())
                .chain(stationary_directories.keys().cloned()),
        );
        let moving_paths = moving_directories
            .iter()
            .map(|entry| entry.path.clone())
            .chain(
                moving_file_indices
                    .iter()
                    .map(|index| files[*index].path.clone()),
            )
            .collect::<Vec<_>>();
        let conflict_root = next_subtree_conflict_path(
            &collision_path,
            &directory.source_app_key_pubkey,
            &moving_paths,
            &occupied,
        );

        directories = stationary_directories;
        for mut entry in moving_directories {
            entry.path = remap_subtree_path(&entry.path, &collision_path, &conflict_root);
            directories.insert(entry.path.clone(), entry);
        }
        for index in moving_file_indices {
            let source_path = merged_entry_source_path(&files[index]).to_string();
            files[index].path =
                remap_subtree_path(&files[index].path, &collision_path, &conflict_root);
            files[index].source_path = Some(source_path);
        }
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok((files, directories))
}

fn active_role_persists_against_descendant(
    path: &str,
    active_app_key: &str,
    active_root: &AppKeyRootRef,
    descendant_app_key: &str,
    descendant_root: &AppKeyRootRef,
    tombstones: &[MergedTombstone],
    path_kind_replacements: &[MergedPathKindReplacement],
    path_kind_role_roots: &[MergedPathKindRoleRoot],
) -> bool {
    root_is_role_aware(active_app_key, active_root, path_kind_role_roots)
        && root_is_role_aware(descendant_app_key, descendant_root, path_kind_role_roots)
        && root_has_active_replacement(
            path,
            active_app_key,
            active_root,
            tombstones,
            path_kind_replacements,
        )
        && !root_has_active_replacement(
            path,
            descendant_app_key,
            descendant_root,
            tombstones,
            path_kind_replacements,
        )
        && !root_has_exact_barrier(path, descendant_app_key, descendant_root, tombstones)
}

fn root_is_role_aware(
    app_key: &str,
    root: &AppKeyRootRef,
    path_kind_role_roots: &[MergedPathKindRoleRoot],
) -> bool {
    path_kind_role_roots.iter().any(|role_root| {
        root_source_matches(
            app_key,
            root,
            &role_root.source_app_key_pubkey,
            &role_root.source_root,
        )
    })
}

fn root_has_active_replacement(
    path: &str,
    app_key: &str,
    root: &AppKeyRootRef,
    tombstones: &[MergedTombstone],
    path_kind_replacements: &[MergedPathKindReplacement],
) -> bool {
    path_kind_replacements.iter().any(|replacement| {
        replacement.path == path
            && root_source_matches(
                app_key,
                root,
                &replacement.source_app_key_pubkey,
                &replacement.source_root,
            )
            && tombstones.iter().any(|tombstone| {
                tombstone.path == path
                    && tombstone.tombstoned_at == replacement.generation
                    && root_source_matches(
                        app_key,
                        root,
                        &tombstone.source_app_key_pubkey,
                        &tombstone.source_root,
                    )
            })
    })
}

fn root_has_exact_barrier(
    path: &str,
    app_key: &str,
    root: &AppKeyRootRef,
    tombstones: &[MergedTombstone],
) -> bool {
    tombstones.iter().any(|tombstone| {
        tombstone.path == path
            && root_source_matches(
                app_key,
                root,
                &tombstone.source_app_key_pubkey,
                &tombstone.source_root,
            )
    })
}

fn inactive_role_barrier_suppresses_file(
    tombstone: &MergedTombstone,
    file: &MergedEntry,
    tombstones: &[MergedTombstone],
    path_kind_replacements: &[MergedPathKindReplacement],
    path_kind_role_roots: &[MergedPathKindRoleRoot],
    source_roots: &BTreeMap<String, AppKeyRootRef>,
) -> bool {
    let source_path = merged_entry_source_path(file);
    if !path_kind_role_roots.iter().any(|role_root| {
        root_source_matches(
            &tombstone.source_app_key_pubkey,
            &tombstone.source_root,
            &role_root.source_app_key_pubkey,
            &role_root.source_root,
        )
    }) || path_kind_replacements.iter().any(|active| {
        active.path == tombstone.path
            && active.generation == tombstone.tombstoned_at
            && root_source_matches(
                &tombstone.source_app_key_pubkey,
                &tombstone.source_root,
                &active.source_app_key_pubkey,
                &active.source_root,
            )
    }) || source_path == tombstone.path
        || tombstones.iter().any(|exact| {
            exact.path == source_path
                && root_source_matches(
                    &tombstone.source_app_key_pubkey,
                    &tombstone.source_root,
                    &exact.source_app_key_pubkey,
                    &exact.source_root,
                )
        })
        || !path_is_at_or_below(source_path, &tombstone.path)
    {
        return false;
    }
    let Some(file_root) = source_roots.get(&file.source_app_key_pubkey) else {
        return false;
    };
    if root_source_matches(
        &tombstone.source_app_key_pubkey,
        &tombstone.source_root,
        &file.source_app_key_pubkey,
        file_root,
    ) {
        return false;
    }
    match root_relation(
        &tombstone.source_app_key_pubkey,
        &tombstone.source_root,
        &file.source_app_key_pubkey,
        file_root,
    ) {
        RootRelation::Same | RootRelation::LeftDescends => true,
        RootRelation::RightDescends => false,
        RootRelation::Concurrent => tombstone.tombstoned_at >= file.published_at,
    }
}

fn insert_replacement_root(
    replacement_roots: &mut BTreeMap<String, Vec<ReplacementRoot>>,
    path: &str,
    candidate: ReplacementRoot,
) {
    let replacements = replacement_roots.entry(path.to_string()).or_default();
    if !replacements.iter().any(|existing| {
        root_source_matches(
            &existing.app_key_pubkey,
            &existing.root,
            &candidate.app_key_pubkey,
            &candidate.root,
        )
    }) {
        replacements.push(candidate);
    }
}

fn root_source_matches(
    left_app_key: &str,
    left_root: &AppKeyRootRef,
    right_app_key: &str,
    right_root: &AppKeyRootRef,
) -> bool {
    left_app_key == right_app_key
        && root_cid_identity_matches(&left_root.root_cid, &right_root.root_cid)
}

fn suppressed_entry_already_visible(files: &[MergedEntry], suppressed: &MergedEntry) -> bool {
    files.iter().any(|file| {
        file.source_app_key_pubkey == suppressed.source_app_key_pubkey
            && merged_entry_source_path(file) == suppressed.path
            && file.hash == suppressed.hash
            && file.size == suppressed.size
            && file.whole_file_hash == suppressed.whole_file_hash
            && file.modified_at == suppressed.modified_at
            && file.published_at == suppressed.published_at
    })
}

pub(super) fn insert_directory_candidate(
    directories: &mut BTreeMap<String, MergedDirectory>,
    candidate: MergedDirectory,
) {
    if let Some(current) = directories.get_mut(&candidate.path) {
        if directory_candidate_wins(&candidate, current) {
            *current = candidate;
        }
    } else {
        directories.insert(candidate.path.clone(), candidate);
    }
}

fn directory_candidate_wins(candidate: &MergedDirectory, current: &MergedDirectory) -> bool {
    match root_relation(
        &candidate.source_app_key_pubkey,
        &candidate.source_root,
        &current.source_app_key_pubkey,
        &current.source_root,
    ) {
        RootRelation::Same | RootRelation::LeftDescends => true,
        RootRelation::RightDescends => false,
        RootRelation::Concurrent => {
            candidate.source_root.published_at > current.source_root.published_at
                || (candidate.source_root.published_at == current.source_root.published_at
                    && candidate.source_app_key_pubkey > current.source_app_key_pubkey)
        }
    }
}

fn compare_paths_by_depth(left: &str, right: &str) -> std::cmp::Ordering {
    left.split('/')
        .count()
        .cmp(&right.split('/').count())
        .then_with(|| left.cmp(right))
}

fn collect_occupied_paths(paths: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    let mut occupied = BTreeSet::new();
    for path in paths {
        let segments = path.split('/').collect::<Vec<_>>();
        for depth in 1..=segments.len() {
            occupied.insert(segments[..depth].join("/"));
        }
    }
    occupied
}

fn next_subtree_conflict_path(
    original_path: &str,
    app_key_pubkey: &str,
    moving_paths: &[String],
    occupied_paths: &BTreeSet<String>,
) -> String {
    for index in 1..=256 {
        let label = if index == 1 {
            app_key_pubkey.to_string()
        } else {
            format!("{app_key_pubkey} {index}")
        };
        let candidate = conflict_filename(original_path, &label);
        if moving_paths.iter().all(|path| {
            !occupied_paths.contains(&remap_subtree_path(path, original_path, &candidate))
        }) {
            return candidate;
        }
    }
    conflict_filename(original_path, &format!("{app_key_pubkey} 257"))
}

fn path_is_at_or_below(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|remainder| remainder.starts_with('/'))
}

fn applicable_tombstone<'a>(
    path: &str,
    tombstones: &'a [MergedTombstone],
) -> Option<&'a MergedTombstone> {
    tombstones
        .iter()
        .filter(|tombstone| path_is_at_or_below(path, &tombstone.path))
        .max_by(|left, right| compare_paths_by_depth(&left.path, &right.path))
}

fn remap_subtree_path(path: &str, from: &str, to: &str) -> String {
    if path == from {
        to.to_string()
    } else {
        format!("{to}{}", &path[from.len()..])
    }
}
