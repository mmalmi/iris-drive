use hashtree_core::{Cid, ObjectKey};
use nostr_sdk::{Event, JsonUtil, Keys};
use serde::Deserialize;

use super::*;

#[derive(Deserialize)]
struct WebFixture {
    ask: Event,
    birth_hash_byte: u8,
    birth_key_byte: u8,
    claim: Event,
    edited_hash_byte: u8,
    edited_key_byte: u8,
    editor_key_byte: u8,
    marker: super::markers::RemovalMarker,
    owner: NostrIdentityId,
    owner_key_byte: u8,
    path: Vec<String>,
    request: Event,
    share: NostrIdentityId,
    stored: Event,
}

fn fixture_key(byte: u8) -> Keys {
    Keys::parse(&hex::encode([byte; 32])).unwrap()
}

fn fixture_cid(hash: u8, key: u8) -> Cid {
    Cid {
        hash: [hash; 32],
        key: Some(ObjectKey::from_hex(&hex::encode([key; 32])).unwrap()),
    }
}

#[test]
fn accepts_web_signed_claims_revision_and_encrypted_messages() {
    let f: WebFixture = serde_json::from_str(include_str!(
        "../../tests/fixtures/shared_file_return_web.json"
    ))
    .unwrap();
    let owner = fixture_key(f.owner_key_byte);
    let editor = fixture_key(f.editor_key_byte);
    assert_eq!(f.claim.pubkey, owner.public_key());
    let claim = read_owner(&f.claim).unwrap();
    let request = read_request(&f.request).unwrap();
    assert_eq!(
        claim.origin,
        revision(&fixture_cid(f.birth_hash_byte, f.birth_key_byte)).unwrap()
    );
    assert_eq!(
        request.revision,
        revision(&fixture_cid(f.edited_hash_byte, f.edited_key_byte)).unwrap()
    );
    assert_eq!(f.request.pubkey, editor.public_key());
    assert_eq!(request.claim, f.claim.id.to_hex());
    assert_eq!(request.share, f.share);
    assert_eq!(request.owner, f.owner);
    assert_eq!(request.item, claim.item);
    assert_eq!(request.path, f.path);
    assert_eq!(request.seen.get(&owner.public_key().to_hex()), Some(&1));
    let ask = read_return_note(&f.ask, &owner).unwrap();
    assert_eq!(ask.phase, ReturnPhase::Request);
    assert_eq!(ask.request, f.request);
    assert!(read_return_note(&f.ask, &editor).is_err());
    let stored = read_return_note(&f.stored, &editor).unwrap();
    assert_eq!(stored.phase, ReturnPhase::Stored);
    assert_eq!(stored.request, f.request);
    let receipt = stored.receipt.unwrap();
    assert_eq!(f.marker.claim, f.claim);
    assert_eq!(f.marker.request, f.request);
    assert_eq!(f.marker.receipt, receipt);
    assert_eq!(f.marker.v, 1);
    wire::verify_receipt(&receipt, &f.request).unwrap();
    assert_eq!(f.stored.pubkey, owner.public_key());
    let native = build_return_note(
        &owner,
        editor.public_key(),
        ReturnPhase::Stored,
        &f.request,
        f.stored.created_at.as_secs(),
    )
    .unwrap();
    assert_eq!(
        native.tags, f.stored.tags,
        "must use the web note coordinate"
    );
    assert_eq!(
        read_return_note(&native, &editor)
            .unwrap()
            .request
            .as_json(),
        f.request.as_json()
    );
}

struct ShareFixture {
    owner_dir: tempfile::TempDir,
    owner: crate::Profile,
    editor: crate::Profile,
    other_writer: crate::Profile,
    folder: SharedFolder,
    root: Cid,
    file: Cid,
}

impl ShareFixture {
    async fn new() -> Self {
        use crate::sharing::{ShareRecipient, ShareRole, create_shared_folder};
        let owner_dir = tempfile::tempdir().unwrap();
        let editor_dir = tempfile::tempdir().unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let owner = crate::Profile::create(owner_dir.path(), None).unwrap();
        let editor = crate::Profile::create(editor_dir.path(), None).unwrap();
        let other_writer = crate::Profile::create(other_dir.path(), None).unwrap();
        let recipient = |profile: &crate::Profile| ShareRecipient {
            profile_id: profile.state.profile_id,
            app_pubkey: profile.state.app_key_pubkey.clone(),
            role: ShareRole::Editor,
            label: None,
            representative_npub_hint: None,
            display_name: None,
        };
        let mut folder = create_shared_folder(
            owner.app_key.keys(),
            owner.state.profile_id,
            "Picnic",
            "Picnic",
            None,
            vec![recipient(&editor), recipient(&other_writer)],
            1,
        )
        .unwrap();
        let config = crate::AppConfig {
            profile: Some(owner.state.clone()),
            drives: vec![crate::Drive::primary(owner.state.root_scope_id())],
            blossom_servers: Vec::new(),
            ..Default::default()
        };
        config
            .save(crate::paths::config_path_in(owner_dir.path()))
            .unwrap();
        let daemon = crate::Daemon::open(owner_dir.path()).unwrap();
        let tree = daemon.tree();
        let empty = tree.put_directory(vec![]).await.unwrap();
        let (file, size) = tree.put(b"exact shared contents").await.unwrap();
        let edited = tree
            .set_entry(&empty, &[], "Plan.txt", &file, size, LinkType::Blob)
            .await
            .unwrap();
        crate::shared_content::write_shared_folder(
            tree,
            &mut folder,
            owner.app_key.keys(),
            edited,
            &empty,
        )
        .await
        .unwrap();
        let root = crate::projection::shared_folder_merged_root(tree, &folder)
            .await
            .unwrap()
            .root_cid;
        let mut config = config;
        config.upsert_shared_folder(folder.clone());
        config
            .save(crate::paths::config_path_in(owner_dir.path()))
            .unwrap();
        Self {
            owner_dir,
            owner,
            editor,
            other_writer,
            folder,
            root,
            file,
        }
    }

    fn open(&self) -> crate::Daemon {
        let dir = self.owner_dir.path();
        let config = crate::AppConfig::load_or_default(crate::paths::config_path_in(dir)).unwrap();
        crate::Daemon::open_with_config(dir, config).unwrap()
    }

    async fn stage_editor_request(&mut self) -> Event {
        let daemon = self.open();
        let tree = daemon.tree();
        let path = vec!["Plan.txt".to_owned()];
        let mut entry = entry_at(tree, &self.root, &path).await.unwrap().unwrap();
        let event = build_request(
            &self.folder,
            &entry,
            path.clone(),
            self.editor.app_key.keys(),
            unix_now().unwrap(),
        )
        .unwrap();
        put_signed_meta(&mut entry, RETURN_META, &event);
        let edited = replace_entry(tree, &self.root, &path, &entry)
            .await
            .unwrap();
        crate::shared_content::write_shared_folder_delta(
            tree,
            &mut self.folder,
            &self.editor.state.app_key_pubkey,
            edited,
            &self.root,
            &self.root,
            &std::collections::BTreeSet::new(),
        )
        .await
        .unwrap();
        self.save_share();
        self.root = crate::projection::shared_folder_merged_root(tree, &self.folder)
            .await
            .unwrap()
            .root_cid;
        event
    }

    fn save_share(&self) {
        let mut config = self.open().config().clone();
        config.upsert_shared_folder(self.folder.clone());
        config
            .save(crate::paths::config_path_in(self.owner_dir.path()))
            .unwrap();
    }
}

#[tokio::test]
async fn production_file_commitment_includes_its_key_suite_byte() {
    let fixture = ShareFixture::new().await;
    let encrypted_key = fixture.file.key.unwrap();
    assert_eq!(encrypted_key.as_bytes().len(), 33);
    assert_eq!(encrypted_key.as_bytes()[0], 1);
    let without_suite = Cid {
        hash: fixture.file.hash,
        key: Some(ObjectKey::from_bytes(&encrypted_key.raw_key()).unwrap()),
    };
    assert_ne!(
        revision(&fixture.file).unwrap(),
        revision(&without_suite).unwrap()
    );
    let entry = entry_at(fixture.open().tree(), &fixture.root, &["Plan.txt".into()])
        .await
        .unwrap()
        .unwrap();
    let (_, claim) = validated_owner(&fixture.folder, &entry).unwrap().unwrap();
    assert_eq!(claim.origin, revision(&fixture.file).unwrap());
}

#[tokio::test]
async fn editor_return_saves_the_exact_file_once_without_overwriting_and_ignores_stale_request() {
    let mut fixture = ShareFixture::new().await;
    let mut daemon = fixture.open();
    let visible = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .unwrap()
        .root_cid;
    let (existing, size) = daemon.tree().put(b"existing private file").await.unwrap();
    let updated = daemon
        .tree()
        .set_entry(&visible, &[], "plan.txt", &existing, size, LinkType::Blob)
        .await
        .unwrap();
    daemon
        .import_visible_root_with_tombstone_base(updated, Some(visible))
        .await
        .unwrap();

    let event = fixture.stage_editor_request().await;
    let mut daemon = fixture.open();
    let pending = current_pending(daemon.tree(), &fixture.folder, &fixture.root, &event)
        .await
        .unwrap();
    assert!(!pending.destination_is_trash);
    let first = storage::stage_private_copy(&mut daemon, &pending, fixture.owner.app_key.keys())
        .await
        .unwrap();
    let again = storage::stage_private_copy(&mut daemon, &pending, fixture.owner.app_key.keys())
        .await
        .unwrap();
    assert_eq!(first.root, again.root);
    let personal = Cid::parse(&first.root.root_cid).unwrap();
    let returned = entry_at(daemon.tree(), &personal, &["Plan (2).txt".into()])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entry_cid(&returned), fixture.file);
    assert_eq!(
        entry_cid(
            &entry_at(daemon.tree(), &personal, &["plan.txt".into()])
                .await
                .unwrap()
                .unwrap()
        ),
        existing
    );
    assert!(
        entry_at(daemon.tree(), &personal, &["Picnic".into()])
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        returned.meta.as_ref().unwrap()[RETURNED_FROM_META],
        event.id.to_hex()
    );
    assert!(!returned.meta.unwrap().contains_key(OWNER_META));

    let (updated, size) = daemon.tree().put(b"a newer version").await.unwrap();
    let edited = daemon
        .tree()
        .set_entry(
            &fixture.root,
            &[],
            "Plan.txt",
            &updated,
            size,
            LinkType::Blob,
        )
        .await
        .unwrap();
    crate::shared_content::write_shared_folder(
        daemon.tree(),
        &mut fixture.folder,
        fixture.other_writer.app_key.keys(),
        edited,
        &fixture.root,
    )
    .await
    .unwrap();
    let current = crate::projection::shared_folder_merged_root(daemon.tree(), &fixture.folder)
        .await
        .unwrap()
        .root_cid;
    assert!(
        current_pending(daemon.tree(), &fixture.folder, &current, &event)
            .await
            .is_err()
    );
    let updated_entry = entry_at(daemon.tree(), &current, &["Plan.txt".into()])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(entry_cid(&updated_entry), updated);
    assert!(
        wire::signed_meta_at(&updated_entry, RETURN_META)
            .unwrap()
            .is_none()
    );
    assert_eq!(
        validated_owner(&fixture.folder, &updated_entry)
            .unwrap()
            .unwrap()
            .1
            .item,
        pending.request.item
    );
}

fn stored_receipt(owner: &Keys, event: &Event) -> Event {
    let note = build_return_note(
        owner,
        owner.public_key(),
        ReturnPhase::Stored,
        event,
        unix_now().unwrap(),
    )
    .unwrap();
    read_return_note(&note, owner).unwrap().receipt.unwrap()
}

#[tokio::test]
async fn owners_delete_through_private_trash_and_repeat_the_same_pending_request() {
    let fixture = ShareFixture::new().await;
    let config_dir = fixture.owner_dir.path();
    let lock = crate::config_lock::ConfigMutationLock::acquire(config_dir)
        .await
        .unwrap();
    let daemon = fixture.open();
    let id = queue_shared_file_return(&daemon, "Picnic/Plan.txt", &lock)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        queue_shared_file_return(&daemon, "Picnic/Plan.txt", &lock)
            .await
            .unwrap(),
        Some(id.clone())
    );
    assert_eq!(
        queue_shared_file_return(&daemon, "Picnic", &lock)
            .await
            .unwrap(),
        None
    );
    let mut daemon = fixture.open();
    let folder = daemon
        .config()
        .shared_folder(fixture.folder.share_id)
        .unwrap()
        .clone();
    let root = crate::projection::shared_folder_merged_root(daemon.tree(), &folder)
        .await
        .unwrap()
        .root_cid;
    let entry = entry_at(daemon.tree(), &root, &["Plan.txt".into()])
        .await
        .unwrap()
        .unwrap();
    let event = wire::signed_meta_at(&entry, RETURN_META).unwrap().unwrap();
    assert_eq!(event.id.to_hex(), id);
    let pending = current_pending(daemon.tree(), &folder, &root, &event)
        .await
        .unwrap();
    assert!(pending.destination_is_trash);
    let first = storage::stage_private_copy(&mut daemon, &pending, fixture.owner.app_key.keys())
        .await
        .unwrap();
    let again = storage::stage_private_copy(&mut daemon, &pending, fixture.owner.app_key.keys())
        .await
        .unwrap();
    assert_eq!(first.root, again.root);
    assert_eq!(first.drive_id(), crate::private_trash::DRIVE_ID);
    let items = crate::private_trash::list(config_dir, i64::try_from(unix_now().unwrap()).unwrap())
        .await
        .unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].id, pending.request.nonce);
    assert_eq!(items[0].path, ["Plan.txt"]);
    sync::commit_removal(
        &daemon,
        &folder,
        &root,
        &pending,
        stored_receipt(fixture.owner.app_key.keys(), &event),
        &fixture.owner.state.app_key_pubkey,
    )
    .await
    .unwrap();
    let daemon = fixture.open();
    let folder = daemon
        .config()
        .shared_folder(fixture.folder.share_id)
        .unwrap();
    let root = crate::projection::shared_folder_merged_root(daemon.tree(), folder)
        .await
        .unwrap()
        .root_cid;
    assert!(
        entry_at(daemon.tree(), &root, &["Plan.txt".into()])
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_concurrent_unseen_write_survives_the_removal_of_an_acknowledged_revision() {
    let mut fixture = ShareFixture::new().await;
    let fork = fixture.folder.clone();
    let fork_root = fixture.root.clone();
    let event = fixture.stage_editor_request().await;
    let daemon = fixture.open();
    let pending = current_pending(daemon.tree(), &fixture.folder, &fixture.root, &event)
        .await
        .unwrap();
    sync::commit_removal(
        &daemon,
        &fixture.folder,
        &fixture.root,
        &pending,
        stored_receipt(fixture.owner.app_key.keys(), &event),
        &fixture.owner.state.app_key_pubkey,
    )
    .await
    .unwrap();
    let mut concurrent = fork;
    let (new, size) = daemon
        .tree()
        .put(b"unseen concurrent version")
        .await
        .unwrap();
    let edited = daemon
        .tree()
        .set_entry(&fork_root, &[], "Plan.txt", &new, size, LinkType::Blob)
        .await
        .unwrap();
    crate::shared_content::write_shared_folder(
        daemon.tree(),
        &mut concurrent,
        fixture.other_writer.app_key.keys(),
        edited,
        &fork_root,
    )
    .await
    .unwrap();
    let updated = fixture.open();
    let mut combined = updated
        .config()
        .shared_folder(fixture.folder.share_id)
        .unwrap()
        .clone();
    let other_key = &fixture.other_writer.state.app_key_pubkey;
    combined.app_key_roots.insert(
        other_key.clone(),
        concurrent.app_key_roots[other_key].clone(),
    );
    // Make the unseen version older by wall clock to exercise the dangerous
    // ordering while retaining its authentic, independent causal history.
    combined
        .app_key_roots
        .get_mut(other_key)
        .unwrap()
        .published_at = 1;
    let root = crate::projection::shared_folder_merged_root(daemon.tree(), &combined)
        .await
        .unwrap()
        .root_cid;
    assert_eq!(
        entry_cid(
            &entry_at(daemon.tree(), &root, &["Plan.txt".into()])
                .await
                .unwrap()
                .unwrap()
        ),
        new
    );
}

#[tokio::test]
async fn a_stored_receipt_disposes_of_observed_revisions_and_keeps_its_proof_on_later_writes() {
    let mut fixture = ShareFixture::new().await;
    let daemon = fixture.open();
    let original_root = fixture.root.clone();
    let (edited_file, size) = daemon.tree().put(b"new visible revision").await.unwrap();
    let edited = daemon
        .tree()
        .set_entry(
            &fixture.root,
            &[],
            "Plan.txt",
            &edited_file,
            size,
            LinkType::Blob,
        )
        .await
        .unwrap();
    crate::shared_content::write_shared_folder(
        daemon.tree(),
        &mut fixture.folder,
        fixture.editor.app_key.keys(),
        edited,
        &fixture.root,
    )
    .await
    .unwrap();
    fixture.root = crate::projection::shared_folder_merged_root(daemon.tree(), &fixture.folder)
        .await
        .unwrap()
        .root_cid;
    fixture.save_share();

    let event = fixture.stage_editor_request().await;
    let mut daemon = fixture.open();
    let pending = current_pending(daemon.tree(), &fixture.folder, &fixture.root, &event)
        .await
        .unwrap();
    let owner_key = &fixture.owner.state.app_key_pubkey;
    assert_eq!(pending.request.seen.get(owner_key), Some(&1));
    assert_eq!(entry_cid(&pending.entry), edited_file);
    assert_eq!(
        entry_cid(
            &entry_at(daemon.tree(), &original_root, &["Plan.txt".into()])
                .await
                .unwrap()
                .unwrap()
        ),
        fixture.file
    );
    storage::stage_private_copy(&mut daemon, &pending, fixture.owner.app_key.keys())
        .await
        .unwrap();
    let receipt = stored_receipt(fixture.owner.app_key.keys(), &event);
    sync::commit_removal(
        &daemon,
        &fixture.folder,
        &fixture.root,
        &pending,
        receipt.clone(),
        &fixture.other_writer.state.app_key_pubkey,
    )
    .await
    .unwrap();
    let daemon = fixture.open();
    let mut folder = daemon
        .config()
        .shared_folder(fixture.folder.share_id)
        .unwrap()
        .clone();
    let finalizer_key = &fixture.other_writer.state.app_key_pubkey;
    let finalizer_root = Cid::parse(&folder.app_key_roots[finalizer_key].root_cid).unwrap();
    let (_, tombstones) = crate::merge::walk_app_key_tree(daemon.tree(), &finalizer_root)
        .await
        .unwrap();
    assert!(
        tombstones.is_empty(),
        "receipt removal must never emit a legacy tombstone"
    );
    let root = crate::projection::shared_folder_merged_root(daemon.tree(), &folder)
        .await
        .unwrap()
        .root_cid;
    assert!(
        entry_at(daemon.tree(), &root, &["Plan.txt".into()])
            .await
            .unwrap()
            .is_none()
    );

    // A different writer republishes its view. The exact receipt stays
    // available without depending on the finalizing writer's latest root.
    let edited = daemon
        .tree()
        .set_entry(
            &root,
            &[],
            "Another.txt",
            &edited_file,
            size,
            LinkType::Blob,
        )
        .await
        .unwrap();
    crate::shared_content::write_shared_folder(
        daemon.tree(),
        &mut folder,
        fixture.owner.app_key.keys(),
        edited,
        &root,
    )
    .await
    .unwrap();
    let later = Cid::parse(&folder.app_key_roots[owner_key].root_cid).unwrap();
    let path = vec![
        crate::merge::META_DIR.into(),
        markers::DIR.into(),
        format!("{}.json", event.id.to_hex()),
    ];
    let record = entry_at(daemon.tree(), &later, &path)
        .await
        .unwrap()
        .unwrap();
    let bytes = daemon
        .tree()
        .get(&entry_cid(&record), None)
        .await
        .unwrap()
        .unwrap();
    let marker: super::markers::RemovalMarker = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(marker.receipt, receipt);
}
