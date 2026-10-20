//! Scoped update routes share the app client without replacing its peer roster.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex, Weak};

use anyhow::{Context, Result};
use nostr_pubsub_fips::FipsPubsubClient;

pub(crate) struct UpdateRoutes {
    client: Weak<FipsPubsubClient>,
    state: Mutex<RouteState>,
}

struct RouteState {
    application: Vec<String>,
    checks: BTreeMap<String, usize>,
}

impl RouteState {
    fn merged(&self, application: &[String], extra: Option<&str>) -> Vec<String> {
        application
            .iter()
            .map(String::as_str)
            .chain(self.checks.keys().map(String::as_str))
            .chain(extra)
            .collect::<BTreeSet<_>>()
            .into_iter()
            .map(str::to_owned)
            .collect()
    }
}

impl UpdateRoutes {
    pub(crate) fn new(client: &Arc<FipsPubsubClient>) -> Arc<Self> {
        Arc::new(Self {
            client: Arc::downgrade(client),
            state: Mutex::new(RouteState {
                application: client.options().routed_peers.clone(),
                checks: BTreeMap::new(),
            }),
        })
    }

    pub(crate) fn set_application_peers(&self, peers: Vec<String>) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("update route lock poisoned"))?;
        self.apply(state.merged(&peers, None))?;
        state.application = peers;
        Ok(())
    }

    pub(super) fn lease(self: &Arc<Self>, publisher: String) -> Result<UpdateRouteLease> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("update route lock poisoned"))?;
        self.apply(state.merged(&state.application, Some(&publisher)))?;
        *state.checks.entry(publisher.clone()).or_default() += 1;
        Ok(UpdateRouteLease {
            routes: self.clone(),
            publisher,
        })
    }

    fn apply(&self, peers: Vec<String>) -> Result<()> {
        self.client
            .upgrade()
            .context("update pubsub client is closed")?
            .set_routed_peers(peers)
            .context("configuring update publisher route")
    }
}

pub(super) struct UpdateRouteLease {
    routes: Arc<UpdateRoutes>,
    publisher: String,
}

impl Drop for UpdateRouteLease {
    fn drop(&mut self) {
        let Ok(mut state) = self.routes.state.lock() else {
            return;
        };
        if let Some(count) = state.checks.get_mut(&self.publisher) {
            *count -= 1;
            if *count == 0 {
                state.checks.remove(&self.publisher);
            }
        }
        if let Some(client) = self.routes.client.upgrade()
            && let Err(error) = client.set_routed_peers(state.merged(&state.application, None))
        {
            tracing::warn!(%error, "releasing update publisher route failed");
        }
    }
}
