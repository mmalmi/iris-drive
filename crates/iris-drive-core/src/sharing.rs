use std::collections::{BTreeMap, BTreeSet};

use nostr_sdk::nips::nip44::{self, Version as Nip44Version};
use nostr_sdk::{Keys, PublicKey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::iris_profile::{
    IrisProfileCapabilities, IrisProfileError, IrisProfileFacet, IrisProfileId,
    IrisProfileRosterOp, IrisProfileRosterProjection, SignedIrisProfileRosterOp,
    build_iris_profile_roster_op_event, parse_iris_profile_roster_op_event,
    project_iris_profile_roster,
};
use crate::provider::{normalize_provider_path, sanitized_provider_file_name};

#[derive(Debug, Error)]
pub enum SharingError {
    #[error("share path: {0}")]
    Path(String),
    #[error("invalid recipient pubkey: {0}")]
    InvalidPubkey(String),
    #[error("iris profile: {0}")]
    IrisProfile(#[from] IrisProfileError),
    #[error("failed to wrap share key: {0}")]
    Wrap(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareRole {
    Admin,
    Editor,
    Reader,
}

impl ShareRole {
    #[must_use]
    pub fn capabilities(self) -> IrisProfileCapabilities {
        match self {
            Self::Admin => IrisProfileCapabilities::app_admin(),
            Self::Editor => IrisProfileCapabilities::app_writer(),
            Self::Reader => IrisProfileCapabilities::app_reader(),
        }
    }

    #[must_use]
    fn rank(self) -> u8 {
        match self {
            Self::Reader => 0,
            Self::Editor => 1,
            Self::Admin => 2,
        }
    }

    #[must_use]
    fn strongest(left: Self, right: Self) -> Self {
        if left.rank() >= right.rank() {
            left
        } else {
            right
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareRecipient {
    pub profile_id: IrisProfileId,
    pub app_pubkey: String,
    pub role: ShareRole,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SharedFolder {
    pub share_id: IrisProfileId,
    pub owner_profile_id: IrisProfileId,
    pub source_path: String,
    pub display_name: String,
    pub local_role: ShareRole,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub participant_profiles: BTreeMap<String, IrisProfileId>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roster_ops: Vec<SignedIrisProfileRosterOp>,
}

impl SharedFolder {
    #[must_use]
    pub fn projection(&self) -> IrisProfileRosterProjection {
        project_iris_profile_roster(self.share_id, self.roster_ops.clone())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShareShortcut {
    pub share_id: IrisProfileId,
    pub path: String,
    pub target_path: String,
}

impl ShareShortcut {
    pub fn new(
        share_id: IrisProfileId,
        path: &str,
        target_path: &str,
    ) -> Result<Self, SharingError> {
        let path =
            normalize_provider_path(path).map_err(|error| SharingError::Path(error.to_string()))?;
        let target_path = if target_path.trim_matches('/').is_empty() {
            String::new()
        } else {
            normalize_provider_path(target_path)
                .map_err(|error| SharingError::Path(error.to_string()))?
        };
        Ok(Self {
            share_id,
            path,
            target_path,
        })
    }
}

pub fn create_shared_folder(
    owner_keys: &nostr_sdk::Keys,
    owner_profile_id: IrisProfileId,
    source_path: &str,
    display_name: &str,
    local_label: Option<String>,
    recipients: Vec<ShareRecipient>,
    created_at: i64,
) -> Result<SharedFolder, SharingError> {
    let source_path = normalize_provider_path(source_path)
        .map_err(|error| SharingError::Path(error.to_string()))?;
    let display_name = sanitized_provider_file_name(display_name);
    let share_id = IrisProfileId::new_v4();
    let owner_pubkey = owner_keys.public_key().to_hex();
    let mut participant_profiles = BTreeMap::from([(owner_pubkey.clone(), owner_profile_id)]);
    let mut normalized_recipients = BTreeMap::<String, ShareRecipient>::new();
    for recipient in recipients {
        PublicKey::from_hex(&recipient.app_pubkey)
            .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
        participant_profiles.insert(recipient.app_pubkey.clone(), recipient.profile_id);
        normalized_recipients
            .entry(recipient.app_pubkey.clone())
            .and_modify(|existing| {
                existing.role = ShareRole::strongest(existing.role, recipient.role);
                if existing.label.is_none() {
                    existing.label.clone_from(&recipient.label);
                }
            })
            .or_insert(recipient);
    }

    let mut ops = Vec::new();
    ops.push(sign_share_roster_op(
        owner_keys,
        share_id,
        IrisProfileRosterOp::AddFacet {
            facet: IrisProfileFacet::app_key(
                owner_pubkey,
                created_at,
                local_label,
                ShareRole::Admin.capabilities(),
            ),
        },
        created_at,
    )?);

    let mut op_time = created_at;
    for recipient in normalized_recipients.into_values() {
        op_time += 1;
        ops.push(sign_share_roster_op(
            owner_keys,
            share_id,
            IrisProfileRosterOp::AddFacet {
                facet: IrisProfileFacet::app_key(
                    recipient.app_pubkey,
                    op_time,
                    recipient.label,
                    recipient.role.capabilities(),
                ),
            },
            op_time,
        )?);
    }

    let share_key = generate_share_key();
    let projection = project_iris_profile_roster(share_id, ops.clone());
    let recipients = projection
        .active_facets
        .values()
        .filter(|facet| facet.capabilities.can_receive_key_wraps)
        .map(|facet| facet.pubkey.as_str())
        .collect::<BTreeSet<_>>();
    let wrapped_dck = wrap_share_key(owner_keys, recipients, &share_key)?;
    ops.push(sign_share_roster_op(
        owner_keys,
        share_id,
        IrisProfileRosterOp::RotateKeyEpoch {
            epoch: 1,
            wrapped_dck,
        },
        op_time + 1,
    )?);

    Ok(SharedFolder {
        share_id,
        owner_profile_id,
        source_path,
        display_name,
        local_role: ShareRole::Admin,
        participant_profiles,
        roster_ops: ops,
    })
}

fn sign_share_roster_op(
    signer_keys: &Keys,
    share_id: IrisProfileId,
    op: IrisProfileRosterOp,
    created_at: i64,
) -> Result<SignedIrisProfileRosterOp, SharingError> {
    let event = build_iris_profile_roster_op_event(
        signer_keys,
        share_id,
        Vec::new(),
        None,
        op,
        created_at,
    )?;
    parse_iris_profile_roster_op_event(&event).map_err(SharingError::from)
}

fn generate_share_key() -> [u8; 32] {
    let keys = Keys::generate();
    let mut out = [0_u8; 32];
    out.copy_from_slice(keys.secret_key().as_secret_bytes());
    out
}

fn wrap_share_key<'a, I>(
    signer_keys: &Keys,
    recipients: I,
    share_key: &[u8; 32],
) -> Result<BTreeMap<String, String>, SharingError>
where
    I: IntoIterator<Item = &'a str>,
{
    let mut wraps = BTreeMap::new();
    for recipient in recipients {
        let recipient_pk = PublicKey::from_hex(recipient)
            .map_err(|error| SharingError::InvalidPubkey(error.to_string()))?;
        let ciphertext = nip44::encrypt(
            signer_keys.secret_key(),
            &recipient_pk,
            share_key.as_slice(),
            Nip44Version::V2,
        )
        .map_err(|error| SharingError::Wrap(error.to_string()))?;
        wraps.insert(recipient.to_string(), ciphertext);
    }
    Ok(wraps)
}

#[cfg(test)]
mod tests {
    use nostr_sdk::Keys;

    use super::*;

    fn recipient(role: ShareRole) -> (Keys, ShareRecipient) {
        let keys = Keys::generate();
        (
            keys.clone(),
            ShareRecipient {
                profile_id: IrisProfileId::new_v4(),
                app_pubkey: keys.public_key().to_hex(),
                role,
                label: Some("Phone".to_string()),
            },
        )
    }

    #[test]
    fn shared_folder_has_own_roster_epoch_and_shortcut() {
        let owner_keys = Keys::generate();
        let owner_profile_id = IrisProfileId::new_v4();
        let (recipient_keys, recipient) = recipient(ShareRole::Editor);

        let folder = create_shared_folder(
            &owner_keys,
            owner_profile_id,
            "Projects/Alpha",
            "Alpha",
            Some("Desktop".to_string()),
            vec![recipient],
            10,
        )
        .unwrap();

        assert_eq!(folder.owner_profile_id, owner_profile_id);
        assert_eq!(folder.source_path, "Projects/Alpha");
        assert_eq!(folder.display_name, "Alpha");

        let projection = folder.projection();
        let owner_pubkey = owner_keys.public_key().to_hex();
        let recipient_pubkey = recipient_keys.public_key().to_hex();
        assert!(projection.can_admin_profile(&owner_pubkey));
        assert!(projection.can_write_roots(&owner_pubkey));
        assert!(projection.can_write_roots(&recipient_pubkey));
        assert!(!projection.can_admin_profile(&recipient_pubkey));
        let epoch = projection.key_epochs.values().next_back().unwrap();
        assert_eq!(epoch.epoch, 1);
        assert!(epoch.wrapped_dck.contains_key(&owner_pubkey));
        assert!(epoch.wrapped_dck.contains_key(&recipient_pubkey));

        let shortcut = ShareShortcut::new(folder.share_id, "Shared/Alpha", "").unwrap();
        assert_eq!(shortcut.path, "Shared/Alpha");
        assert_eq!(shortcut.target_path, "");
    }

    #[test]
    fn reader_recipient_gets_key_wrap_without_write_authority() {
        let owner_keys = Keys::generate();
        let (reader_keys, reader) = recipient(ShareRole::Reader);

        let folder = create_shared_folder(
            &owner_keys,
            IrisProfileId::new_v4(),
            "Photos",
            "Photos",
            None,
            vec![reader],
            20,
        )
        .unwrap();

        let projection = folder.projection();
        let reader_pubkey = reader_keys.public_key().to_hex();
        assert!(!projection.can_write_roots(&reader_pubkey));
        assert!(!projection.can_admin_profile(&reader_pubkey));
        assert_eq!(
            projection.key_wrap_status(&reader_pubkey, 1),
            crate::iris_profile::KeyWrapStatus::Available
        );
    }

    #[test]
    fn share_paths_reject_traversal_and_native_separators() {
        let owner_keys = Keys::generate();
        assert!(
            create_shared_folder(
                &owner_keys,
                IrisProfileId::new_v4(),
                "../Secrets",
                "Secrets",
                None,
                Vec::new(),
                30,
            )
            .is_err()
        );
        assert!(ShareShortcut::new(IrisProfileId::new_v4(), "Shared\\Alpha", "").is_err());
    }

    #[test]
    fn shared_folders_and_shortcuts_are_config_helpers() {
        let owner_keys = Keys::generate();
        let folder = create_shared_folder(
            &owner_keys,
            IrisProfileId::new_v4(),
            "Projects/Beta",
            "Beta",
            None,
            Vec::new(),
            40,
        )
        .unwrap();
        let shortcut = ShareShortcut::new(folder.share_id, "Shared/Beta", "").unwrap();

        let mut config = crate::config::AppConfig::default();
        assert!(config.upsert_shared_folder(folder.clone()));
        assert!(!config.upsert_shared_folder(folder.clone()));
        assert_eq!(
            config.shared_folder(folder.share_id).unwrap().source_path,
            "Projects/Beta"
        );
        assert!(config.upsert_share_shortcut(shortcut.clone()));
        assert!(!config.upsert_share_shortcut(shortcut));

        let raw = toml::to_string(&config).unwrap();
        let round_tripped: crate::config::AppConfig = toml::from_str(&raw).unwrap();
        assert_eq!(round_tripped.shared_folders, vec![folder]);
        assert_eq!(round_tripped.share_shortcuts.len(), 1);
    }
}
