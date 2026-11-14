use super::{
    AppKey, NostrIdentityId, NostrIdentityKeyPurpose, Profile, ProfileError,
    SignedNostrIdentityRosterOp, key_path_in, project_nostr_identity_roster,
    recovery_authority_purpose,
};
use nostr_sdk::Keys;
use std::path::Path;

impl Profile {
    /// Join an existing account with its recovery phrase key.
    ///
    /// Like [`Profile::restore_with_profile_roster_ops`], the phrase key admits
    /// this install's `AppKey` and rewraps the current Drive key for it. This
    /// keeps nothing on disk: neither the phrase nor a new `AppKey` is saved,
    /// so the caller saves the `AppKey` with the rest of the restore.
    pub fn restore_with_recovery_phrase_keys(
        config_dir: &Path,
        recovery_keys: &Keys,
        profile_id: NostrIdentityId,
        profile_roster_ops: Vec<SignedNostrIdentityRosterOp>,
        app_key_label: Option<String>,
    ) -> Result<Self, ProfileError> {
        let projection = project_nostr_identity_roster(profile_id, profile_roster_ops.clone());
        let purpose = recovery_authority_purpose(
            &projection,
            &recovery_keys.public_key().to_hex(),
            Some(NostrIdentityKeyPurpose::RecoveryPhrase),
        )?;
        let key_path = key_path_in(config_dir);
        let app_key = if key_path.exists() {
            AppKey::load(key_path)?
        } else {
            AppKey::generate(key_path)
        };
        let mut profile =
            Self::pending_recovery(app_key, profile_id, profile_roster_ops, app_key_label);
        let label = profile.state.app_key_label.clone();
        profile.admit_current_app_key_with_authority_keys(recovery_keys, purpose, label)?;
        Ok(profile)
    }
}
