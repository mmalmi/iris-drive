impl DirectRootExchange {
    fn peer_state_request_pending(&self) -> bool {
        !self.pending_state_request_peers.is_empty()
    }

    fn record_state_request_send(&mut self, stats: DirectRootAppSendStats) {
        // A successful send is not a remote acknowledgement. Partial/failed
        // batches remain pending, and periodic reconciliation repairs any
        // announcement lost after a locally successful send.
        if stats.selected_peers > 0
            && stats.sent_peers == stats.selected_peers
            && stats.failed_peers == 0
        {
            self.pending_state_request_peers.clear();
        }
    }
}

pub(crate) fn periodic_direct_root_repair_trigger(
    missing_online_roots: usize,
    pending_remote_roots: usize,
) -> &'static str {
    if pending_remote_roots > 0 {
        "pending_remote_root"
    } else if missing_online_roots > 0 {
        "missing_online_root"
    } else {
        // Completeness only describes roots we already know. The unchanged
        // five-minute timer also discovers announcements missed entirely.
        "periodic_reconcile"
    }
}

#[cfg(test)]
#[path = "root_recovery_tests.rs"]
mod root_recovery_tests;
