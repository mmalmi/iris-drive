use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Duration,
};

use anyhow::{Context, Result, ensure};
use hashtree_core::Cid;
use nostr_sdk::{Client, Event, Keys, PublicKey};
use serde::{Deserialize, Serialize};

use super::{
    RETURN_META, ReturnPhase, active_profile, build_return_note, current_pending, entry_at,
    read_request, read_return_note, replace_entry, return_note_filter,
    storage::{RemoteRead, stage_private_copy},
    unix_now, wire,
};
use crate::{
    AppConfig, AppKey, Daemon, NostrIdentityId, PRIMARY_DRIVE_ID, SharedFolder,
    config_lock::ConfigMutationLock,
    paths::{config_path_in, key_path_in, touch_provider_root_signal_in},
    relay_sync,
    shared_content::{
        SharedFolderChange, share_root_for_publish, write_shared_folder_change,
        write_shared_folder_delta,
    },
    shared_folder_app_key_can_write_roots,
};

const OUTBOX: &str = "shared-file-returns.json";

#[derive(Default, Serialize, Deserialize)]
struct Outbox {
    requests: BTreeMap<String, Event>,
    publish: BTreeSet<NostrIdentityId>,
}

impl Outbox {
    fn load(dir: &Path) -> Result<Self> {
        match std::fs::read(dir.join(OUTBOX)) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.into()),
        }
    }

    fn save(&self, dir: &Path) -> Result<()> {
        crate::atomic_file::atomic_write_private(&dir.join(OUTBOX), &serde_json::to_vec(self)?)?;
        Ok(())
    }
}

/// The caller holds the provider's config transaction. A pending ID means
/// the provider must keep the entry visible until the share root removes it.
pub async fn queue_shared_file_return(
    daemon: &Daemon,
    provider_path: &str,
    _lock: &ConfigMutationLock,
) -> Result<Option<String>> {
    let dir = daemon.config_dir();
    let mut config = AppConfig::load_or_default(config_path_in(dir))?;
    if config.shared_folders.is_empty() {
        return Ok(None);
    }
    let tree = daemon.tree();
    let view = crate::primary_merged_view(tree, &config).await?;
    let Some((mount, relative)) = view
        .shared_mounts
        .iter()
        .filter_map(|mount| {
            let relative = provider_path.strip_prefix(&mount.path)?.strip_prefix('/')?;
            Some((mount, relative))
        })
        .max_by_key(|(mount, _)| mount.path.len())
    else {
        return Ok(None);
    };
    let path: Vec<String> = mount
        .target_path
        .split('/')
        .filter(|part| !part.is_empty())
        .chain(relative.split('/'))
        .map(str::to_owned)
        .collect();
    let keys = AppKey::load(key_path_in(dir))?;
    let keys = keys.keys();
    let mut folder = config
        .shared_folder(mount.share_id)
        .context("Shared folder disappeared")?
        .clone();
    let root = current_root(daemon, &folder).await?;
    let entry = entry_at(tree, &root, &path)
        .await?
        .context("Shared file is unavailable")?;
    let (replacement, event) =
        super::mark_pending(tree, &folder, &root, entry.clone(), path.clone(), keys).await?;
    let id = event.id.to_hex();
    if event.pubkey == keys.public_key() {
        remember_requests(dir, vec![event])?;
    }
    if replacement.meta != entry.meta {
        let edited = replace_entry(tree, &root, &path, &replacement).await?;
        write_shared_folder_delta(
            tree,
            &mut folder,
            &keys.public_key().to_hex(),
            edited,
            &root,
            &root,
            &BTreeSet::new(),
        )
        .await?;
    }
    config.upsert_shared_folder(folder);
    config.save(config_path_in(dir))?;
    touch_provider_root_signal_in(dir)?;
    Ok(Some(id))
}

/// A config transaction records outgoing notes before installing the new root.
pub(crate) fn remember_requests(dir: &Path, events: Vec<Event>) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let mut outbox = Outbox::load(dir)?;
    outbox
        .requests
        .extend(events.into_iter().map(|event| (event.id.to_hex(), event)));
    outbox.save(dir)
}

pub async fn sync_shared_file_returns(
    client: &Client,
    dir: &Path,
    timeout: Duration,
) -> Result<()> {
    let config = AppConfig::load_or_default(config_path_in(dir))?;
    if config.shared_folders.is_empty() {
        return Ok(());
    }
    let key = AppKey::load(key_path_in(dir))?;
    let keys = key.keys();
    let outbox = Outbox::load(dir)?;
    for share in outbox.publish {
        if let Err(error) = flush_share(client, dir, keys, share).await {
            tracing::debug!(%error, "shared file removal publication will retry");
        }
    }
    for event in outbox.requests.into_values() {
        if let Err(error) = deliver_request(client, dir, keys, &event).await {
            tracing::debug!(%error, "shared file return request will retry");
        }
    }
    let filter = return_note_filter(keys.public_key());
    for event in relay_sync::fetch_events(client, vec![filter], timeout).await? {
        if let Err(error) = apply_note(client, dir, keys, &event).await {
            tracing::debug!(%error, "shared file return note did not apply");
        }
    }
    Ok(())
}

fn open(dir: &Path) -> Result<Daemon> {
    let config = AppConfig::load_or_default(config_path_in(dir))?;
    Ok(Daemon::open_with_config(dir, config)?)
}

async fn current_root(daemon: &Daemon, folder: &SharedFolder) -> Result<Cid> {
    // A missing directory in a known writer root must never look like an
    // absent competing revision when finalizing a removal.
    for writer in crate::shared_folder_authorized_writer_pubkeys(folder) {
        if let Some(root) = folder.app_key_roots.get(&writer) {
            let cid = Cid::parse(&root.root_cid)?;
            crate::indexer::required_visible_entries(daemon.tree(), &cid).await?;
        }
    }
    Ok(
        crate::projection::shared_folder_merged_root(daemon.tree(), folder)
            .await?
            .root_cid,
    )
}

async fn deliver_request(client: &Client, dir: &Path, keys: &Keys, event: &Event) -> Result<()> {
    let request = read_request(event)?;
    let lock = ConfigMutationLock::acquire(dir).await?;
    let daemon = open(dir)?;
    let folder = daemon
        .config()
        .shared_folder(request.share)
        .context("Shared folder disappeared")?;
    ensure!(
        event.pubkey == keys.public_key(),
        "Another device authored this return request"
    );
    let root = current_root(&daemon, folder).await?;
    let entry = entry_at(daemon.tree(), &root, &request.path).await?;
    if entry
        .as_ref()
        .map(|entry| wire::signed_meta_at(entry, RETURN_META))
        .transpose()?
        .flatten()
        .as_ref()
        != Some(event)
    {
        return settle_request(dir, event);
    }
    current_pending(daemon.tree(), folder, &root, event).await?;
    let recipients = active_recipients(folder, |key| {
        active_profile(folder, key) == Some(request.owner)
    })?;
    ensure!(
        !recipients.is_empty(),
        "The file owner has no active share devices"
    );
    let publication = share_publication(&daemon, folder, keys)?;
    drop(lock);
    publication.send(client, &daemon, keys).await?;
    send_notes(client, keys, recipients, ReturnPhase::Request, event).await?;
    let _lock = ConfigMutationLock::acquire(dir).await?;
    settle_request(dir, event)
}

fn settle_request(dir: &Path, event: &Event) -> Result<()> {
    let mut outbox = Outbox::load(dir)?;
    if outbox.requests.remove(&event.id.to_hex()).is_some() {
        outbox.save(dir)?;
    }
    Ok(())
}

async fn apply_note(client: &Client, dir: &Path, keys: &Keys, event: &Event) -> Result<()> {
    let note = read_return_note(event, keys)?;
    let request = read_request(&note.request)?;
    let lock = ConfigMutationLock::acquire(dir).await?;
    let mut daemon = open(dir)?;
    let folder = daemon
        .config()
        .shared_folder(request.share)
        .context("Shared folder disappeared")?;
    let root = current_root(&daemon, folder).await?;
    let pending = current_pending(daemon.tree(), folder, &root, &note.request).await?;
    let this_key = keys.public_key().to_hex();
    match note.phase {
        ReturnPhase::Request => {
            ensure!(
                active_profile(folder, &this_key) == Some(request.owner),
                "This share device does not represent the file owner"
            );
            let copy = stage_private_copy(&mut daemon, &pending, keys).await?;
            let publication = Publication {
                cid: Cid::parse(&copy.root.root_cid)?,
                event: crate::nostr_events::build_drive_root_publish_event(
                    dir,
                    keys,
                    &copy.scope,
                    copy.drive_id(),
                    &copy.root,
                    &copy.recipients,
                )?,
            };
            drop(lock);
            let blossom = publication.send(client, &daemon, keys).await?;
            publication.confirm(client, keys).await?;
            let remote = hashtree_core::HashTree::new(hashtree_core::HashTreeConfig::new(
                std::sync::Arc::new(RemoteRead(blossom)),
            ));
            copy.verify(&remote, &publication.cid, &pending).await?;

            let lock = ConfigMutationLock::acquire(dir).await?;
            let daemon = open(dir)?;
            let state = daemon
                .config()
                .profile
                .as_ref()
                .context("Profile required")?;
            let folder = daemon
                .config()
                .shared_folder(request.share)
                .context("Shared folder disappeared")?;
            ensure!(
                state.profile_id == request.owner
                    && state.app_key_pubkey == this_key
                    && state.profile_projection().can_write_roots(&this_key)
                    && active_profile(folder, &this_key) == Some(request.owner),
                "Owner authorization changed while saving the file"
            );
            let root = current_root(&daemon, folder).await?;
            current_pending(daemon.tree(), folder, &root, &note.request).await?;
            let drive = daemon
                .config()
                .drive(copy.drive_id())
                .context("Private owner drive is missing")?;
            let live = crate::publishable_root::publishable_app_key_root(dir, drive, state)
                .await?
                .context("Owner copy is no longer published")?;
            ensure!(
                live.root_cid == copy.root.root_cid
                    && live.app_key_seq == copy.root.app_key_seq
                    && copy.recipients
                        == crate::recovery_wraps::drive_root_key_recipient_pubkeys(state, drive),
                "The owner's publishable root or recipient roster changed"
            );
            let recipients = active_recipients(folder, |key| {
                shared_folder_app_key_can_write_roots(folder, key)
            })?;
            drop(lock);
            send_notes(client, keys, recipients, ReturnPhase::Stored, &note.request).await?;
        }
        ReturnPhase::Stored => {
            ensure!(
                active_profile(folder, &event.pubkey.to_hex()) == Some(request.owner)
                    && active_profile(folder, &this_key).is_some()
                    && shared_folder_app_key_can_write_roots(folder, &this_key),
                "Only an active owner receipt and authorized writer can remove this file"
            );
            let receipt = note.receipt.context("Signed owner receipt is missing")?;
            commit_removal(&daemon, folder, &root, &pending, receipt, &this_key).await?;
            drop(lock);
            flush_share(client, dir, keys, request.share).await?;
        }
    }
    Ok(())
}

pub(super) async fn commit_removal(
    daemon: &Daemon,
    folder: &SharedFolder,
    root: &Cid,
    pending: &super::PendingReturn,
    receipt: Event,
    app_key: &str,
) -> Result<()> {
    let request = &pending.request;
    let dir = daemon.config_dir();
    let mut folder = folder.clone();
    let (name, parent) = request
        .path
        .split_last()
        .context("Shared file path required")?;
    let parent: Vec<&str> = parent.iter().map(String::as_str).collect();
    let edited = daemon.tree().remove_entry(root, &parent, name).await?;
    let marker = super::markers::RemovalMarker {
        v: 1,
        claim: pending.claim.clone(),
        request: pending.event.clone(),
        receipt,
    };
    write_shared_folder_change(
        daemon.tree(),
        &mut folder,
        app_key,
        SharedFolderChange {
            edited,
            base: root,
            current: root,
            allowed_removals: &BTreeSet::from([request.path.join("/")]),
            marker: Some(&marker),
        },
    )
    .await?;
    let mut outbox = Outbox::load(dir)?;
    outbox.publish.insert(request.share);
    outbox.save(dir)?;
    let mut config = daemon.config().clone();
    config.upsert_shared_folder(folder);
    config.save(config_path_in(dir))?;
    touch_provider_root_signal_in(dir)?;
    Ok(())
}

fn active_recipients(
    folder: &SharedFolder,
    is_recipient: impl Fn(&str) -> bool,
) -> Result<Vec<PublicKey>> {
    folder
        .projection()
        .active_facets
        .keys()
        .filter(|key| active_profile(folder, key).is_some() && is_recipient(key))
        .map(|key| PublicKey::parse(key).map_err(Into::into))
        .collect()
}

async fn send_notes(
    client: &Client,
    keys: &Keys,
    recipients: Vec<PublicKey>,
    phase: ReturnPhase,
    request: &Event,
) -> Result<()> {
    let now = unix_now()?;
    for recipient in recipients {
        let event = build_return_note(keys, recipient, phase, request, now)?;
        let output = client.send_event(&event).await?;
        ensure!(
            !output.success.is_empty(),
            "No relay accepted a shared file return note"
        );
    }
    Ok(())
}

struct Publication {
    cid: Cid,
    event: Event,
}

impl Publication {
    async fn send(
        &self,
        client: &Client,
        daemon: &Daemon,
        keys: &Keys,
    ) -> Result<hashtree_blossom::BlossomClient> {
        let blossom = crate::blossom_sync_client(keys.clone(), &daemon.config().blossom_servers);
        crate::blossom_sync::upload_tree(daemon.tree(), &self.cid, &blossom).await?;
        relay_sync::publish_prebuilt_drive_root(client, &self.event).await?;
        Ok(blossom)
    }

    async fn confirm(&self, client: &Client, keys: &Keys) -> Result<()> {
        let expected = crate::nostr_events::parse_drive_root_event_preview(&self.event)?;
        let coordinate =
            crate::nostr_events::drive_root_d_tag(&expected.root_scope_id, &expected.drive_id);
        let filter = nostr_sdk::Filter::new()
            .kind(self.event.kind)
            .author(keys.public_key())
            .identifier(coordinate)
            .limit(4);
        let events = relay_sync::fetch_events(client, vec![filter], Duration::from_secs(5)).await?;
        ensure!(
            events.iter().any(|event| {
                crate::nostr_events::parse_drive_root_event_for_device(event, keys).is_ok_and(
                    |(author, scope, drive, root)| {
                        author == expected.app_key_pubkey_hex
                            && scope == expected.root_scope_id
                            && drive == expected.drive_id
                            && root.app_key_seq == expected.app_key_seq
                            && Cid::parse(&root.root_cid).is_ok_and(|cid| cid == self.cid)
                    },
                )
            }),
            "No relay returned the exact signed private owner root"
        );
        Ok(())
    }
}

fn share_publication(daemon: &Daemon, folder: &SharedFolder, keys: &Keys) -> Result<Publication> {
    let local = folder
        .app_key_roots
        .get(&keys.public_key().to_hex())
        .context("Shared folder has no local root to send")?;
    let root = share_root_for_publish(folder, local);
    Ok(Publication {
        cid: Cid::parse(&root.root_cid)?,
        event: crate::nostr_events::build_drive_root_publish_event(
            daemon.config_dir(),
            keys,
            &folder.share_id.to_string(),
            PRIMARY_DRIVE_ID,
            &root,
            &crate::shared_folder_key_recipient_pubkeys(folder),
        )?,
    })
}

async fn flush_share(
    client: &Client,
    dir: &Path,
    keys: &Keys,
    share: NostrIdentityId,
) -> Result<()> {
    let lock = ConfigMutationLock::acquire(dir).await?;
    let daemon = open(dir)?;
    let folder = daemon
        .config()
        .shared_folder(share)
        .context("Shared folder disappeared")?;
    ensure!(
        shared_folder_app_key_can_write_roots(folder, &keys.public_key().to_hex()),
        "This device can no longer publish the share"
    );
    let publication = share_publication(&daemon, folder, keys)?;
    drop(lock);
    publication.send(client, &daemon, keys).await?;
    let _lock = ConfigMutationLock::acquire(dir).await?;
    let current = AppConfig::load_or_default(config_path_in(dir))?;
    if current
        .shared_folder(share)
        .is_some_and(|current| current == folder)
    {
        let mut outbox = Outbox::load(dir)?;
        outbox.publish.remove(&share);
        outbox.save(dir)?;
    }
    Ok(())
}
