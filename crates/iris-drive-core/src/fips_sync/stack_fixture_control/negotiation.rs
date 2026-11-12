//! Source-qualified observation of the fixture's sole initial traversal target.
//! This records existing events; it does not initiate, retry or complete work.
use std::fmt::{self, Write};
use std::sync::{Mutex, OnceLock};

use anyhow::{Result, anyhow, ensure};
use serde_json::{Value, json};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Metadata};

const LIMIT: usize = 32;
static LEDGER: OnceLock<Mutex<Ledger>> = OnceLock::new();

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Start,
    Offer,
    Answer,
    Failed,
    Invalid,
}

fn classify(meta: &Metadata<'_>) -> Option<Kind> {
    let (kind, level, fields): (Kind, Level, &[&str]) =
        match (meta.target(), meta.file()?, meta.line()?) {
            ("fips_core::discovery::nostr::runtime::traversal", file, line)
                if file.ends_with("src/discovery/nostr/runtime/traversal.rs") =>
            {
                match line {
                    124 => (
                        Kind::Start,
                        Level::DEBUG,
                        &["message", "peer", "mesh_signaling_allowed"],
                    ),
                    188 => (Kind::Offer, Level::DEBUG, &["message", "peer", "session"]),
                    206 => (
                        Kind::Answer,
                        Level::DEBUG,
                        &[
                            "message",
                            "peer",
                            "session",
                            "accepted",
                            "signal_path",
                            "reflexive",
                            "local",
                        ],
                    ),
                    _ => return None,
                }
            }
            ("fips_core::node::lifecycle::discovery", file, line)
                if file.ends_with("src/node/lifecycle/discovery.rs") =>
            {
                match line {
                    448 => (
                        Kind::Failed,
                        Level::WARN,
                        &[
                            "message",
                            "npub",
                            "error",
                            "consecutive_failures",
                            "cooldown_secs",
                        ],
                    ),
                    458 => (
                        Kind::Failed,
                        Level::DEBUG,
                        &["message", "npub", "error", "consecutive_failures"],
                    ),
                    _ => return None,
                }
            }
            _ => return None,
        };
    (meta.is_event()
        && *meta.level() == level
        && meta.fields().len() == fields.len()
        && fields
            .iter()
            .all(|name| meta.fields().field(name).is_some()))
    .then_some(kind)
    .or(Some(Kind::Invalid))
}

pub(super) fn enabled(meta: &Metadata<'_>) -> bool {
    classify(meta).is_some()
}

// Only selected synthetic peer/session fields are formatted, with a strict cap.
// Unknown error text is discarded and never retained in the ledger or output.
#[derive(Default)]
struct Bounded(String);
impl Write for Bounded {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        if self.0.len().saturating_add(text.len()) > 128 {
            return Err(fmt::Error);
        }
        self.0.push_str(text);
        Ok(())
    }
}

#[derive(Default, Debug)]
struct Fields {
    peer: Option<String>,
    npub: Option<String>,
    session: Option<String>,
    mesh: Option<bool>,
    accepted: Option<bool>,
    unsupported: Option<bool>,
    signal_path: Option<bool>,
    invalid: bool,
}

impl Fields {
    fn text(&mut self, name: &str, text: &str) {
        let value = match name {
            "peer" => &mut self.peer,
            "npub" => &mut self.npub,
            "session" => &mut self.session,
            "error" => {
                self.invalid |= self
                    .unsupported
                    .replace(text == "protocol error: traversal-unsupported")
                    .is_some();
                return;
            }
            "signal_path" => {
                self.invalid |= self.signal_path.replace(text == "fips-session").is_some();
                return;
            }
            _ => return,
        };
        if text.len() > 128 {
            self.invalid = true;
            return;
        }
        self.invalid |= value.replace(text.to_owned()).is_some();
    }
}
impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.text(field.name(), value);
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        let slot = match field.name() {
            "mesh_signaling_allowed" => &mut self.mesh,
            "accepted" => &mut self.accepted,
            _ => return,
        };
        self.invalid |= slot.replace(value).is_some();
    }
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if !matches!(
            field.name(),
            "peer" | "npub" | "session" | "error" | "signal_path"
        ) {
            return;
        }
        let mut text = Bounded::default();
        if write!(&mut text, "{value:?}").is_err() {
            self.invalid = true;
            return;
        }
        self.text(field.name(), &text.0);
    }
}

#[derive(Debug)]
struct Attempt {
    session: Option<String>,
    answer: bool,
    terminal: bool,
}
#[derive(Debug)]
struct Ledger {
    expected: String,
    short: String,
    attempts: Vec<Attempt>,
    invalid: bool,
}
impl Ledger {
    fn new(expected: &str) -> Result<Self> {
        ensure!(
            expected.len() == 63 && expected.starts_with("npub1") && expected.is_ascii(),
            "invalid fixture target"
        );
        Ok(Self {
            expected: expected.into(),
            short: format!("npub1{}..{}", &expected[5..9], &expected[59..]),
            attempts: vec![],
            invalid: false,
        })
    }
    fn record(&mut self, kind: Kind, f: Fields) {
        if self.invalid {
            return;
        }
        if kind == Kind::Invalid
            || f.invalid
            || (kind == Kind::Failed && f.npub.as_deref() != Some(&self.expected))
            || (kind != Kind::Failed && f.peer.as_deref() != Some(&self.short))
        {
            self.invalid = true;
            return;
        }
        if kind == Kind::Start {
            if f.mesh != Some(true)
                || self.attempts.len() >= LIMIT
                || self.attempts.last().is_some_and(|a| !a.terminal)
            {
                self.invalid = true;
                return;
            }
            self.attempts.push(Attempt {
                session: None,
                answer: false,
                terminal: false,
            });
            return;
        }
        if kind == Kind::Offer {
            let Some(token) = f
                .session
                .as_ref()
                .filter(|s| s.len() == 8 && s.bytes().all(|b| b.is_ascii_hexdigit()))
            else {
                self.invalid = true;
                return;
            };
            // The logs shorten the random session ID. Ambiguous repeated tokens
            // fail; the core itself validates the full nonce/identity before the
            // exact terminal error below. Tokens alone are never authentication.
            if self
                .attempts
                .iter()
                .any(|a| a.session.as_ref() == Some(token))
            {
                self.invalid = true;
                return;
            }
        }
        let Some(attempt) = self.attempts.last_mut().filter(|a| !a.terminal) else {
            self.invalid = true;
            return;
        };
        match kind {
            Kind::Offer if attempt.session.is_none() && !attempt.answer => {
                attempt.session = f.session
            }
            Kind::Answer
                if attempt.session.is_some()
                    && attempt.session == f.session
                    && !attempt.answer
                    && f.accepted == Some(false)
                    && f.signal_path == Some(true) =>
            {
                attempt.answer = true
            }
            Kind::Failed if attempt.answer && f.unsupported == Some(true) => {
                attempt.terminal = true
            }
            _ => self.invalid = true,
        }
    }
    fn snapshot(&self) -> Value {
        json!({"version": 1, "expected_peer": self.expected, "invalid": self.invalid,
            "attempts": self.attempts.iter().enumerate().map(|(i, a)| json!({
                "ordinal": i + 1, "session_token": a.session, "answer_received": a.answer,
                "terminal": if a.terminal { "authenticated-unsupported" } else { "pending" }
            })).collect::<Vec<_>>()})
    }
}

pub(super) fn configure(expected: &str) -> Result<()> {
    LEDGER
        .set(Mutex::new(Ledger::new(expected)?))
        .map_err(|_| anyhow!("fixture target already configured"))
}
pub(super) fn event(event: &Event<'_>) {
    let Some(kind) = classify(event.metadata()) else {
        return;
    };
    let Some(ledger) = LEDGER.get() else {
        return;
    };
    let Ok(mut ledger) = ledger.lock() else {
        return;
    };
    let mut fields = Fields::default();
    event.record(&mut fields);
    ledger.record(kind, fields);
}
pub(super) fn snapshot() -> Result<Value> {
    Ok(LEDGER
        .get()
        .ok_or_else(|| anyhow!("fixture target unavailable"))?
        .lock()
        .map_err(|_| anyhow!("fixture negotiation observer poisoned"))?
        .snapshot())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::callsite::{Callsite, Identifier};
    use tracing::field::{FieldSet, Value as FieldValue};
    struct Site;
    static SITE: Site = Site;
    impl Callsite for Site {
        fn set_interest(&self, _: tracing::subscriber::Interest) {}
        fn metadata(&self) -> &Metadata<'_> {
            panic!("unused test metadata")
        }
    }
    fn meta(
        target: &'static str,
        file: &'static str,
        line: u32,
        level: Level,
        fields: &'static [&'static str],
    ) -> &'static Metadata<'static> {
        Box::leak(Box::new(Metadata::new(
            "synthetic",
            target,
            level,
            Some(file),
            Some(line),
            None,
            FieldSet::new(fields, Identifier(&SITE)),
            tracing::metadata::Kind::EVENT,
        )))
    }
    struct UnrelatedPrivate;
    impl fmt::Debug for UnrelatedPrivate {
        fn fmt(&self, _: &mut fmt::Formatter<'_>) -> fmt::Result {
            panic!("unrelated private field formatted")
        }
    }
    const PEER: &str = "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqq";
    fn fields(kind: Kind, token: &str) -> Fields {
        Fields {
            peer: Some(format!("npub1{}..{}", &PEER[5..9], &PEER[59..])),
            npub: Some(PEER.into()),
            session: Some(token.into()),
            mesh: Some(true),
            accepted: Some(false),
            unsupported: Some(kind == Kind::Failed),
            signal_path: Some(true),
            ..Fields::default()
        }
    }
    fn finish(l: &mut Ledger, token: &str) {
        for kind in [Kind::Start, Kind::Offer, Kind::Answer, Kind::Failed] {
            l.record(kind, fields(kind, token));
        }
    }
    #[test]
    fn exact_terminal_sequence_is_positive_without_assuming_five_attempts() {
        let mut l = Ledger::new(PEER).unwrap();
        finish(&mut l, "0123abcd");
        finish(&mut l, "89abcdef");
        assert!(!l.invalid);
        assert!(l.attempts.iter().all(|a| a.terminal));
        assert_eq!(l.snapshot()["attempts"].as_array().unwrap().len(), 2);
    }
    #[test]
    fn missing_unmatched_wrong_peer_unknown_outcome_and_ambiguous_tokens_fail() {
        for bad in 0..7 {
            let mut l = Ledger::new(PEER).unwrap();
            l.record(Kind::Start, fields(Kind::Start, "0123abcd"));
            l.record(Kind::Offer, fields(Kind::Offer, "0123abcd"));
            let mut answer = fields(Kind::Answer, "0123abcd");
            if bad == 0 {
                answer.session = Some("00000000".into());
            }
            if bad == 1 {
                answer.accepted = Some(true);
            }
            if bad == 2 {
                answer.peer = Some("other".into());
            }
            if bad != 3 {
                l.record(Kind::Answer, answer);
            }
            let mut end = fields(Kind::Failed, "0123abcd");
            if bad == 4 {
                end.unsupported = Some(false);
            }
            if bad == 5 {
                end.npub = Some("other".into());
            }
            l.record(Kind::Failed, end);
            if bad == 6 {
                finish(&mut l, "0123abcd");
            }
            assert!(l.invalid, "case {bad}");
        }
    }
    #[test]
    fn unfinished_attempt_remains_pending_and_overlap_is_invalid() {
        let mut l = Ledger::new(PEER).unwrap();
        l.record(Kind::Start, fields(Kind::Start, "0123abcd"));
        assert_eq!(l.snapshot()["attempts"][0]["terminal"], "pending");
        l.record(Kind::Start, fields(Kind::Start, "89abcdef"));
        assert!(l.invalid);
    }
    #[test]
    fn bounded_writer_rejects_overflow() {
        let mut b = Bounded::default();
        assert!(b.write_str(&"x".repeat(129)).is_err());
        assert!(b.0.is_empty());
    }

    #[test]
    fn exact_callsites_read_selected_fields_and_ignore_private_context() {
        let target = "fips_core::discovery::nostr::runtime::traversal";
        let file = "src/discovery/nostr/runtime/traversal.rs";
        let short = format!("npub1{}..{}", &PEER[5..9], &PEER[59..]);
        let peer = tracing::field::display(&short);
        let full_peer = tracing::field::display(PEER);
        let token = tracing::field::display("0123abcd");
        let reason = tracing::field::display("protocol error: traversal-unsupported");
        let private = tracing::field::debug(UnrelatedPrivate);
        let mut l = Ledger::new(PEER).unwrap();
        let start = meta(
            target,
            file,
            124,
            Level::DEBUG,
            &["message", "peer", "mesh_signaling_allowed"],
        );
        let values: [Option<&dyn FieldValue>; 3] = [Some(&private), Some(&peer), Some(&true)];
        let mut f = Fields::default();
        Event::new(start, &start.fields().value_set_all(&values)).record(&mut f);
        assert_eq!(classify(&start), Some(Kind::Start));
        l.record(Kind::Start, f);
        let offer = meta(
            target,
            file,
            188,
            Level::DEBUG,
            &["message", "peer", "session"],
        );
        let values: [Option<&dyn FieldValue>; 3] = [Some(&private), Some(&peer), Some(&token)];
        let mut f = Fields::default();
        Event::new(offer, &offer.fields().value_set_all(&values)).record(&mut f);
        assert_eq!(classify(&offer), Some(Kind::Offer));
        l.record(Kind::Offer, f);
        let answer = meta(
            target,
            file,
            206,
            Level::DEBUG,
            &[
                "message",
                "peer",
                "session",
                "accepted",
                "signal_path",
                "reflexive",
                "local",
            ],
        );
        let values: [Option<&dyn FieldValue>; 7] = [
            Some(&private),
            Some(&peer),
            Some(&token),
            Some(&false),
            Some(&"fips-session"),
            Some(&private),
            Some(&0u64),
        ];
        let mut f = Fields::default();
        Event::new(answer, &answer.fields().value_set_all(&values)).record(&mut f);
        assert_eq!(classify(&answer), Some(Kind::Answer));
        l.record(Kind::Answer, f);
        let failure = meta(
            "fips_core::node::lifecycle::discovery",
            "src/node/lifecycle/discovery.rs",
            448,
            Level::WARN,
            &[
                "message",
                "npub",
                "error",
                "consecutive_failures",
                "cooldown_secs",
            ],
        );
        let values: [Option<&dyn FieldValue>; 5] = [
            Some(&private),
            Some(&full_peer),
            Some(&reason),
            Some(&1u64),
            Some(&private),
        ];
        let mut f = Fields::default();
        Event::new(failure, &failure.fields().value_set_all(&values)).record(&mut f);
        assert_eq!(classify(&failure), Some(Kind::Failed));
        l.record(Kind::Failed, f);
        assert!(!l.invalid);
        assert!(l.attempts[0].terminal);
    }

    #[test]
    fn malformed_relevant_callsite_is_observed_as_invalid() {
        let m = meta(
            "fips_core::discovery::nostr::runtime::traversal",
            "src/discovery/nostr/runtime/traversal.rs",
            124,
            Level::INFO,
            &["message", "peer"],
        );
        assert!(enabled(&m));
        assert_eq!(classify(&m), Some(Kind::Invalid));
        let mut l = Ledger::new(PEER).unwrap();
        l.record(Kind::Invalid, Fields::default());
        assert!(l.invalid);
    }
}
