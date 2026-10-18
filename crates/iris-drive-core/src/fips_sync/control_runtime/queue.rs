use std::collections::VecDeque;

use super::SendMarker;

#[derive(Default)]
pub(super) struct PeerQueue {
    pub(super) records: VecDeque<QueuedRecord>,
    pub(super) bytes: usize,
    pub(super) next_attempt_ms: u64,
    pub(super) restart_stream: bool,
}

pub(super) struct QueuedRecord {
    pub(super) bytes: Vec<u8>,
    pub(super) offset: usize,
    pub(super) ack_marker: Option<SendMarker>,
    pub(super) expires_at_ms: u64,
    pub(super) bootstrap: bool,
}

impl PeerQueue {
    pub(super) fn push(&mut self, record: QueuedRecord) {
        self.bytes = self.bytes.saturating_add(record.bytes.len());
        if record.bootstrap {
            let index = self
                .records
                .iter()
                .position(|queued| !queued.bootstrap)
                .unwrap_or(self.records.len());
            self.restart_stream |=
                index == 0 && self.records.front().is_some_and(|queued| queued.offset > 0);
            self.records.insert(index, record);
        } else {
            self.records.push_back(record);
        }
    }

    pub(super) fn rewind_after_stream_change(&mut self) {
        self.restart_stream = false;
        for record in &mut self.records {
            record.offset = 0;
            record.ack_marker = None;
        }
    }

    pub(super) fn pop_front(&mut self) {
        if let Some(record) = self.records.pop_front() {
            self.bytes = self.bytes.saturating_sub(record.bytes.len());
        }
    }

    pub(super) fn expire_unstarted(&mut self, now_ms: u64) {
        while self
            .records
            .front()
            .is_some_and(|record| record.offset == 0 && now_ms >= record.expires_at_ms)
        {
            self.pop_front();
        }
    }
}
