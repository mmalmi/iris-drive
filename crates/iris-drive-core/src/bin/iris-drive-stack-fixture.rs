//! Process fixture for cross-repository Iris Stack integration tests.
//!
//! This binary deliberately contains no transport or blob-routing logic. It
//! composes the same [`FipsBlockSync`] used by Iris Drive and exposes a tiny
//! line-oriented control surface so an external lab can exercise real process
//! lifecycle without reaching into Drive's private test modules.

use std::io::Write as _;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use hashtree_core::{Cid, MemoryStore, Store};
use iris_drive_core::{
    AppActorEntry, AppConfig, AppKey, AppKeyAuthorizationState, AppKeysProjection, FipsBlockSync,
    NostrIdentityId, ProfileState,
};
use nostr_sdk::PublicKey;
use nostr_sdk::nips::nip19::FromBech32;
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().context("missing fixture mode")?;
    if mode == "identity" {
        let key_path = PathBuf::from(args.next().context("missing fixture key path")?);
        if args.next().is_some() {
            bail!("usage: iris-drive-stack-fixture identity <key-path>");
        }
        let device = AppKey::load_or_generate(key_path).context("load fixture AppKey")?;
        let profile_id = NostrIdentityId::new_v4();
        emit(&json!({
            "event": "identity",
            "npub": device.pubkey_bech32(),
            "profile_id": profile_id.to_string(),
            "discovery_scope": format!("iris-drive:{profile_id}"),
        }))?;
        return Ok(());
    }
    if mode != "run" {
        bail!("fixture mode must be identity or run");
    }
    let remote_npub = args.next().context("missing remote htree npub")?;
    let key_path = PathBuf::from(args.next().context("missing fixture key path")?);
    let profile_id = args
        .next()
        .context("missing fixture profile id")?
        .parse::<NostrIdentityId>()
        .context("parse fixture profile id")?;
    if args.next().is_some() {
        bail!("usage: iris-drive-stack-fixture run <remote-htree-npub> <key-path> <profile-id>");
    }

    let remote_pubkey =
        PublicKey::from_bech32(&remote_npub).context("remote htree identity must be an npub")?;
    let device = AppKey::load_or_generate(key_path).context("load fixture AppKey")?;
    let config = fixture_config(&device, remote_pubkey.to_hex(), profile_id);
    let local = Arc::new(MemoryStore::new());
    let sync = FipsBlockSync::start(&device, local.clone(), &config)
        .await
        .context("start Iris Drive FIPS block sync")?;

    emit(&json!({
        "event": "ready",
        "npub": sync.endpoint_npub(),
        "remote_npub": remote_npub,
        "discovery_scope": sync.discovery_scope(),
    }))?;

    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    while let Some(line) = lines.next_line().await? {
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("fetch") => {
                let cid = Cid::parse(fields.next().context("fetch requires a CID")?)
                    .context("parse requested CID")?;
                if fields.next().is_some() {
                    bail!("fetch accepts exactly one CID");
                }
                let report = sync
                    .download_tree(&cid)
                    .await
                    .with_context(|| format!("Iris Drive fetch {cid}"))?;
                let cached = local
                    .has(&cid.hash)
                    .await
                    .context("check Drive root cache")?;
                let remote = sync
                    .fips_peer_statuses()
                    .await
                    .into_iter()
                    .find(|peer| peer.npub == remote_npub);
                let remote_connected = remote
                    .as_ref()
                    .is_some_and(|peer| peer.transport_addr.is_some());
                emit(&json!({
                    "event": "fetch",
                    "cid": cid.to_string(),
                    "fetched": report.fetched,
                    "already_local": report.already_local,
                    "root_cached": cached,
                    "remote_connected": remote_connected,
                    "remote_transport": remote.as_ref().and_then(|peer| peer.transport_type.clone()),
                    "remote_addr": remote.as_ref().and_then(|peer| peer.transport_addr.clone()),
                }))?;
            }
            Some("status") => {
                let remote = sync
                    .fips_peer_statuses()
                    .await
                    .into_iter()
                    .find(|peer| peer.npub == remote_npub);
                let remote_connected = remote
                    .as_ref()
                    .is_some_and(|peer| peer.transport_addr.is_some());
                emit(&json!({
                    "event": "status",
                    "remote_connected": remote_connected,
                    "remote_transport": remote.as_ref().and_then(|peer| peer.transport_type.clone()),
                    "remote_addr": remote.as_ref().and_then(|peer| peer.transport_addr.clone()),
                    "same_host_blob_providers": sync.same_host_blob_provider_ids(),
                }))?;
            }
            Some("stop") | None => break,
            Some(command) => bail!("unknown fixture command: {command}"),
        }
    }

    sync.shutdown().await.context("stop Iris Drive FIPS sync")?;
    emit(&json!({ "event": "stopped" }))?;
    Ok(())
}

fn fixture_config(
    device: &AppKey,
    remote_pubkey: String,
    profile_id: NostrIdentityId,
) -> AppConfig {
    let local_pubkey = device.pubkey_hex();
    AppConfig {
        profile: Some(ProfileState {
            profile_id,
            app_key_pubkey: local_pubkey.clone(),
            profile_roster_ops: Vec::new(),
            app_key_link_secret: "iris-stack-fixture".to_string(),
            authorization_state: AppKeyAuthorizationState::Authorized,
            app_key_label: Some("iris-stack-drive".to_string()),
            app_keys: Some(AppKeysProjection {
                profile_id: profile_id.to_string(),
                signed_by_pubkey: Some(local_pubkey.clone()),
                created_at: 1,
                app_actors: vec![
                    AppActorEntry::admin(local_pubkey, 1, Some("iris-stack-drive".to_string())),
                    AppActorEntry::member(
                        remote_pubkey,
                        1,
                        Some("iris-stack-remote-htree".to_string()),
                    ),
                ],
                dck_generation: 0,
                wrapped_dck: std::collections::BTreeMap::default(),
            }),
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
            pending_device_approval_receipts: Vec::new(),
        }),
        relays: Vec::new(),
        blossom_servers: Vec::new(),
        ..AppConfig::default()
    }
}

fn emit(value: &serde_json::Value) -> Result<()> {
    println!("{value}");
    std::io::stdout().flush().context("flush fixture output")
}
