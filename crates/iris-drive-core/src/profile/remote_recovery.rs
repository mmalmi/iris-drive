use super::{
    AppKey, AppKeyAuthorizationState, NostrIdentityId, Profile, ProfileError, ProfileState,
    SignedNostrIdentityRosterOp, decode_dck_plaintext, default_app_key_link_secret, key_path_in,
    next_profile_timestamp, nostr_identity_roster_parent_ids, parse_nostr_identity_roster_op_event,
    project_nostr_identity_roster, resolve_app_key_label,
};
use anyhow::{Result, anyhow, ensure};
use nostr_identity::{
    IDENTITY_CAPABILITY_ADMIN, IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS,
    IDENTITY_CAPABILITY_RECEIVE_SECRET_WRAPS, IDENTITY_CAPABILITY_WRITE,
    IDENTITY_GRAPH_ROSTER_TYPE, IDENTITY_PURPOSE_APP,
};
use nostr_sdk::PublicKey;
use nostr_sdk::{EventBuilder, Kind, NostrSigner, Tag, Timestamp, UnsignedEvent};
use std::path::Path;
use uuid::Uuid;

impl Profile {
    pub(super) fn pending_recovery(
        device: AppKey,
        profile_id: NostrIdentityId,
        ops: Vec<SignedNostrIdentityRosterOp>,
        label: Option<String>,
    ) -> Self {
        let pubkey = device.pubkey_hex();
        let mut state = ProfileState {
            profile_id,
            app_key_label: resolve_app_key_label(label, &pubkey),
            app_key_pubkey: pubkey,
            profile_roster_ops: ops,
            app_key_link_secret: default_app_key_link_secret(),
            authorization_state: AppKeyAuthorizationState::AwaitingApproval,
            app_keys: None,
            profile_roster_projection: None,
            outbound_app_key_link_request: None,
            inbound_app_key_link_requests: Vec::new(),
            handled_app_key_link_requests: Vec::new(),
            pending_device_approval_receipts: Vec::new(),
        };
        state.sync_app_keys_from_profile();
        Self {
            state,
            app_key: device,
        }
    }

    /// The external account admits one fresh local admin key. That key signs
    /// subsequent Drive writes, including the epoch that wraps the existing DCK.
    pub async fn restore_with_remote_signer(
        dir: &Path,
        signer: &dyn NostrSigner,
        profile_id: NostrIdentityId,
        ops: Vec<SignedNostrIdentityRosterOp>,
        label: Option<String>,
    ) -> Result<Self> {
        let authority = signer
            .get_public_key()
            .await
            .map_err(|_| anyhow!("Signer did not provide its account"))?;
        let projection = project_nostr_identity_roster(profile_id, ops.clone());
        let facet = projection
            .active_facets
            .get(&authority.to_hex())
            .ok_or(ProfileError::RecoveryAuthorityUnavailable)?;
        ensure!(
            facet.capabilities.can_recover_app_keys || facet.capabilities.can_admin_profile,
            ProfileError::RecoveryAuthorityUnavailable
        );
        let dck = if facet.capabilities.can_decrypt_secret_epochs {
            let epoch = projection
                .secret_epochs
                .values()
                .next_back()
                .ok_or(ProfileError::NoCurrentAppKeysProjection)?;
            let wrap = epoch
                .wrapped_secrets
                .get(&authority.to_hex())
                .ok_or(ProfileError::NoWrapForThisAppKey)?;
            let sender = PublicKey::from_hex(&epoch.signed_by_pubkey)?;
            let plaintext = signer
                .nip44_decrypt(&sender, wrap)
                .await
                .map_err(|_| anyhow!("Signer could not decrypt the Drive recovery key"))?;
            Some(decode_dck_plaintext(plaintext.as_bytes())?)
        } else {
            None
        };

        let key_path = key_path_in(dir);
        let app_key = if key_path.exists() {
            AppKey::load(key_path)?
        } else {
            AppKey::generate(key_path)
        };
        let mut profile = Self::pending_recovery(app_key, profile_id, ops, label);
        let now = next_profile_timestamp(&profile.state);
        let mut draft = device_admission_draft(&profile.state, authority, now)?;
        let expected_id = draft.id();
        let admission = signer
            .sign_event(draft)
            .await
            .map_err(|_| anyhow!("Device approval was declined or timed out"))?;
        ensure!(
            admission.id == expected_id && admission.verify().is_ok(),
            "Signer returned a different device approval"
        );
        profile
            .state
            .profile_roster_ops
            .push(parse_nostr_identity_roster_op_event(&admission)?);
        profile.state.profile_roster_projection = None;
        profile.sync_app_keys_from_profile_with_device_labels();
        ensure!(
            profile.state.can_admin_profile(),
            "Device approval was not accepted by the Drive roster"
        );
        if let Some(dck) = dck {
            profile.rotate_profile_dck_epoch(&dck, now + 1)?;
            profile.sync_app_keys_from_profile_with_device_labels();
            ensure!(
                profile.current_dck()? == dck,
                "Device could not open its Drive recovery key"
            );
        }
        Ok(profile)
    }
}

// The identity crate's synchronous event builder requires the signer's private
// key. Build its single add-key operation as an unsigned event for NIP-46.
fn device_admission_draft(
    state: &ProfileState,
    signer: PublicKey,
    now: i64,
) -> Result<UnsignedEvent> {
    let mut tags = vec![Tag::parse(["i", &state.profile_id.to_string(), "subject"])?];
    for parent in nostr_identity_roster_parent_ids(&state.profile_roster_ops) {
        tags.push(Tag::parse(["e", &parent, "", "prev"])?);
    }
    let nonce = Uuid::new_v4().to_string();
    tags.push(Tag::parse(["i", &nonce])?);
    tags.push(Tag::parse(["p", &state.app_key_pubkey])?);
    let created = now.to_string();
    let actor = signer.to_hex();
    for (name, value) in [
        ("actor_pubkey", actor.as_str()),
        ("client_nonce", nonce.as_str()),
        ("created_at", created.as_str()),
        ("key_added_at", created.as_str()),
        ("key_capability", IDENTITY_CAPABILITY_ADMIN),
        ("key_capability", IDENTITY_CAPABILITY_DECRYPT_SECRET_EPOCHS),
        ("key_capability", IDENTITY_CAPABILITY_RECEIVE_SECRET_WRAPS),
        ("key_capability", IDENTITY_CAPABILITY_WRITE),
        ("key_pubkey", state.app_key_pubkey.as_str()),
        ("key_purpose", IDENTITY_PURPOSE_APP),
        ("op", "add_key"),
        ("schema", "1"),
        ("type", IDENTITY_GRAPH_ROSTER_TYPE),
    ] {
        tags.push(Tag::parse([name, value])?);
    }
    Ok(
        EventBuilder::new(Kind::from(crate::KIND_NOSTR_IDENTITY_ROSTER_OP), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(u64::try_from(now)?))
            .build(signer),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::recovery_phrase_path_in;
    use nostr_sdk::Keys;
    use tempfile::tempdir;
    #[tokio::test]
    async fn remote_signer_restores_drive_without_importing_owner_key() {
        let original = tempdir().unwrap();
        let fresh = tempdir().unwrap();
        let mut owner = Profile::create(original.path(), None).unwrap();
        let signer = Keys::generate();
        owner
            .add_nip46_recovery(&signer.public_key().to_hex(), None, true)
            .unwrap();
        let dck = owner.current_dck().unwrap();
        let profile = Profile::restore_with_remote_signer(
            fresh.path(),
            &signer,
            owner.state.profile_id,
            owner.state.profile_roster_ops.clone(),
            Some("new device".into()),
        )
        .await
        .unwrap();
        assert_eq!(profile.current_dck().unwrap(), dck);
        assert!(profile.state.can_admin_profile());
        assert_ne!(profile.app_key.pubkey_hex(), signer.public_key().to_hex());
        assert!(!recovery_phrase_path_in(fresh.path()).exists());
        assert_eq!(profile.state.profile_id, owner.state.profile_id);

        let rejected = tempdir().unwrap();
        assert!(
            Profile::restore_with_remote_signer(
                rejected.path(),
                &Keys::generate(),
                owner.state.profile_id,
                owner.state.profile_roster_ops.clone(),
                None,
            )
            .await
            .is_err()
        );
        assert!(!key_path_in(rejected.path()).exists());
    }
}
