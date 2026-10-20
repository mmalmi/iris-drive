use super::{DecodedRecord, MAX_QUEUED_BYTES_PER_PEER, MAX_QUEUED_RECORDS_PER_PEER};

/// Bound the whole unauthenticated exchange, including repeated record IDs.
/// The connection's absolute lifetime is enforced separately by the actor.
#[derive(Default)]
pub(super) struct BootstrapBudget {
    records: usize,
    bytes: usize,
}

impl BootstrapBudget {
    pub(super) fn admit(&mut self, record: &DecodedRecord) -> bool {
        self.records = self.records.saturating_add(1);
        // Length prefix, version, record ID, topic length and payload length.
        self.bytes = self
            .bytes
            .saturating_add(27 + record.topic.len() + record.data.len());
        self.records <= MAX_QUEUED_RECORDS_PER_PEER && self.bytes <= MAX_QUEUED_BYTES_PER_PEER
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BootstrapBudget, DecodedRecord, MAX_QUEUED_BYTES_PER_PEER, MAX_QUEUED_RECORDS_PER_PEER,
    };

    fn record(bytes: usize) -> DecodedRecord {
        DecodedRecord {
            id: [0; 16],
            topic: "bootstrap".into(),
            data: vec![0; bytes],
        }
    }

    #[test]
    fn duplicate_bootstrap_records_cannot_extend_the_record_budget() {
        let mut budget = BootstrapBudget::default();
        let record = record(1);
        for _ in 0..MAX_QUEUED_RECORDS_PER_PEER {
            assert!(budget.admit(&record));
        }
        assert!(!budget.admit(&record));
    }

    #[test]
    fn bootstrap_byte_budget_includes_framing_and_stays_exhausted() {
        let mut budget = BootstrapBudget::default();
        let first = record(MAX_QUEUED_BYTES_PER_PEER - 27 - "bootstrap".len());
        assert!(budget.admit(&first));
        assert!(!budget.admit(&record(0)));
    }
}
