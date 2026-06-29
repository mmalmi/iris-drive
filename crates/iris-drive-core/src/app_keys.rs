//! Derived `AppKey` roster projection.
//!
//! Iris Drive's authoritative profile membership state is the signed
//! `NostrIdentity` roster-op log. This module owns the app-facing projection of
//! that log: active `AppKey` actors, their roles, and current drive content key
//! wraps. `profile_id` is an `NostrIdentity` UUID string, never a Nostr pubkey.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Privilege level for an app actor in the roster.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AppActorRole {
    /// Can sign and publish future roster events.
    Admin,
    /// Can decrypt and publish its own drive/share roots, but cannot alter
    /// roster membership or roles.
    #[default]
    Member,
}

/// One authorized per-install `AppKey` actor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppActorEntry {
    /// Hex-encoded `AppKey` pubkey.
    pub pubkey: String,
    /// When this `AppKey` actor was first added (unix seconds).
    pub added_at: i64,
    /// Optional human-readable label (e.g. "Mac native app").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Whether this `AppKey` actor may sign future roster updates.
    #[serde(default)]
    pub role: AppActorRole,
}

impl AppActorEntry {
    #[must_use]
    pub fn admin(pubkey: String, added_at: i64, label: Option<String>) -> Self {
        Self {
            pubkey,
            added_at,
            label,
            role: AppActorRole::Admin,
        }
    }

    #[must_use]
    pub fn member(pubkey: String, added_at: i64, label: Option<String>) -> Self {
        Self {
            pubkey,
            added_at,
            label,
            role: AppActorRole::Member,
        }
    }

    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.role == AppActorRole::Admin
    }
}

/// Derived active `AppKey` actor view for one `NostrIdentity` roster.
///
/// This is a deterministic cache rebuilt from signed roster ops, not a signed
/// full-roster authority. It carries the current drive content key (DCK)
/// NIP-44-wrapped to each authorized `AppKey` pubkey.
///
/// Rotating the DCK on every membership change gives forward secrecy against
/// `AppKey` revocation: a revoked app install retains anything it already
/// downloaded but cannot decrypt the drive's new root once the next rotation
/// lands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppKeysProjection {
    /// Stable `NostrIdentity` UUID string that scopes this roster.
    #[serde(default)]
    pub profile_id: String,
    /// Pubkey of the admin `AppKey` that signed the key epoch represented by
    /// this projection.
    #[serde(
        default,
        alias = "owner_pubkey",
        skip_serializing_if = "Option::is_none"
    )]
    pub signed_by_pubkey: Option<String>,
    /// Created-at timestamp of the key epoch represented by this projection.
    pub created_at: i64,
    #[serde(alias = "devices")]
    pub app_actors: Vec<AppActorEntry>,
    /// Monotonically-increasing counter. Bumped each time the DCK
    /// rotates (on approve, revoke, or explicit `rotate_dck`).
    #[serde(default)]
    pub dck_generation: u64,
    /// NIP-44 wraps of the current DCK, keyed by `AppKey` pubkey hex.
    /// Encrypted by the roster-signing admin `AppKey` to each `AppKey`'s
    /// pubkey. `AppKeys` not present in the map are effectively revoked from
    /// the current content.
    #[serde(default)]
    pub wrapped_dck: BTreeMap<String, String>,
}

impl AppKeysProjection {
    /// Sort app actor list deterministically (by pubkey). Use after merges
    /// so equality checks and serialization are stable.
    pub fn normalize(&mut self) {
        self.app_actors.sort_by(|a, b| a.pubkey.cmp(&b.pubkey));
        self.app_actors.dedup_by(|a, b| a.pubkey == b.pubkey);
    }

    #[must_use]
    pub fn contains(&self, app_actor_pubkey: &str) -> bool {
        self.app_actors
            .iter()
            .any(|actor| actor.pubkey == app_actor_pubkey)
    }

    #[must_use]
    pub fn app_actor(&self, pubkey: &str) -> Option<&AppActorEntry> {
        self.app_actors.iter().find(|actor| actor.pubkey == pubkey)
    }

    #[must_use]
    pub fn is_admin(&self, pubkey: &str) -> bool {
        self.app_actor(pubkey).is_some_and(AppActorEntry::is_admin)
    }

    #[must_use]
    pub fn signer_pubkey(&self) -> Option<&str> {
        self.signed_by_pubkey.as_deref()
    }
}

/// Outcome of applying incoming roster-op material to local profile state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApplyDecision {
    /// No current projection; incoming material becomes current.
    Adopted,
    /// Incoming material advances the current key epoch.
    Replaced,
    /// Incoming material extends or repairs the known roster op log.
    Merged,
    /// Incoming is older or from a different profile; ignored.
    Rejected,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn projection(
        profile_id: &str,
        created_at: i64,
        app_actors: &[(&str, i64)],
    ) -> AppKeysProjection {
        AppKeysProjection {
            profile_id: profile_id.into(),
            signed_by_pubkey: Some("admin".into()),
            created_at,
            app_actors: app_actors
                .iter()
                .map(|(pk, added)| AppActorEntry::member((*pk).into(), *added, None))
                .collect(),
            dck_generation: 0,
            wrapped_dck: BTreeMap::new(),
        }
    }

    #[test]
    fn normalize_dedupes_and_sorts() {
        let mut s = projection("profile", 100, &[("z", 100), ("a", 100), ("a", 200)]);
        s.normalize();
        assert_eq!(
            s.app_actors
                .iter()
                .map(|d| d.pubkey.as_str())
                .collect::<Vec<_>>(),
            vec!["a", "z"]
        );
    }

    #[test]
    fn round_trip_through_toml() {
        let s = AppKeysProjection {
            profile_id: "abc123".into(),
            signed_by_pubkey: Some("admin".into()),
            created_at: 1_700_000_000,
            app_actors: vec![AppActorEntry {
                pubkey: "dev1".into(),
                added_at: 1_699_000_000,
                label: Some("Mac mini".into()),
                role: AppActorRole::Admin,
            }],
            dck_generation: 1,
            wrapped_dck: BTreeMap::from([("dev1".to_string(), "abcdef".to_string())]),
        };
        let serialized = toml::to_string(&s).unwrap();
        assert!(serialized.contains("profile_id = \"abc123\""));
        assert!(!serialized.contains("owner_pubkey"));
        assert!(serialized.contains("role = \"admin\""));
        let back: AppKeysProjection = toml::from_str(&serialized).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn signer_pubkey_is_explicit_not_profile_id_fallback() {
        let projection = AppKeysProjection {
            profile_id: "550e8400-e29b-41d4-a716-446655440000".into(),
            signed_by_pubkey: None,
            created_at: 1,
            app_actors: vec![AppActorEntry::admin("admin".into(), 1, None)],
            dck_generation: 1,
            wrapped_dck: BTreeMap::new(),
        };

        assert_eq!(projection.signer_pubkey(), None);
    }

    #[test]
    fn app_actor_role_defaults_to_member() {
        let entry: AppActorEntry = toml::from_str(
            r#"
pubkey = "dev"
added_at = 1
"#,
        )
        .unwrap();
        assert_eq!(entry.role, AppActorRole::Member);
        assert!(!entry.is_admin());
    }

    #[test]
    fn projection_identifies_admin_app_actors() {
        let projection = AppKeysProjection {
            profile_id: "acct".into(),
            signed_by_pubkey: Some("admin".into()),
            created_at: 1,
            app_actors: vec![
                AppActorEntry::admin("admin".into(), 1, None),
                AppActorEntry::member("phone".into(), 1, None),
            ],
            dck_generation: 1,
            wrapped_dck: BTreeMap::new(),
        };
        assert!(projection.is_admin("admin"));
        assert!(!projection.is_admin("phone"));
    }
}
