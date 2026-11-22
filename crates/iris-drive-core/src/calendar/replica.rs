use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::CalendarError;

pub const TREE_PREFIX: &str = "calendar-replica-";
pub const REPLICA_LABEL: &str = "calendar-replica";
pub(super) const CHANGES_FILE: &str = "changes.json";
pub(super) const MAX_CLOCK: u64 = 9_007_199_254_740_991;

pub(super) type FieldPath = Vec<String>;
pub(super) type Fields = BTreeMap<FieldPath, Value>;

/// A writer retains its most recent operation on each path, including removals.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Replica {
    version: u8,
    pub(super) writer: String,
    changes: Vec<(FieldPath, u64, Value)>,
}

impl Replica {
    pub(super) fn new(writer: String) -> Self {
        Self {
            version: 1,
            writer,
            changes: Vec::new(),
        }
    }

    pub(super) fn decode(bytes: &[u8], writer: &str) -> Result<Self, CalendarError> {
        let replica: Self = serde_json::from_slice(bytes)?;
        if replica.version != 1 || replica.writer != writer || !valid_writer(writer) {
            return Err(invalid("invalid calendar replica identity or version"));
        }
        let mut paths = BTreeSet::new();
        for (path, clock, value) in &replica.changes {
            if *clock == 0
                || *clock > MAX_CLOCK
                || !valid_register(path, value)
                || !paths.insert(path)
            {
                return Err(invalid("invalid or duplicate calendar replica register"));
            }
        }
        Ok(replica)
    }

    pub(super) fn write(&mut self, clock: u64, edits: Fields) {
        self.changes
            .retain(|(path, _, _)| !edits.contains_key(path));
        self.changes
            .extend(edits.into_iter().map(|(path, value)| (path, clock, value)));
        self.changes.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    }
}

#[derive(Default)]
pub(super) struct Registers {
    pub(super) clock: u64,
    winners: BTreeMap<FieldPath, (u64, String, Value)>,
}

impl Registers {
    pub(super) fn merge(&mut self, replica: &Replica) {
        for (path, clock, value) in &replica.changes {
            self.clock = self.clock.max(*clock);
            let ordering = (*clock, replica.writer.as_str());
            if self
                .winners
                .get(path)
                .is_none_or(|(clock, writer, _)| ordering > (*clock, writer.as_str()))
            {
                self.winners.insert(
                    path.clone(),
                    (*clock, replica.writer.clone(), value.clone()),
                );
            }
        }
    }

    pub(super) fn overlay(&self, mut base: Fields) -> Fields {
        base.extend(
            self.winners
                .iter()
                .map(|(path, (_, _, value))| (path.clone(), value.clone())),
        );
        base
    }
}

pub(super) fn invalid(message: impl Into<String>) -> CalendarError {
    CalendarError::Data(message.into())
}

pub(super) fn valid_writer(writer: &str) -> bool {
    writer.len() == 32
        && writer
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub fn replica_writer(tree: &str) -> Option<&str> {
    tree.strip_prefix(TREE_PREFIX)
        .filter(|writer| valid_writer(writer))
}

fn valid_register(path: &[String], value: &Value) -> bool {
    if path.iter().any(|key| key.is_empty() || key.len() > 4096) {
        return false;
    }
    let path: Vec<_> = path.iter().map(String::as_str).collect();
    let presence = value.is_null() || value.as_bool() == Some(true);
    match path.as_slice() {
        ["availability", "rules", _] | ["events" | "bookingLinks" | "subscriptions", _] => presence,
        ["availability", "rules", _, field] => *field != "id",
        ["availability", field] => *field != "rules",
        ["events", _, "guests", _] => value.is_object() || value.is_null(),
        ["events", _, "schedule"] => value.is_object() || value.is_null(),
        ["events", _, field] => !matches!(
            *field,
            "id" | "calendar" | "guests" | "allDay" | "start" | "end" | "day" | "days"
        ),
        ["subscriptions", _, field] => *field != "npub",
        ["bookingLinks", _, field] => *field != "id",
        ["shareWithdrawals", _] => {
            value.is_null() || value.as_u64().is_some_and(|n| n <= MAX_CLOCK)
        }
        ["sharing"] => value.is_object() || value.is_null(),
        [field] => !matches!(
            *field,
            "events"
                | "subscriptions"
                | "bookingLinks"
                | "availability"
                | "shareWithdrawals"
                | "shareKey"
                | "sharedWith"
                | "version"
                | "ownerNpub"
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const A: &str = "00000000000000000000000000000001";
    const B: &str = "00000000000000000000000000000002";

    #[test]
    fn concurrent_register_merge_is_independent_of_arrival_order() {
        let a = Replica::decode(
            &serde_json::to_vec(&json!({"version":1,"writer":A,"changes":[
                [["events","one","title"],5,"A"], [["events","one","location"],5,"Brooklyn"]
            ]}))
            .unwrap(),
            A,
        )
        .unwrap();
        let b = Replica::decode(
            &serde_json::to_vec(&json!({"version":1,"writer":B,"changes":[
                [["events","one","title"],5,"B"], [["events","one","notes"],5,"note"],
                [["events","deleted"],6,null]
            ]}))
            .unwrap(),
            B,
        )
        .unwrap();
        let expected = Fields::from([
            (
                vec!["events".into(), "one".into(), "title".into()],
                json!("B"),
            ),
            (
                vec!["events".into(), "one".into(), "location".into()],
                json!("Brooklyn"),
            ),
            (
                vec!["events".into(), "one".into(), "notes".into()],
                json!("note"),
            ),
            (vec!["events".into(), "deleted".into()], Value::Null),
        ]);
        for replicas in [[&a, &b, &a], [&b, &a, &b]] {
            let mut merged = Registers::default();
            for replica in replicas {
                merged.merge(replica);
            }
            assert_eq!(merged.clock, 6);
            assert_eq!(merged.overlay(Fields::new()), expected);
        }
    }

    #[test]
    fn rejects_wrong_writer_duplicate_register_and_unsafe_clock() {
        for changes in [
            json!([[["title"], 1, "one"], [["title"], 2, "two"]]),
            json!([[["title"], MAX_CLOCK + 1, "one"]]),
            json!([[["events", "id"], 1, false]]),
        ] {
            let encoded =
                serde_json::to_vec(&json!({"version":1,"writer":A,"changes":changes})).unwrap();
            assert!(Replica::decode(&encoded, A).is_err());
        }
        let encoded = serde_json::to_vec(&Replica::new(A.into())).unwrap();
        assert!(Replica::decode(&encoded, B).is_err());
    }
}
