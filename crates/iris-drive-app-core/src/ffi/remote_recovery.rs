use super::{FfiApp, ProviderSummaryMode, label_option};
use crate::{NativeAppAction, NativeAppState};
use anyhow::{Result, anyhow, bail, ensure};
use iris_drive_core::{
    AppConfig, Drive, Profile,
    config_lock::ConfigMutationLock,
    paths::{config_path_in, key_path_in},
    relay_sync,
};
use nostr_connect::prelude::BoxedFuture;
use nostr_connect::prelude::{AuthUrlHandler, NostrConnect, NostrConnectURI};
use nostr_sdk::{Keys, NostrSigner, Url};
use std::{
    fmt,
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::sync::watch;
use tracing::instrument::WithSubscriber;

fn signer_uri(value: &str) -> Result<NostrConnectURI> {
    let invalid = || anyhow!("Enter a bunker:// link from your signer, including its relay");
    let url = Url::parse(value.trim()).map_err(|_| invalid())?;
    ensure!(
        url.scheme() == "bunker"
            && url.host_str().is_some_and(|key| {
                key.len() == 64
                    && key
                        .bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && matches!(url.path(), "" | "/")
            && url.fragment().is_none(),
        invalid()
    );
    let mut relay_count = 0;
    let mut secret_count = 0;
    for (name, value) in url.query_pairs() {
        if name == "relay" {
            let relay = Url::parse(&value).map_err(|_| invalid())?;
            ensure!(
                matches!(relay.scheme(), "ws" | "wss")
                    && relay.host().is_some()
                    && relay.username().is_empty()
                    && relay.password().is_none()
                    && relay.fragment().is_none(),
                invalid()
            );
            relay_count += 1;
        } else if name == "secret" {
            secret_count += 1;
        }
    }
    ensure!(
        (1..=8).contains(&relay_count) && secret_count <= 1,
        invalid()
    );
    NostrConnectURI::parse(url.as_str()).map_err(|_| invalid())
}

#[derive(Clone, Default)]
struct SignerApproval(Arc<Mutex<Option<String>>>);

impl fmt::Debug for SignerApproval {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SignerApproval")
    }
}

impl AuthUrlHandler for SignerApproval {
    fn on_auth_url(
        &self,
        url: Url,
    ) -> BoxedFuture<'_, std::result::Result<(), Box<dyn std::error::Error>>> {
        Box::pin(async move {
            if url.scheme() == "https"
                && url.host().is_some()
                && url.username().is_empty()
                && url.password().is_none()
            {
                *self
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(url.into());
            }
            Ok(())
        })
    }
}

#[derive(Debug)]
pub(super) struct SignerRequest {
    approval: SignerApproval,
    cancel: watch::Sender<bool>,
}

impl FfiApp {
    pub(super) fn dispatch_signer_action(
        &self,
        action: &NativeAppAction,
    ) -> Option<NativeAppState> {
        match action {
            NativeAppAction::SignerApprovalLink | NativeAppAction::CancelSignerRecovery => {
                Some(self.signer_interaction(action))
            }
            NativeAppAction::RestoreProfile {
                recovery_secret,
                app_key_label,
            } if recovery_secret.trim().starts_with("bunker://") => {
                Some(self.restore_with_signer(recovery_secret, app_key_label))
            }
            _ => None,
        }
    }

    fn signer_interaction(&self, action: &NativeAppAction) -> NativeAppState {
        self.with_runtime(|r| {
            let mut state = r.state();
            if let Some(request) = &r.signer_request {
                if matches!(action, NativeAppAction::CancelSignerRecovery) {
                    request.cancel.send_replace(true);
                } else {
                    state.signer_approval_url.clone_from(
                        &request
                            .approval
                            .0
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner),
                    );
                }
            }
            state
        })
    }

    fn restore_with_signer(&self, connection: &str, label: &str) -> NativeAppState {
        let (cancel, mut cancelled) = watch::channel(false);
        let approval = SignerApproval::default();
        let prepared = self.with_runtime(|r| {
            if r.signer_request.is_some() {
                return None;
            }
            r.signer_request = Some(SignerRequest {
                approval: approval.clone(),
                cancel,
            });
            Some(r.data_dir.clone())
        });
        let Some(dir) = prepared else {
            let mut state = self.state();
            state.error = "Waiting for your signer".into();
            return state;
        };
        let result = (|| -> Result<()> {
            let dir = Path::new(&dir);
            let config = AppConfig::load_or_default(config_path_in(dir))?;
            ensure!(
                config
                    .profile
                    .as_ref()
                    .is_none_or(|p| p.profile_roster_ops.is_empty() && !p.is_authorized()),
                "A Drive profile is already configured"
            );
            let uri = signer_uri(connection)?;
            let tokio = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            // The library logs decrypted RPC parameters at DEBUG, including the pairing token.
            let profile = tokio.block_on(
                async {
                    tokio::select! {
                        biased;
                        _ = cancelled.changed() => bail!("Sign-in cancelled"),
                        result = recover(dir, uri, &config.relays, label_option(label), approval) => result,
                    }
                }.with_subscriber(tracing::subscriber::NoSubscriber::default()),
            )?;
            self.with_runtime(|_| persist_profile(dir, &config, profile, &cancelled))
        })();
        self.with_runtime(|r| {
            r.signer_request = None;
            r.state_revision = r.state_revision.wrapping_add(1);
            match result {
                Ok(()) => {
                    r.state.error.clear();
                    r.set_sync_running(true);
                    r.reload_from_disk(ProviderSummaryMode::Skip);
                    r.reconcile_app_key_link_exchange(true);
                    r.start_browser_gateway_if_needed();
                }
                Err(error) => r.state.error = error.to_string(),
            }
            r.state()
        })
    }
}

fn persist_profile(
    dir: &Path,
    config: &AppConfig,
    profile: Profile,
    cancelled: &watch::Receiver<bool>,
) -> Result<()> {
    let _guard = ConfigMutationLock::acquire_blocking(dir)?;
    let mut current = AppConfig::load_or_default(config_path_in(dir))?;
    ensure!(
        !(*cancelled.borrow())
            && current
                .profile
                .as_ref()
                .is_none_or(|p| p.profile_roster_ops.is_empty() && !p.is_authorized())
            && current
                .profile
                .as_ref()
                .map(|p| (p.profile_id, &p.app_key_pubkey))
                == config
                    .profile
                    .as_ref()
                    .map(|p| (p.profile_id, &p.app_key_pubkey)),
        "Drive setup changed during sign-in"
    );
    if key_path_in(dir).exists() {
        ensure!(
            iris_drive_core::AppKey::load(key_path_in(dir))?.pubkey_hex()
                == profile.app_key.pubkey_hex(),
            "The device key changed during sign-in"
        );
    } else {
        profile.app_key.save()?;
    }
    // A pending, unbound join has a placeholder root. Replace it with the restored UUID.
    current.upsert_drive(Drive::primary(profile.state.root_scope_id()));
    current.profile = Some(profile.state);
    current.save(config_path_in(dir))?;
    Ok(())
}

async fn recover(
    dir: &Path,
    uri: NostrConnectURI,
    relays: &[String],
    label: Option<String>,
    approval: SignerApproval,
) -> Result<Profile> {
    let mut signer = NostrConnect::new(uri, Keys::generate(), Duration::from_secs(90), None)
        .map_err(|_| anyhow!("Could not connect to the signer"))?;
    signer.auth_url_handler(approval);
    let result = async {
        let pubkey = signer.get_public_key().await.map_err(|_| {
            anyhow!("Signer did not approve the connection. Open your signer and try again.")
        })?;
        let client = relay_sync::connect(relays).await?;
        let candidates =
            relay_sync::fetch_signer_restore_candidates(&client, pubkey, Duration::from_secs(8))
                .await;
        relay_sync::shutdown_client(&client).await;
        let Some(candidate) = candidates?.into_iter().next() else {
            bail!("No Drive profile found for this signer on your Drive relays");
        };
        Profile::restore_with_remote_signer(
            dir,
            &signer,
            candidate.profile_id,
            candidate.profile_roster_ops,
            label,
        )
        .await
    }
    .await;
    signer.shutdown().await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn approval_and_pairing_urls_hide_secrets_and_validate_scheme() {
        let approval = SignerApproval::default();
        for invalid in [
            "http://example.com/a",
            "https://user:password@example.com/a",
            "custom:unsafe",
        ] {
            approval
                .on_auth_url(Url::parse(invalid).unwrap())
                .await
                .unwrap();
            assert!(approval.0.lock().unwrap().is_none());
        }
        let valid = "https://example.com/authorize?token=private-test-token";
        approval
            .on_auth_url(Url::parse(valid).unwrap())
            .await
            .unwrap();
        assert_eq!(approval.0.lock().unwrap().as_deref(), Some(valid));
        assert!(!format!("{approval:?}").contains("private-test-token"));
        let pubkey = Keys::generate().public_key().to_hex();
        for value in [
            "bunker://wrong?relay=wss://example.com&secret=private-test-token".to_owned(),
            format!("bunker://{pubkey}?relay=https://example.com&secret=private-test-token"),
            format!("bunker://{pubkey}?relay=wss://user:private-test-token@example.com"),
        ] {
            let error = signer_uri(&value).unwrap_err().to_string();
            assert!(!error.contains("private-test-token"));
        }
    }
}
