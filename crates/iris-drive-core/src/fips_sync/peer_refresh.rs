use std::sync::Mutex;

use super::FipsPeerConfigSnapshot;

pub(super) struct PeerConfigRefresh {
    applied: Mutex<Option<FipsPeerConfigSnapshot>>,
    serialization: tokio::sync::Mutex<()>,
}

impl PeerConfigRefresh {
    pub(super) fn new(applied: Option<FipsPeerConfigSnapshot>) -> Self {
        Self {
            applied: Mutex::new(applied),
            serialization: tokio::sync::Mutex::new(()),
        }
    }

    pub(super) async fn load_serialized<T>(
        &self,
        load: impl FnOnce() -> T,
    ) -> (PeerConfigRefreshGuard<'_>, T) {
        let guard = PeerConfigRefreshGuard {
            state: self,
            _serialization: self.serialization.lock().await,
        };
        let loaded = load();
        (guard, loaded)
    }

    pub(super) fn applied(&self) -> Option<FipsPeerConfigSnapshot> {
        self.applied
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

pub(super) struct PeerConfigRefreshGuard<'a> {
    state: &'a PeerConfigRefresh,
    _serialization: tokio::sync::MutexGuard<'a, ()>,
}

impl PeerConfigRefreshGuard<'_> {
    pub(super) fn is_current(&self, snapshot: &FipsPeerConfigSnapshot) -> bool {
        self.state.applied().as_ref() == Some(snapshot)
    }

    pub(super) fn commit(self, snapshot: FipsPeerConfigSnapshot) {
        *self
            .state
            .applied
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(snapshot);
    }
}
