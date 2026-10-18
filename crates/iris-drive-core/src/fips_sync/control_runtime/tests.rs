use super::*;

#[test]
fn control_record_roundtrip_and_limit() {
    let record = encode_record("iris-drive/test", &[1, 2, 3]).unwrap();
    let mut input = record;
    let decoded = decode_record(&mut input).unwrap().unwrap();
    assert_eq!(decoded.topic, "iris-drive/test");
    assert_eq!(decoded.data, vec![1, 2, 3]);
    assert_eq!(input.len(), 0);
    let oversized = vec![0; DRIVE_CONTROL_MAX_PAYLOAD_BYTES + 1];
    assert!(encode_record("test", &oversized).is_err());
}

#[test]
fn partial_record_waits_without_consuming() {
    let record = encode_record("test", &[1, 2, 3]).unwrap();
    let mut input = record[..record.len() - 1].to_vec();
    let before = input.clone();
    assert_eq!(decode_record(&mut input).unwrap(), None);
    assert_eq!(input, before);
}

#[test]
fn retry_coalescing_ignores_only_the_random_record_id() {
    let first = encode_record("test", b"same payload").unwrap();
    let retry = encode_record("test", b"same payload").unwrap();
    let other_topic = encode_record("other", b"same payload").unwrap();
    let other_payload = encode_record("test", b"other payload").unwrap();

    assert_ne!(first, retry);
    assert!(same_logical_record(&first, &retry));
    assert!(!same_logical_record(&first, &other_topic));
    assert!(!same_logical_record(&first, &other_payload));
}

#[test]
fn reconnect_rewinds_the_whole_queue_without_reordering() {
    let interrupted = encode_record("test", b"interrupted").unwrap();
    let waiting = encode_record("test", b"waiting").unwrap();
    let mut queue = PeerQueue {
        bytes: interrupted.len() + waiting.len(),
        records: VecDeque::from([
            QueuedRecord {
                bytes: interrupted.clone(),
                offset: 5,
                ack_marker: None,
                expires_at_ms: 100,
                bootstrap: false,
            },
            QueuedRecord {
                bytes: waiting.clone(),
                offset: 0,
                ack_marker: None,
                expires_at_ms: 100,
                bootstrap: false,
            },
        ]),
        next_attempt_ms: 0,
        restart_stream: false,
    };

    queue.rewind_after_stream_change();

    assert_eq!(queue.records.front().unwrap().offset, 0);
    assert_eq!(queue.records.front().unwrap().bytes, interrupted);
    assert_eq!(queue.records.back().unwrap().offset, 0);
    assert_eq!(queue.records.back().unwrap().bytes, waiting);
    assert_eq!(
        queue.bytes,
        queue
            .records
            .iter()
            .map(|record| record.bytes.len())
            .sum::<usize>()
    );
}

#[test]
fn expiry_never_discards_a_partial_record_tail() {
    let bytes = encode_record("test", b"payload").unwrap();
    let mut queue = PeerQueue {
        bytes: bytes.len(),
        records: VecDeque::from([QueuedRecord {
            bytes,
            offset: 5,
            ack_marker: None,
            expires_at_ms: 100,
            bootstrap: false,
        }]),
        next_attempt_ms: 0,
        restart_stream: false,
    };

    queue.expire_unstarted(101);
    assert_eq!(queue.records.len(), 1);

    queue.rewind_after_stream_change();
    queue.expire_unstarted(101);
    assert!(queue.records.is_empty());
    assert_eq!(queue.bytes, 0);
}

#[test]
fn bootstrap_overtakes_partial_roots_without_reordering_or_losing_records() {
    let record = |name: &str, bootstrap, offset| QueuedRecord {
        bytes: encode_record(name, b"payload").unwrap(),
        offset,
        ack_marker: None,
        expires_at_ms: 100,
        bootstrap,
    };
    let mut queue = PeerQueue::default();
    queue.push(record("root-one", false, 5));
    queue.push(record("root-two", false, 0));
    queue.push(record("receipt", true, 0));
    queue.push(record("roster", true, 0));
    assert!(queue.restart_stream);
    queue.rewind_after_stream_change();
    assert!(!queue.restart_stream);
    assert!(queue.records.iter().all(|record| record.offset == 0));
    let topics = queue
        .records
        .iter()
        .map(|record| {
            decode_record(&mut record.bytes.clone())
                .unwrap()
                .unwrap()
                .topic
        })
        .collect::<Vec<_>>();
    assert_eq!(topics, ["receipt", "roster", "root-one", "root-two"]);
    assert_eq!(
        queue.bytes,
        queue
            .records
            .iter()
            .map(|record| record.bytes.len())
            .sum::<usize>()
    );
}
