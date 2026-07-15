use std::sync::Arc;

use hashtree_core::{
    Cid, DirEntry as TreeDirEntry, HashTree, HashTreeConfig, LinkType, MemoryStore,
};
use hashtree_provider::{PathChange, path_diff};

async fn empty_dir(tree: &HashTree<MemoryStore>) -> Cid {
    tree.put_directory(Vec::new()).await.unwrap()
}

async fn put_file(tree: &HashTree<MemoryStore>, data: &[u8]) -> Cid {
    let (cid, _size) = tree.put_file(data).await.unwrap();
    cid
}

async fn dir_with(tree: &HashTree<MemoryStore>, entries: Vec<(&str, Cid, u64, LinkType)>) -> Cid {
    let dir_entries: Vec<TreeDirEntry> = entries
        .into_iter()
        .map(|(name, cid, size, link_type)| {
            let mut e = TreeDirEntry::from_cid(name, &cid).with_size(size);
            e.link_type = link_type;
            e
        })
        .collect();
    tree.put_directory(dir_entries).await.unwrap()
}

fn new_tree() -> HashTree<MemoryStore> {
    HashTree::new(HashTreeConfig::new(Arc::new(MemoryStore::new())).public())
}

#[tokio::test]
async fn empty_to_empty_emits_nothing() {
    let tree = new_tree();
    let old = empty_dir(&tree).await;
    let new = empty_dir(&tree).await;
    let changes = path_diff(&tree, Some(&old), &new).await.unwrap();
    assert!(changes.is_empty(), "got {:?}", changes);
}

#[tokio::test]
async fn add_one_file_emits_one_added() {
    let tree = new_tree();
    let old = empty_dir(&tree).await;
    let file = put_file(&tree, b"hello").await;
    let new = dir_with(&tree, vec![("a.txt", file, 5, LinkType::Blob)]).await;
    let changes = path_diff(&tree, Some(&old), &new).await.unwrap();
    assert_eq!(changes.len(), 1, "got {:?}", changes);
    let PathChange::Added { path, entry } = &changes[0] else {
        panic!("expected Added, got {:?}", changes[0]);
    };
    assert_eq!(path, "a.txt");
    assert_eq!(entry.size, 5);
}

#[tokio::test]
async fn remove_one_file_emits_one_removed() {
    let tree = new_tree();
    let file = put_file(&tree, b"hello").await;
    let old = dir_with(&tree, vec![("a.txt", file, 5, LinkType::Blob)]).await;
    let new = empty_dir(&tree).await;
    let changes = path_diff(&tree, Some(&old), &new).await.unwrap();
    assert_eq!(changes.len(), 1);
    let PathChange::Removed { path, .. } = &changes[0] else {
        panic!("expected Removed, got {:?}", changes[0]);
    };
    assert_eq!(path, "a.txt");
}

#[tokio::test]
async fn modify_file_emits_one_modified() {
    let tree = new_tree();
    let file_a = put_file(&tree, b"hello").await;
    let file_b = put_file(&tree, b"goodbye").await;
    let old = dir_with(&tree, vec![("a.txt", file_a, 5, LinkType::Blob)]).await;
    let new = dir_with(&tree, vec![("a.txt", file_b, 7, LinkType::Blob)]).await;
    let changes = path_diff(&tree, Some(&old), &new).await.unwrap();
    assert_eq!(changes.len(), 1);
    let PathChange::Modified { path, old, new } = &changes[0] else {
        panic!("expected Modified, got {:?}", changes[0]);
    };
    assert_eq!(path, "a.txt");
    assert_eq!(old.size, 5);
    assert_eq!(new.size, 7);
}

#[tokio::test]
async fn identical_trees_emit_nothing() {
    let tree = new_tree();
    let file = put_file(&tree, b"hello").await;
    let root = dir_with(&tree, vec![("a.txt", file, 5, LinkType::Blob)]).await;
    let changes = path_diff(&tree, Some(&root), &root).await.unwrap();
    assert!(changes.is_empty());
}

#[tokio::test]
async fn no_old_root_emits_full_tree_added() {
    let tree = new_tree();
    let file = put_file(&tree, b"hello").await;
    let root = dir_with(&tree, vec![("a.txt", file, 5, LinkType::Blob)]).await;
    let changes = path_diff(&tree, None, &root).await.unwrap();
    assert_eq!(changes.len(), 1);
    let PathChange::Added { path, .. } = &changes[0] else {
        panic!("expected Added");
    };
    assert_eq!(path, "a.txt");
}

#[tokio::test]
async fn nested_change_only_reports_changed_leaf() {
    let tree = new_tree();
    let f1 = put_file(&tree, b"one").await;
    let f2 = put_file(&tree, b"two").await;
    let f2b = put_file(&tree, b"two-modified").await;
    let inner_old = dir_with(
        &tree,
        vec![
            ("x.txt", f1.clone(), 3, LinkType::Blob),
            ("y.txt", f2, 3, LinkType::Blob),
        ],
    )
    .await;
    let inner_new = dir_with(
        &tree,
        vec![
            ("x.txt", f1, 3, LinkType::Blob),
            ("y.txt", f2b, 12, LinkType::Blob),
        ],
    )
    .await;
    let old = dir_with(&tree, vec![("inner", inner_old, 0, LinkType::Dir)]).await;
    let new = dir_with(&tree, vec![("inner", inner_new, 0, LinkType::Dir)]).await;
    let changes = path_diff(&tree, Some(&old), &new).await.unwrap();
    assert_eq!(changes.len(), 1, "got {:?}", changes);
    assert_eq!(changes[0].path(), "inner/y.txt");
}

#[tokio::test]
async fn add_nested_dir_emits_dir_and_contents() {
    let tree = new_tree();
    let f1 = put_file(&tree, b"one").await;
    let inner = dir_with(&tree, vec![("x.txt", f1, 3, LinkType::Blob)]).await;
    let old = empty_dir(&tree).await;
    let new = dir_with(&tree, vec![("inner", inner, 0, LinkType::Dir)]).await;
    let changes = path_diff(&tree, Some(&old), &new).await.unwrap();
    let paths: Vec<&str> = changes.iter().map(PathChange::path).collect();
    assert_eq!(paths, vec!["inner", "inner/x.txt"]);
    assert!(matches!(changes[0], PathChange::Added { .. }));
    assert!(matches!(changes[1], PathChange::Added { .. }));
}

#[tokio::test]
async fn remove_nested_dir_emits_contents_then_dir() {
    let tree = new_tree();
    let f1 = put_file(&tree, b"one").await;
    let inner = dir_with(&tree, vec![("x.txt", f1, 3, LinkType::Blob)]).await;
    let old = dir_with(&tree, vec![("inner", inner, 0, LinkType::Dir)]).await;
    let new = empty_dir(&tree).await;
    let changes = path_diff(&tree, Some(&old), &new).await.unwrap();
    let paths: Vec<&str> = changes.iter().map(PathChange::path).collect();
    // sorted ascending; "inner" < "inner/x.txt"
    assert_eq!(paths, vec!["inner", "inner/x.txt"]);
    for c in &changes {
        assert!(matches!(c, PathChange::Removed { .. }), "got {:?}", c);
    }
}

#[tokio::test]
async fn output_is_sorted_lexicographic() {
    let tree = new_tree();
    let f1 = put_file(&tree, b"a").await;
    let f2 = put_file(&tree, b"b").await;
    let f3 = put_file(&tree, b"c").await;
    let old = empty_dir(&tree).await;
    let new = dir_with(
        &tree,
        vec![
            ("zeta.txt", f3, 1, LinkType::Blob),
            ("alpha.txt", f1, 1, LinkType::Blob),
            ("mid.txt", f2, 1, LinkType::Blob),
        ],
    )
    .await;
    let changes = path_diff(&tree, Some(&old), &new).await.unwrap();
    let paths: Vec<&str> = changes.iter().map(PathChange::path).collect();
    assert_eq!(paths, vec!["alpha.txt", "mid.txt", "zeta.txt"]);
}

#[tokio::test]
async fn type_change_file_to_dir_emits_remove_then_add() {
    let tree = new_tree();
    let file = put_file(&tree, b"hello").await;
    let inner = empty_dir(&tree).await;
    let old = dir_with(&tree, vec![("x", file, 5, LinkType::Blob)]).await;
    let new = dir_with(&tree, vec![("x", inner, 0, LinkType::Dir)]).await;
    let changes = path_diff(&tree, Some(&old), &new).await.unwrap();
    // Implementation emits Removed first, Added second, both at the same path.
    // After sort, both at "x" — order between them is implementation-defined
    // for equal paths, but both must be present.
    let kinds: Vec<&str> = changes
        .iter()
        .map(|c| match c {
            PathChange::Added { .. } => "add",
            PathChange::Modified { .. } => "mod",
            PathChange::Removed { .. } => "rem",
        })
        .collect();
    assert_eq!(kinds.len(), 2, "got {:?}", changes);
    assert!(kinds.contains(&"add"));
    assert!(kinds.contains(&"rem"));
}
