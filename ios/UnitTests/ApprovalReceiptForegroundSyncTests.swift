import XCTest
@testable import IrisDriveIOS

final class ApprovalReceiptForegroundSyncTests: XCTestCase {
    func testPendingApprovalReceiptBypassesOrdinaryDriveSyncMinimum() {
        XCTAssertTrue(
            irisDriveForegroundSyncIsDue(
                elapsed: 1,
                pendingApprovalReceiptCount: 1
            )
        )
        XCTAssertFalse(
            irisDriveForegroundSyncIsDue(
                elapsed: 1,
                pendingApprovalReceiptCount: 0
            )
        )
    }

    func testPendingApprovalReceiptUsesFastForegroundPoll() {
        XCTAssertLessThanOrEqual(
            irisDriveForegroundSyncIntervalNanoseconds(
                isAwaitingApproval: false,
                pendingApprovalReceiptCount: 1
            ),
            1_000_000_000
        )
        XCTAssertEqual(
            irisDriveForegroundSyncIntervalNanoseconds(
                isAwaitingApproval: false,
                pendingApprovalReceiptCount: 0
            ),
            foregroundSyncIntervalNanoseconds
        )
    }

    func testPendingApprovalReceiptUsesDedicatedAckSyncBeforeGeneralSync() {
        XCTAssertTrue(irisDriveShouldRunApprovalAckSync(pendingApprovalReceiptCount: 1))
        XCTAssertFalse(irisDriveShouldRunApprovalAckSync(pendingApprovalReceiptCount: 0))
    }

    func testReceiptTransitionAuditTracksOnlyPendingReceiptChanges() {
        XCTAssertFalse(
            irisDriveShouldAuditPendingReceiptTransition(from: nil, to: 0)
        )
        XCTAssertTrue(
            irisDriveShouldAuditPendingReceiptTransition(from: nil, to: 1)
        )
        XCTAssertTrue(
            irisDriveShouldAuditPendingReceiptTransition(from: 1, to: 0)
        )
        XCTAssertFalse(
            irisDriveShouldAuditPendingReceiptTransition(from: 0, to: 0)
        )
    }
}
