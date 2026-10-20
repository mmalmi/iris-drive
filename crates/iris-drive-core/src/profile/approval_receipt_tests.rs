use super::*;

#[test]
fn pending_link_request_migrates_one_receipt_to_the_bounded_collection() {
    let request: PendingAppKeyLinkRequest = serde_json::from_value(serde_json::json!({
        "admin_app_key_pubkey": "admin",
        "approval_receipt_event": "legacy-event",
        "requested_at": 7,
    }))
    .unwrap();
    assert_eq!(
        request.approval_receipt_event.iter().collect::<Vec<_>>(),
        vec!["legacy-event"]
    );

    let encoded = serde_json::to_value(request).unwrap();
    assert_eq!(
        encoded["approval_receipt_event"],
        serde_json::json!(["legacy-event"])
    );
}

#[test]
fn persisted_device_approval_receipts_are_deduplicated_and_bounded() {
    let mut receipts = PersistedDeviceApprovalReceipts::default();
    for index in 0..=MAX_PERSISTED_DEVICE_APPROVAL_RECEIPTS {
        assert!(receipts.insert(format!("event-{index}")));
    }
    assert_eq!(receipts.len(), MAX_PERSISTED_DEVICE_APPROVAL_RECEIPTS);
    assert_eq!(receipts.iter().next(), Some("event-1"));
    assert!(!receipts.insert("event-1".to_string()));
}
