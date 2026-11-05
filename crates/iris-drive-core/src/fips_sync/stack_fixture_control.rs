//! Numeric, process-wide diagnostics for synthetic stack-fixture binaries only.
//! No writer, formatter, payload, identity, filter or error string is retained.
//! Metadata selectors below are qualified against nvpn-fips-core 0.4.119.
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::level_filters::LevelFilter;
use tracing::span::{Attributes, Id, Record};
use tracing::subscriber::Interest;
use tracing::{Event, Level, Metadata, Subscriber};

const TYPES: usize = 8;
const RETRIES: usize = 11;
const SNAPSHOT_CAP: usize = 2048;
static COLLECTOR: OnceLock<Arc<Collector>> = OnceLock::new();

// Retry order is part of schema 1, shared with Stack's exact projection:
// handshake sent/failed/exhausted/timeout, msg3 sent/failed/exhausted,
// pending probe sent/failed, duplicate-ACK msg3 sent/failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Kind {
    Submission,
    Retry(usize),
}

fn shape(metadata: &Metadata<'_>, names: &[&str]) -> bool {
    metadata.fields().len() == names.len()
        && names
            .iter()
            .all(|name| metadata.fields().field(name).is_some())
}

fn source(metadata: &Metadata<'_>, target: &str, file: &str) -> bool {
    metadata.is_event()
        && metadata.target() == target
        && metadata.file().is_some_and(|actual| actual.ends_with(file))
}

fn classify(metadata: &Metadata<'_>) -> Option<Kind> {
    if source(
        metadata,
        "fips_core::node::handlers::session",
        "src/node/handlers/session/node_datagram_tun.rs",
    ) && *metadata.level() == Level::TRACE
        && shape(
            metadata,
            &["message", "dest", "msg_type", "body_bytes", "submitted"],
        )
    {
        return Some(Kind::Submission);
    }
    if source(
        metadata,
        "fips_core::node::handlers::timeout",
        "src/node/handlers/timeout.rs",
    ) {
        // These four level/field shapes are unique in the pinned source file.
        let index = if *metadata.level() == Level::INFO && shape(metadata, &["message", "dest"]) {
            3
        } else if *metadata.level() == Level::DEBUG
            && shape(metadata, &["message", "dest", "rekey"])
        {
            2
        } else if *metadata.level() == Level::DEBUG
            && shape(metadata, &["message", "dest", "error"])
        {
            1
        } else if *metadata.level() == Level::DEBUG
            && shape(metadata, &["message", "dest", "resend"])
        {
            0
        } else {
            return None;
        };
        return Some(Kind::Retry(index));
    }
    if source(
        metadata,
        "fips_core::node::handlers::rekey",
        "src/node/handlers/rekey.rs",
    ) {
        // FMP/FSP events share field shapes. Line discriminators avoid ever
        // formatting their message/error values. Changing the pinned core
        // requires reviewing these selectors and the source-contract receipt.
        let (index, level, fields): (usize, Level, &[&str]) = match metadata.line()? {
            794 => (4, Level::TRACE, &["message", "peer", "resend"]),
            777 => (5, Level::DEBUG, &["message", "peer", "error"]),
            765 => (6, Level::DEBUG, &["message", "peer"]),
            855 => (7, Level::TRACE, &["message", "peer"]),
            859 => (8, Level::DEBUG, &["message", "peer", "error"]),
            _ => return None,
        };
        return (*metadata.level() == level && shape(metadata, fields))
            .then_some(Kind::Retry(index));
    }
    if source(
        metadata,
        "fips_core::node::handlers::session",
        "src/node/handlers/session/node_handshake.rs",
    ) && *metadata.level() == Level::DEBUG
    {
        if metadata.line() == Some(521) && shape(metadata, &["message", "src"]) {
            return Some(Kind::Retry(9));
        }
        // This shape is unique in the pinned handshake source.
        if shape(metadata, &["message", "src", "error"]) {
            return Some(Kind::Retry(10));
        }
    }
    None
}

#[derive(Default)]
struct NumericFields {
    msg_type: u64,
    body_bytes: u64,
    submitted: bool,
    seen: u8,
    invalid: bool,
}

impl NumericFields {
    fn mark(&mut self, bit: u8) {
        self.invalid |= self.seen & bit != 0;
        self.seen |= bit;
    }

    fn reject_numeric(&mut self, field: &Field) {
        self.invalid |= matches!(field.name(), "msg_type" | "body_bytes" | "submitted");
    }
}

impl Visit for NumericFields {
    fn record_u64(&mut self, field: &Field, value: u64) {
        match field.name() {
            "msg_type" => {
                self.mark(1);
                self.msg_type = value;
            }
            "body_bytes" => {
                self.mark(2);
                self.body_bytes = value;
            }
            "submitted" => self.invalid = true,
            _ => {}
        }
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        if field.name() == "submitted" {
            self.mark(4);
            self.submitted = value;
        } else {
            self.reject_numeric(field);
        }
    }

    fn record_debug(&mut self, field: &Field, _value: &dyn fmt::Debug) {
        // In particular, never format message, dest, src, peer or error.
        self.reject_numeric(field);
    }

    fn record_str(&mut self, field: &Field, _value: &str) {
        self.reject_numeric(field);
    }
}

#[derive(Default)]
struct Counters {
    initialized: bool,
    service_enabled_before_start: bool,
    invalid_events: u64,
    overflow: bool,
    // Fixed rows for msg_type 16..23: attempts, submitted, submitted body bytes.
    submissions: [[u64; 3]; TYPES],
    retries: [u64; RETRIES],
}

#[derive(Default)]
struct Collector(Mutex<Counters>);

fn add(value: &mut u64, amount: u64) -> bool {
    if let Some(next) = value.checked_add(amount) {
        *value = next;
        true
    } else {
        *value = u64::MAX;
        false
    }
}

impl Collector {
    fn record(&self, kind: Kind, event: &Event<'_>) {
        let mut fields = NumericFields::default();
        if kind == Kind::Submission {
            event.record(&mut fields);
        }
        // Poison remains observable by snapshot; a callback never panics or logs.
        let Ok(mut state) = self.0.lock() else { return };
        match kind {
            Kind::Submission
                if fields.invalid || fields.seen != 7 || !(16..24).contains(&fields.msg_type) =>
            {
                let valid = add(&mut state.invalid_events, 1);
                state.overflow |= !valid;
            }
            Kind::Submission => {
                let Ok(index) = usize::try_from(fields.msg_type - 16) else {
                    let valid = add(&mut state.invalid_events, 1);
                    state.overflow |= !valid;
                    return;
                };
                let row = &mut state.submissions[index];
                let mut valid = add(&mut row[0], 1);
                if fields.submitted {
                    valid &= add(&mut row[1], 1);
                    valid &= add(&mut row[2], fields.body_bytes);
                }
                state.overflow |= !valid;
            }
            Kind::Retry(index) => {
                let valid = add(&mut state.retries[index], 1);
                state.overflow |= !valid;
            }
        }
    }

    fn snapshot(&self) -> Result<Value> {
        let state = self
            .0
            .lock()
            .map_err(|_| anyhow!("fixture control counters poisoned"))?;
        ensure!(
            state.initialized && state.service_enabled_before_start,
            "fixture diagnostics were not initialized before service startup"
        );
        ensure!(
            state.invalid_events == 0 && !state.overflow,
            "fixture control evidence is incomplete"
        );
        let rows = state
            .submissions
            .iter()
            .enumerate()
            .map(|(index, row)| [index as u64 + 16, row[0], row[1], row[2]])
            .collect::<Vec<_>>();
        let result = json!({"version": 1, "initialized": 1, "service_enabled_before_start": 1,
            "invalid_events": state.invalid_events, "overflow": u8::from(state.overflow),
            "submissions": rows, "retries": state.retries});
        ensure!(
            serde_json::to_vec(&result)?.len() <= SNAPSHOT_CAP,
            "fixture control snapshot exceeds cap"
        );
        Ok(result)
    }
}

struct NumericSubscriber(Arc<Collector>);

impl Subscriber for NumericSubscriber {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        classify(metadata).is_some()
    }
    fn register_callsite(&self, metadata: &'static Metadata<'static>) -> Interest {
        if self.enabled(metadata) {
            Interest::always()
        } else {
            Interest::never()
        }
    }
    fn max_level_hint(&self) -> Option<LevelFilter> {
        Some(LevelFilter::TRACE)
    }
    fn new_span(&self, _attributes: &Attributes<'_>) -> Id {
        Id::from_u64(1)
    }
    fn record(&self, _span: &Id, _values: &Record<'_>) {}
    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}
    fn event(&self, event: &Event<'_>) {
        if let Some(kind) = classify(event.metadata()) {
            self.0.record(kind, event);
        }
    }
    fn enter(&self, _span: &Id) {}
    fn exit(&self, _span: &Id) {}
}

/// Called only by the synthetic fixture binary, before any app/runtime startup.
pub fn install() -> Result<()> {
    ensure!(
        tracing::level_filters::STATIC_MAX_LEVEL >= LevelFilter::TRACE,
        "fixture control callsites are disabled by compile-time filtering"
    );
    ensure!(
        COLLECTOR.get().is_none(),
        "fixture diagnostics already installed"
    );
    let collector = Arc::new(Collector::default());
    tracing::subscriber::set_global_default(NumericSubscriber(collector.clone()))
        .map_err(|_| anyhow!("fixture diagnostics require exclusive subscriber ownership"))?;
    collector
        .0
        .lock()
        .map_err(|_| anyhow!("fixture control counters poisoned"))?
        .initialized = true;
    COLLECTOR
        .set(collector)
        .map_err(|_| anyhow!("fixture diagnostics already installed"))?;
    Ok(())
}

pub(crate) fn is_installed() -> bool {
    COLLECTOR.get().is_some()
}

/// Invoked immediately after the primary endpoint's diagnostic services enable.
pub(crate) fn mark_services_enabled() -> Result<()> {
    COLLECTOR
        .get()
        .ok_or_else(|| anyhow!("fixture control collector unavailable"))?
        .0
        .lock()
        .map_err(|_| anyhow!("fixture control counters poisoned"))?
        .service_enabled_before_start = true;
    Ok(())
}

pub(crate) fn snapshot() -> Result<Value> {
    COLLECTOR
        .get()
        .ok_or_else(|| anyhow!("fixture control collector unavailable"))?
        .snapshot()
}

#[cfg(test)]
#[path = "stack_fixture_control/tests.rs"]
mod tests;
