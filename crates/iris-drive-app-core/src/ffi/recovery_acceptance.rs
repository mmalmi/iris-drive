//! Have a new recovery phrase sign its acceptance while its words are known.
//!
//! The apps show a phrase (`generate_recovery_key` or
//! `recovery_pubkey_for_phrase`), then add only its public key with
//! `AddRecoveryDevice`. The phrase key waits here, in memory, between the
//! two. Adding the phrase signs the acceptance the web's phrase sign-in looks
//! for and queues it for publishing, then forgets the key.

use iris_drive_core::ProfileState;
use iris_drive_core::recovery_acceptance::{
    queue_facet_acceptance, recovery_phrase_add_op, sign_recovery_phrase_acceptance,
};
use nostr_sdk::Keys;
use std::path::Path;
use std::sync::Mutex;

static SHOWN_RECOVERY_PHRASE_KEY: Mutex<Option<Keys>> = Mutex::new(None);

fn shown_key() -> std::sync::MutexGuard<'static, Option<Keys>> {
    SHOWN_RECOVERY_PHRASE_KEY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Keep the key of the phrase just shown, replacing any earlier one.
pub(super) fn remember_shown_recovery_phrase_key(keys: &Keys) {
    *shown_key() = Some(keys.clone());
}

/// Sign and queue the acceptance for the phrase `recovery_pubkey_hex` that
/// `state` just added. Does nothing when that phrase was not just shown here.
pub(super) fn queue_added_recovery_phrase_acceptance(
    config_dir: &Path,
    state: &ProfileState,
    recovery_pubkey_hex: &str,
) -> Result<bool, String> {
    let keys = {
        let mut shown = shown_key();
        if shown
            .as_ref()
            .is_none_or(|keys| keys.public_key().to_hex() != recovery_pubkey_hex)
        {
            return Ok(false);
        }
        shown.take().expect("checked above")
    };
    let Some(add_op) = recovery_phrase_add_op(state, recovery_pubkey_hex) else {
        return Ok(false);
    };
    let event = sign_recovery_phrase_acceptance(&keys, state.profile_id, add_op)
        .map_err(|error| format!("signing recovery phrase acceptance: {error}"))?;
    queue_facet_acceptance(config_dir, &event)
        .map_err(|error| format!("saving recovery phrase acceptance: {error}"))?;
    Ok(true)
}
