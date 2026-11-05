use super::*;
use tracing::callsite::{Callsite, Identifier};
use tracing::field::{FieldSet, Value as FieldValue};

struct TestCallsite;
static SITE: TestCallsite = TestCallsite;
impl Callsite for TestCallsite {
    fn set_interest(&self, _interest: Interest) {}
    fn metadata(&self) -> &Metadata<'_> {
        panic!("test callsite metadata must not be queried")
    }
}

fn metadata(
    target: &'static str,
    file: &'static str,
    line: u32,
    level: Level,
    fields: &'static [&'static str],
) -> &'static Metadata<'static> {
    Box::leak(Box::new(Metadata::new(
        "synthetic event",
        target,
        level,
        Some(file),
        Some(line),
        None,
        FieldSet::new(fields, Identifier(&SITE)),
        tracing::metadata::Kind::EVENT,
    )))
}

fn submission_metadata() -> &'static Metadata<'static> {
    metadata(
        "fips_core::node::handlers::session",
        "src/node/handlers/session/node_datagram_tun.rs",
        61,
        Level::TRACE,
        &["message", "dest", "msg_type", "body_bytes", "submitted"],
    )
}

fn ready_collector() -> Arc<Collector> {
    Arc::new(Collector(Mutex::new(Counters {
        initialized: true,
        service_enabled_before_start: true,
        ..Counters::default()
    })))
}

struct PrivateValue;
impl fmt::Debug for PrivateValue {
    fn fmt(&self, _formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        panic!("private value was formatted")
    }
}

fn submit(collector: &Arc<Collector>, kind: u64, bytes: Option<u64>, submitted: bool) {
    let metadata = submission_metadata();
    let private = tracing::field::debug(PrivateValue);
    let values: [Option<&dyn FieldValue>; 5] = [
        Some(&private),
        Some(&private),
        Some(&kind),
        bytes.as_ref().map(|value| value as &dyn FieldValue),
        Some(&submitted),
    ];
    let fields = metadata.fields().value_set_all(&values);
    NumericSubscriber(collector.clone()).event(&Event::new(metadata, &fields));
}

#[test]
fn only_exact_control_metadata_is_enabled_without_a_line_dependency() {
    assert_eq!(classify(submission_metadata()), Some(Kind::Submission));
    assert_eq!(
        classify(metadata(
            "fips_core::node::handlers::session",
            "src/node/handlers/session/node_datagram_tun.rs",
            999,
            Level::TRACE,
            &["message", "dest", "msg_type", "body_bytes", "submitted"]
        )),
        Some(Kind::Submission)
    );
    for (target, file) in [
        ("other", "src/node/handlers/session/node_datagram_tun.rs"),
        ("fips_core::node::handlers::session", "unrelated.rs"),
    ] {
        assert_eq!(
            classify(metadata(
                target,
                file,
                61,
                Level::TRACE,
                &["message", "dest", "msg_type", "body_bytes", "submitted"]
            )),
            None
        );
    }
}

#[test]
fn every_retry_bucket_is_source_bound_and_never_formats_fields() {
    let collector = ready_collector();
    let specs: [(
        &'static str,
        &'static str,
        u32,
        Level,
        &'static [&'static str],
    ); 11] = [
        (
            "timeout",
            "src/node/handlers/timeout.rs",
            538,
            Level::DEBUG,
            &["message", "dest", "resend"],
        ),
        (
            "timeout",
            "src/node/handlers/timeout.rs",
            505,
            Level::DEBUG,
            &["message", "dest", "error"],
        ),
        (
            "timeout",
            "src/node/handlers/timeout.rs",
            469,
            Level::DEBUG,
            &["message", "dest", "rekey"],
        ),
        (
            "timeout",
            "src/node/handlers/timeout.rs",
            421,
            Level::INFO,
            &["message", "dest"],
        ),
        (
            "rekey",
            "src/node/handlers/rekey.rs",
            794,
            Level::TRACE,
            &["message", "peer", "resend"],
        ),
        (
            "rekey",
            "src/node/handlers/rekey.rs",
            777,
            Level::DEBUG,
            &["message", "peer", "error"],
        ),
        (
            "rekey",
            "src/node/handlers/rekey.rs",
            765,
            Level::DEBUG,
            &["message", "peer"],
        ),
        (
            "rekey",
            "src/node/handlers/rekey.rs",
            855,
            Level::TRACE,
            &["message", "peer"],
        ),
        (
            "rekey",
            "src/node/handlers/rekey.rs",
            859,
            Level::DEBUG,
            &["message", "peer", "error"],
        ),
        (
            "session",
            "src/node/handlers/session/node_handshake.rs",
            521,
            Level::DEBUG,
            &["message", "src"],
        ),
        (
            "session",
            "src/node/handlers/session/node_handshake.rs",
            527,
            Level::DEBUG,
            &["message", "src", "error"],
        ),
    ];
    let private = tracing::field::debug(PrivateValue);
    for (index, (target, file, line, level, fields)) in specs.into_iter().enumerate() {
        let target = match target {
            "timeout" => "fips_core::node::handlers::timeout",
            "rekey" => "fips_core::node::handlers::rekey",
            _ => "fips_core::node::handlers::session",
        };
        let meta = metadata(target, file, line, level, fields);
        assert_eq!(classify(meta), Some(Kind::Retry(index)));
        let values = vec![Some(&private as &dyn FieldValue); fields.len()];
        let value_set = meta.fields().value_set_all(&values);
        NumericSubscriber(collector.clone()).event(&Event::new(meta, &value_set));
    }
    assert_eq!(
        collector.snapshot().unwrap()["retries"],
        serde_json::to_value([1u64; RETRIES]).unwrap()
    );
    // The otherwise identical FMP resend must not be attributed to FSP.
    assert_eq!(
        classify(metadata(
            "fips_core::node::handlers::rekey",
            "src/node/handlers/rekey.rs",
            734,
            Level::TRACE,
            &["message", "peer", "resend"]
        )),
        None
    );
}

#[test]
fn numeric_submissions_count_success_and_failure_without_private_values() {
    let collector = ready_collector();
    submit(&collector, 17, Some(46), true);
    submit(&collector, 17, Some(46), false);
    let value = collector.snapshot().unwrap();
    assert_eq!(value["submissions"][1], json!([17, 2, 1, 46]));
    assert!(
        !serde_json::to_string(&value)
            .unwrap()
            .contains("PrivateValue")
    );
}

#[test]
fn missing_or_unallocated_numeric_values_fail_closed() {
    for (kind, bytes) in [(17, None), (31, Some(2))] {
        let collector = ready_collector();
        submit(&collector, kind, bytes, true);
        assert!(collector.snapshot().is_err());
    }
    let meta = submission_metadata();
    let field = meta.fields().field("msg_type").unwrap();
    let mut visitor = NumericFields::default();
    visitor.record_u64(&field, 17);
    visitor.record_u64(&field, 18);
    assert!(visitor.invalid);
    let mut visitor = NumericFields::default();
    visitor.record_str(&field, "not a number");
    assert!(visitor.invalid);
    visitor.record_debug(&field, &PrivateValue);
}

#[test]
fn uninitialized_poisoned_or_overflowed_evidence_never_becomes_zero_success() {
    assert!(Collector::default().snapshot().is_err());
    let collector = ready_collector();
    collector.0.lock().unwrap().submissions[1][0] = u64::MAX;
    submit(&collector, 17, Some(46), true);
    assert!(collector.snapshot().is_err());
    let second = ready_collector();
    let other = second.clone();
    assert!(
        std::thread::spawn(move || {
            let _guard = other.0.lock().unwrap();
            panic!("synthetic poison");
        })
        .join()
        .is_err()
    );
    assert!(second.snapshot().is_err());
}

#[test]
fn concurrent_callbacks_remain_bounded_and_snapshots_preserve_numeric_limits() {
    let collector = ready_collector();
    let workers = (0..4)
        .map(|_| {
            let collector = collector.clone();
            std::thread::spawn(move || {
                for _ in 0..25 {
                    submit(&collector, 19, Some(2), true);
                }
            })
        })
        .collect::<Vec<_>>();
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(
        collector.snapshot().unwrap()["submissions"][3],
        json!([19, 100, 100, 200])
    );
    let mut state = collector.0.lock().unwrap();
    state.submissions = [[u64::MAX; 3]; TYPES];
    state.retries = [u64::MAX; RETRIES];
    drop(state);
    assert!(
        serde_json::to_vec(&collector.snapshot().unwrap())
            .unwrap()
            .len()
            <= SNAPSHOT_CAP
    );
}
