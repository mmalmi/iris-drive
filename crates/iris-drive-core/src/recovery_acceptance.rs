//! The acceptance a recovery phrase signs for its own account.
//!
//! Signing in with a phrase on the web finds the account through events the
//! phrase's own key signed. A native device adds a phrase with an
//! `add_facet` op signed by its `AppKey`, so the phrase key also signs this
//! acceptance while the words are still known: the same event the web signs
//! when it adds a phrase. Only the signed event is kept, in a small queue in
//! the config directory that is published with the roster ops.

use std::path::{Path, PathBuf};

use nostr_sdk::{Client, Event, EventBuilder, JsonUtil, Keys, Tag};

use crate::relay_sync::RelayError;
use crate::{
    NostrIdentityError, NostrIdentityId, NostrIdentityKeyPurpose, NostrIdentityRosterOp,
    ProfileState, SignedNostrIdentityRosterOp, parse_nostr_identity_facet_acceptance_event,
};

const QUEUE_FILE: &str = "facet-acceptances.json";

fn queue_path(config_dir: &Path) -> PathBuf {
    config_dir.join(QUEUE_FILE)
}

/// The op that added `recovery_pubkey` to the account, if it is in `state`.
#[must_use]
pub fn recovery_phrase_add_op<'a>(
    state: &'a ProfileState,
    recovery_pubkey: &str,
) -> Option<&'a SignedNostrIdentityRosterOp> {
    state.profile_roster_ops.iter().rev().find(|op| {
        matches!(
            &op.content.op,
            NostrIdentityRosterOp::AddFacet { facet }
                if facet.pubkey == recovery_pubkey
                    && facet.has_purpose(NostrIdentityKeyPurpose::RecoveryPhrase)
        )
    })
}

/// Sign the acceptance for the recovery phrase that `add_op` added.
///
/// The same event the web's `createRecoveryPhraseRosterOps` signs: purposes
/// `["recovery_phrase"]`, the add op's id, accepted one second after it, and
/// a client nonce ending in `:accept`.
pub fn sign_recovery_phrase_acceptance(
    recovery_keys: &Keys,
    profile_id: NostrIdentityId,
    add_op: &SignedNostrIdentityRosterOp,
) -> Result<Event, NostrIdentityError> {
    sign_acceptance_with_nonce(
        recovery_keys,
        profile_id,
        &add_op.op_id,
        add_op.content.created_at + 1,
        &format!("{}:accept", uuid::Uuid::new_v4()),
    )
}

fn sign_acceptance_with_nonce(
    recovery_keys: &Keys,
    profile_id: NostrIdentityId,
    roster_op_id: &str,
    accepted_at: i64,
    client_nonce: &str,
) -> Result<Event, NostrIdentityError> {
    let accepted_at = u64::try_from(accepted_at)
        .map_err(|_| NostrIdentityError::Event("accepted_at before 1970".into()))?;
    let event = nostr_identity::build_identity_key_acceptance_event(
        recovery_keys,
        profile_id.as_uuid(),
        [nostr_identity::IDENTITY_PURPOSE_RECOVERY.to_owned()],
        Some(roster_op_id.to_owned()),
        client_nonce,
        accepted_at,
    )
    .map_err(|error| NostrIdentityError::Event(error.to_string()))?;
    let event = with_signer_index_tag(&event, recovery_keys).map_err(NostrIdentityError::Event)?;
    parse_nostr_identity_facet_acceptance_event(&event)?;
    Ok(event)
}

/// nostr-sdk drops `p` tags that name the event's own author unless told
/// not to, so the fact builder loses the index tag for the signer's key. The
/// web keeps it, and its phrase sign-in looks for events that tag their own
/// signer. Put it back in its sorted place among the `p` index tags, which
/// sit between the `i`/`e` tags and the facts, and sign again.
fn with_signer_index_tag(built: &Event, keys: &Keys) -> Result<Event, String> {
    let signer_tag = vec!["p".to_owned(), keys.public_key().to_hex()];
    let mut tags = built
        .tags
        .iter()
        .map(|tag| tag.as_slice().to_vec())
        .collect::<Vec<_>>();
    if !tags.contains(&signer_tag) {
        let p_start = tags
            .iter()
            .position(|tag| tag[0] != "i" && tag[0] != "e")
            .unwrap_or(tags.len());
        let p_len = tags[p_start..]
            .iter()
            .take_while(|tag| tag[0] == "p")
            .count();
        let at = p_start + tags[p_start..p_start + p_len].partition_point(|tag| *tag < signer_tag);
        tags.insert(at, signer_tag);
    }
    let tags = tags
        .into_iter()
        .map(Tag::parse)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    EventBuilder::new(built.kind, built.content.clone())
        .tags(tags)
        .allow_self_tagging()
        .custom_created_at(built.created_at)
        .sign_with_keys(keys)
        .map_err(|error| error.to_string())
}

/// Keep a signed acceptance until it is published.
pub fn queue_facet_acceptance(config_dir: &Path, event: &Event) -> std::io::Result<()> {
    let mut queued = queued_facet_acceptance_json(config_dir);
    let json = event.as_json();
    if !queued.contains(&json) {
        queued.push(json);
    }
    let bytes = serde_json::to_vec_pretty(&queued).map_err(std::io::Error::other)?;
    crate::atomic_file::atomic_write(&queue_path(config_dir), &bytes)
}

fn queued_facet_acceptance_json(config_dir: &Path) -> Vec<String> {
    std::fs::read(queue_path(config_dir))
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Queued acceptances that still verify.
#[must_use]
pub fn queued_facet_acceptances(config_dir: &Path) -> Vec<Event> {
    queued_facet_acceptance_json(config_dir)
        .iter()
        .filter_map(|json| Event::from_json(json).ok())
        .filter(|event| parse_nostr_identity_facet_acceptance_event(event).is_ok())
        .collect()
}

/// Publish the queued acceptances. They stay queued: publishing again is
/// harmless, and it reaches relays that missed them.
pub async fn publish_queued_facet_acceptances(
    client: &Client,
    config_dir: &Path,
) -> Result<usize, RelayError> {
    let mut published = 0;
    for event in queued_facet_acceptances(config_dir) {
        let output = client
            .send_event(&event)
            .await
            .map_err(|e| RelayError::Client(e.to_string()))?;
        if !output.success.is_empty() {
            published += 1;
        }
    }
    Ok(published)
}

#[cfg(test)]
#[path = "recovery_acceptance/tests.rs"]
mod tests;
